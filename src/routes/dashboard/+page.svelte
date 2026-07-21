<script>
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { settings, formatSpeed, applyAccentTheme } from "../../lib/settingsStore";
  import { groupProcesses, getDisplayName } from "../../lib/processGroups";

  $effect(() => {
    applyAccentTheme($settings.accentColor, $settings.theme);
  });

  let period = $state("live"); // 'live', 'hourly', 'daily', 'weekly', 'monthly', 'yearly'
  let stats = $state([]);
  let loading = $state(true);
  let errorMsg = $state("");
  let groupApps = $state(true);

  // Live tracking states
  let liveDownloadSpeed = $state(0);
  let liveUploadSpeed = $state(0);
  let liveActiveApp = $state("System");
  let liveHistory = $state({ download: Array(60).fill(0), upload: Array(60).fill(0) });
  let liveProcessMap = $state({});
  let sortField = $state("total"); // 'name', 'live_down', 'live_up', 'download', 'upload', 'time', 'total', 'share'
  let sortAscending = $state(false);

  let unlistenStats;
  let unlistenSpeedTest;

  // Speed Test & Speedometer state
  let showSpeedTestModal = $state(false);
  let speedTestRunning = $state(false);
  let speedTestStage = $state("idle"); // 'idle', 'ping', 'download', 'upload', 'complete', 'error'
  let speedTestMsg = $state("Click 'Run Speed Test' to benchmark your connection.");
  let speedTestProgressPct = $state(0);
  let currentTestSpeedBps = $state(0);
  let peakDownloadBps = $state(0);
  let peakUploadBps = $state(0);
  let dnsPings = $state([
    { name: "Cloudflare", ip: "1.1.1.1", latency_ms: 0 },
    { name: "Google DNS", ip: "8.8.8.8", latency_ms: 0 },
    { name: "Quad9 DNS", ip: "9.9.9.9", latency_ms: 0 },
    { name: "OpenDNS", ip: "208.67.222.222", latency_ms: 0 }
  ]);
  let avgPing = $state(0);

  // Derived speedometer angle and gauge ratio calculations
  let speedMbps = $derived((currentTestSpeedBps * 8) / 1000000);
  let maxScaleMbps = $derived(speedMbps > 500 ? 1000 : (speedMbps > 100 ? 500 : 100));
  let speedRatio = $derived(Math.min(speedMbps / maxScaleMbps, 1.0));
  let needleAngle = $derived(-120 + (speedRatio * 240));
  let gaugeFillPct = $derived(speedRatio);

  let displayedSpeedVal = $derived(
    currentTestSpeedBps > 0 
      ? formatSpeed(currentTestSpeedBps, $settings.unit)
      : (speedTestStage === 'complete' ? formatSpeed(peakDownloadBps, $settings.unit) : '0.0 Mbps')
  );

  let qualityBadge = $derived.by(() => {
    if (avgPing === 0) return { label: "Ready to Test", icon: "⚡", class: "neutral" };
    if (avgPing < 25) return { label: "Gaming & 4K Ultra Ready", icon: "🎮", class: "excellent" };
    if (avgPing < 60) return { label: "Streaming & Video Calls Ready", icon: "📺", class: "good" };
    if (avgPing < 120) return { label: "Web Browsing Standard", icon: "🌐", class: "fair" };
    return { label: "High Latency Connection", icon: "⚠️", class: "poor" };
  });

  async function startSpeedTest() {
    if (speedTestRunning) return;
    speedTestRunning = true;
    speedTestStage = "ping";
    speedTestMsg = "Testing DNS latency...";
    speedTestProgressPct = 5;
    currentTestSpeedBps = 0;
    peakDownloadBps = 0;
    peakUploadBps = 0;

    try {
      const res = await invoke("run_speed_test");
      if (res) {
        speedTestStage = "complete";
        speedTestProgressPct = 100;
        peakDownloadBps = res.download_speed;
        peakUploadBps = res.upload_speed;
        if (res.pings && res.pings.length > 0) dnsPings = res.pings;
        avgPing = res.average_ping;
        speedTestMsg = "Speed Test Completed Successfully!";
      }
    } catch (e) {
      console.error("Speed test error:", e);
      speedTestMsg = String(e || "Speed test failed to run");
      speedTestStage = "error";
    } finally {
      speedTestRunning = false;
    }
  }

  // Track live process session usage list
  let liveProcessList = $derived(
    (() => {
      const list = Object.entries(liveProcessMap).map(([name, val]) => ({
        process_name: name,
        bytes_downloaded: val.download,
        bytes_uploaded: val.upload,
        current_download_speed: val.current_down || 0,
        current_upload_speed: val.current_up || 0,
        screen_time_seconds: val.time
      }));

      if (!groupApps) {
        return list.sort((a, b) => (b.bytes_downloaded + b.bytes_uploaded) - (a.bytes_downloaded + a.bytes_uploaded));
      }

      const grouped = {};
      list.forEach(item => {
        const displayName = getDisplayName(item.process_name);
        if (!grouped[displayName]) {
          grouped[displayName] = {
            process_name: displayName,
            bytes_downloaded: 0,
            bytes_uploaded: 0,
            current_download_speed: 0,
            current_upload_speed: 0,
            screen_time_seconds: 0
          };
        }
        grouped[displayName].bytes_downloaded += item.bytes_downloaded;
        grouped[displayName].bytes_uploaded += item.bytes_uploaded;
        grouped[displayName].current_download_speed += item.current_download_speed;
        grouped[displayName].current_upload_speed += item.current_upload_speed;
        grouped[displayName].screen_time_seconds += item.screen_time_seconds;
      });

      return Object.values(grouped).sort((a, b) => (b.bytes_downloaded + b.bytes_uploaded) - (a.bytes_downloaded + a.bytes_uploaded));
    })()
  );

  // Dynamic SVG path calculations for the live chart (width: 800, height: 120)
  let smoothLiveMax = $state(1024);

  $effect(() => {
    const rawMax = Math.max(...liveHistory.download, ...liveHistory.upload, 1024);
    if (rawMax >= smoothLiveMax) {
      smoothLiveMax = rawMax;
    } else {
      smoothLiveMax = Math.max(rawMax, smoothLiveMax * 0.95);
    }
  });

  let scaleLiveMax = $derived(smoothLiveMax * 1.20);

  let avgLiveDown = $derived.by(() => {
    if (!liveHistory.download || liveHistory.download.length === 0) return 0;
    const active = liveHistory.download.filter(v => v > 0);
    if (active.length === 0) return 0;
    const sum = active.reduce((acc, v) => acc + v, 0);
    return sum / active.length;
  });

  let avgLiveUp = $derived.by(() => {
    if (!liveHistory.upload || liveHistory.upload.length === 0) return 0;
    const active = liveHistory.upload.filter(v => v > 0);
    if (active.length === 0) return 0;
    const sum = active.reduce((acc, v) => acc + v, 0);
    return sum / active.length;
  });

  let peakLiveDown = $derived.by(() => {
    if (!liveHistory.download || liveHistory.download.length === 0) return 0;
    return Math.max(...liveHistory.download);
  });

  let peakLiveUp = $derived.by(() => {
    if (!liveHistory.upload || liveHistory.upload.length === 0) return 0;
    return Math.max(...liveHistory.upload);
  });

  let liveDownPath = $derived(getSvgPath(liveHistory.download, 120, scaleLiveMax));
  let liveDownAreaPath = $derived(getSvgAreaPath(liveHistory.download, 120, scaleLiveMax));
  let liveUpPath = $derived(getSvgPath(liveHistory.upload, 120, scaleLiveMax));
  let liveUpAreaPath = $derived(getSvgAreaPath(liveHistory.upload, 120, scaleLiveMax));

  function getSvgPath(history, height, maxVal) {
    if (maxVal === 0) maxVal = 1;
    const points = history.map((val, idx) => {
      const x = (idx / (history.length - 1)) * 800;
      const y = height - (val / maxVal) * height;
      return `${x},${y}`;
    });
    return `M ${points.join(" L ")}`;
  }

  function getSvgAreaPath(history, height, maxVal) {
    if (maxVal === 0) maxVal = 1;
    const points = history.map((val, idx) => {
      const x = (idx / (history.length - 1)) * 800;
      const y = height - (val / maxVal) * height;
      return `${x},${y}`;
    });
    return `M 0,${height} L ${points.join(" L ")} L 800,${height} Z`;
  }

  let groupedStats = $derived(groupApps ? groupProcesses(stats) : stats);

  let grandTotalBytes = $derived(
    period === 'live'
      ? liveProcessList.reduce((acc, i) => acc + (i.bytes_downloaded || 0) + (i.bytes_uploaded || 0), 0)
      : stats.reduce((acc, i) => acc + (i.bytes_downloaded || 0) + (i.bytes_uploaded || 0), 0)
  );

  // Calculate totals safely for database metrics
  let totalDownload = $derived(
    Array.isArray(stats) ? stats.reduce((acc, s) => acc + (s.bytes_downloaded || 0), 0) : 0
  );
  let totalUpload = $derived(
    Array.isArray(stats) ? stats.reduce((acc, s) => acc + (s.bytes_uploaded || 0), 0) : 0
  );
  let totalScreenTime = $derived(
    Array.isArray(stats) ? stats.reduce((acc, s) => acc + (s.screen_time_seconds || 0), 0) : 0
  );

  let sortedList = $derived(
    (() => {
      const activeList = period === 'live' ? liveProcessList : groupedStats;
      let list = [...activeList];
      
      list.sort((a, b) => {
        let valA, valB;
        switch (sortField) {
          case 'name':
            valA = (a.process_name || 'System').toLowerCase();
            valB = (b.process_name || 'System').toLowerCase();
            return sortAscending ? valA.localeCompare(valB) : valB.localeCompare(valA);
          case 'live_down':
            valA = a.current_download_speed || 0;
            valB = b.current_download_speed || 0;
            break;
          case 'live_up':
            valA = a.current_upload_speed || 0;
            valB = b.current_upload_speed || 0;
            break;
          case 'download':
            valA = a.bytes_downloaded || 0;
            valB = b.bytes_downloaded || 0;
            break;
          case 'upload':
            valA = a.bytes_uploaded || 0;
            valB = b.bytes_uploaded || 0;
            break;
          case 'time':
            valA = a.screen_time_seconds || 0;
            valB = b.screen_time_seconds || 0;
            break;
          case 'share':
          case 'total':
          default:
            valA = (a.bytes_downloaded || 0) + (a.bytes_uploaded || 0);
            valB = (b.bytes_downloaded || 0) + (b.bytes_uploaded || 0);
            break;
        }
        
        if (valA < valB) return sortAscending ? -1 : 1;
        if (valA > valB) return sortAscending ? 1 : -1;
        return 0;
      });
      return list;
    })()
  );

  function toggleSort(field) {
    if (sortField === field) {
      sortAscending = !sortAscending;
    } else {
      sortField = field;
      sortAscending = field === 'name';
    }
  }

  async function loadStats() {
    if (period === "live") {
      loading = false;
      return;
    }
    loading = true;
    errorMsg = "";
    try {
      const res = await invoke("get_historical_stats", { period });
      stats = Array.isArray(res) ? res : [];
    } catch (e) {
      console.error("Failed to query stats", e);
      errorMsg = String(e || "Failed to load telemetry stats");
      stats = [];
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    // 1. Load initial values
    loadStats();

    // 2. Listen to real-time stats
    unlistenStats = await listen("realtime-stats", (event) => {
      const data = event.payload;
      liveDownloadSpeed = data.download_speed;
      liveUploadSpeed = data.upload_speed;
      liveActiveApp = data.active_app || "System";

      // Keep rolling history of last 60 seconds
      liveHistory.download = [...liveHistory.download.slice(1), data.download_speed];
      liveHistory.upload = [...liveHistory.upload.slice(1), data.upload_speed];

      // Log all active process speeds to the live map concurrently
      if (period === "live") {
        // Reset instantaneous current speed across all processes
        for (const key in liveProcessMap) {
          liveProcessMap[key].current_down = 0;
          liveProcessMap[key].current_up = 0;
        }

        // Aggregate speeds for each active process reported by backend
        if (data.process_speeds) {
          data.process_speeds.forEach(p => {
            const app = p.name;
            if (!liveProcessMap[app]) {
              liveProcessMap[app] = { download: 0, upload: 0, time: 0, current_down: 0, current_up: 0 };
            }
            liveProcessMap[app].download += p.download_speed;
            liveProcessMap[app].upload += p.upload_speed;
            liveProcessMap[app].current_down += p.download_speed;
            liveProcessMap[app].current_up += p.upload_speed;
          });
        }

        // Increment screen time for the actual active foreground app
        const app = liveActiveApp;
        if (!liveProcessMap[app]) {
          liveProcessMap[app] = { download: 0, upload: 0, time: 0, current_down: 0, current_up: 0 };
        }
        liveProcessMap[app].time += 1;

        // Force Svelte 5 reactivity refresh for liveProcessMap updates
        liveProcessMap = { ...liveProcessMap };
      }
    });

    unlistenSpeedTest = await listen("speedtest-progress", (event) => {
      const data = event.payload;
      speedTestStage = data.stage;
      speedTestMsg = data.message;
      speedTestProgressPct = data.progress_percent;
      currentTestSpeedBps = data.current_speed;
      if (data.download_speed > 0) peakDownloadBps = data.download_speed;
      if (data.upload_speed > 0) peakUploadBps = data.upload_speed;
      if (data.pings && data.pings.length > 0) dnsPings = data.pings;
      if (data.average_ping > 0) avgPing = data.average_ping;
    });
  });

  onDestroy(() => {
    if (unlistenStats) unlistenStats();
    if (unlistenSpeedTest) unlistenSpeedTest();
  });

  function handlePeriodChange(newPeriod) {
    period = newPeriod;
    loadStats();
  }

  async function openSettings() {
    try {
      await invoke("open_settings");
    } catch (e) {
      console.error("Failed to open settings", e);
    }
  }

  async function closeWindow() {
    try {
      const win = getCurrentWindow();
      await win.close();
    } catch (e) {
      window.close();
    }
  }

  // Format volume helper respecting settings store unit (b, iB, B)
  function formatVolume(bytes) {
    const unit = $settings?.unit || 'B';
    if (!bytes || bytes <= 0) return `0 ${unit === 'b' ? 'b' : 'B'}`;
    
    if (unit === 'b') {
      const bits = bytes * 8;
      const k = 1000;
      const sizes = ['b', 'Kb', 'Mb', 'Gb', 'Tb'];
      const i = Math.floor(Math.log(bits) / Math.log(k));
      return parseFloat((bits / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    } else if (unit === 'ib') {
      const bits = bytes * 8;
      const k = 1024;
      const sizes = ['b', 'Kib', 'Mib', 'Gib', 'Tib'];
      const i = Math.floor(Math.log(bits) / Math.log(k));
      return parseFloat((bits / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    } else if (unit === 'iB') {
      const k = 1024;
      const sizes = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    } else {
      const k = 1000;
      const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    }
  }

  // Format duration helper (seconds -> 1h 45m / 35s)
  function formatDuration(seconds) {
    if (!seconds || seconds <= 0) return '0s';
    const hrs = Math.floor(seconds / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;
    
    if (hrs > 0) return `${hrs}h ${mins}m`;
    if (mins > 0) return `${mins}m ${secs}s`;
    return `${secs}s`;
  }
</script>

<main class="dashboard-panel">
  <!-- Top Bar -->
  <header class="header">
    <div class="title-group">
      <h1>📊 System Analytics & Data Usage</h1>
      <p>Hourly, daily, and long-term telemetry metrics</p>
    </div>
    <div class="actions-group">
      <button class="action-btn group-btn" class:active-group={groupApps} onclick={() => groupApps = !groupApps} title="Toggle process grouping by application name">
        {groupApps ? "🔗 Grouped" : "⛓️ Individual"}
      </button>
      <button class="action-btn settings-btn" onclick={openSettings} title="Open settings">
        ⚙️ Settings
      </button>
      <button class="action-btn speedtest-btn" onclick={() => showSpeedTestModal = true} title="Run Network Speed Test & Multi-DNS Latency">
        ⚡ Speed Test
      </button>
      <button class="action-btn close-btn" onclick={closeWindow} title="Close window">
        ✕ Close
      </button>
    </div>
  </header>

  <!-- Period Filters -->
  <div class="period-bar">
    <button class:active={period === 'live'} onclick={() => handlePeriodChange('live')}>🟢 Live</button>
    <button class:active={period === 'daily'} onclick={() => handlePeriodChange('daily')}>Today</button>
    <button class:active={period === 'hourly'} onclick={() => handlePeriodChange('hourly')}>Last 24 Hrs</button>
    <button class:active={period === 'weekly'} onclick={() => handlePeriodChange('weekly')}>Weekly</button>
    <button class:active={period === 'monthly'} onclick={() => handlePeriodChange('monthly')}>Monthly</button>
    <button class:active={period === 'yearly'} onclick={() => handlePeriodChange('yearly')}>Yearly</button>
  </div>

  <!-- Summary Cards -->
  <section class="summary-cards">
    {#if period === 'live'}
      <div class="card up-card">
        <span class="card-label">Live Upload Speed</span>
        <span class="card-val up-text">↑ {formatSpeed(liveUploadSpeed, $settings.unit)}</span>
      </div>
      <div class="card down-card">
        <span class="card-label">Live Download Speed</span>
        <span class="card-val down-text">↓ {formatSpeed(liveDownloadSpeed, $settings.unit)}</span>
      </div>
      <div class="card time-card">
        <span class="card-label">Active Focus Application</span>
        <span class="card-val app-focus-text">🖥️ {liveActiveApp}</span>
      </div>
    {:else}
      <div class="card up-card">
        <span class="card-label">Total Uploaded</span>
        <span class="card-val up-text">↑ {formatVolume(totalUpload)}</span>
      </div>
      <div class="card down-card">
        <span class="card-label">Total Downloaded</span>
        <span class="card-val down-text">↓ {formatVolume(totalDownload)}</span>
      </div>
      <div class="card time-card">
        <span class="card-label">Total Screen Time</span>
        <span class="card-val time-text">⏱ {formatDuration(totalScreenTime)}</span>
      </div>
    {/if}
  </section>

  <!-- Live Area Chart -->
  {#if period === 'live'}
    <section class="live-chart-section">
      <div class="chart-header">
        <span class="chart-title">Real-time Net Throughput (60s window)</span>
        <div class="chart-stats-badges">
          <span class="chart-stat-pill up" title="Average Upload Speed">Avg ↑: {formatSpeed(avgLiveUp, $settings.unit)}</span>
          <span class="chart-stat-pill down" title="Average Download Speed">Avg ↓: {formatSpeed(avgLiveDown, $settings.unit)}</span>
          <span class="chart-stat-pill up-peak" title="Peak Upload Speed">Peak ↑: {formatSpeed(peakLiveUp, $settings.unit)}</span>
          <span class="chart-stat-pill down-peak" title="Peak Download Speed">Peak ↓: {formatSpeed(peakLiveDown, $settings.unit)}</span>
        </div>
      </div>
      <div class="chart-body">
        <div class="y-axis-container">
          <span class="y-label top">{formatSpeed(scaleLiveMax, $settings.unit)}</span>
          <span class="y-label mid">{formatSpeed(scaleLiveMax / 2, $settings.unit)}</span>
          <span class="y-label bottom">0 {$settings?.unit === 'b' ? 'b/s' : 'B/s'}</span>
        </div>
        <svg viewBox="0 0 800 120" class="live-chart-svg" preserveAspectRatio="none">
          <!-- Horizontal Reference Gridlines -->
          <line x1="0" y1="0" x2="800" y2="0" stroke="var(--card-border)" stroke-dasharray="4 4" opacity="0.6" vector-effect="non-scaling-stroke" />
          <line x1="0" y1="60" x2="800" y2="60" stroke="var(--card-border)" stroke-dasharray="4 4" opacity="0.4" vector-effect="non-scaling-stroke" />
          <line x1="0" y1="120" x2="800" y2="120" stroke="var(--card-border)" opacity="0.6" vector-effect="non-scaling-stroke" />

          <!-- Vertical Time Gridlines (45s, 30s, 15s ago) -->
          <line x1="200" y1="0" x2="200" y2="120" stroke="var(--card-border)" stroke-dasharray="3 3" opacity="0.2" vector-effect="non-scaling-stroke" />
          <line x1="400" y1="0" x2="400" y2="120" stroke="var(--card-border)" stroke-dasharray="3 3" opacity="0.2" vector-effect="non-scaling-stroke" />
          <line x1="600" y1="0" x2="600" y2="120" stroke="var(--card-border)" stroke-dasharray="3 3" opacity="0.2" vector-effect="non-scaling-stroke" />

          <!-- Gradients -->
          <defs>
            <linearGradient id="liveDownGrad" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stop-color="var(--accent-emerald)" stop-opacity="0.25" />
              <stop offset="100%" stop-color="var(--accent-emerald)" stop-opacity="0.00" />
            </linearGradient>
            <linearGradient id="liveUpGrad" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stop-color="var(--accent-blue)" stop-opacity="0.18" />
              <stop offset="100%" stop-color="var(--accent-blue)" stop-opacity="0.00" />
            </linearGradient>
          </defs>

          <!-- Download Area & Line -->
          <path d={liveDownAreaPath} fill="url(#liveDownGrad)" />
          <path d={liveDownPath} fill="none" stroke="var(--accent-emerald)" stroke-width="2" vector-effect="non-scaling-stroke" />

          <!-- Upload Area & Line -->
          <path d={liveUpAreaPath} fill="url(#liveUpGrad)" />
          <path d={liveUpPath} fill="none" stroke="var(--accent-blue)" stroke-width="2" vector-effect="non-scaling-stroke" />
        </svg>
      </div>
      <div class="x-axis-container">
        <span class="x-label">60s ago</span>
        <span class="x-label">45s ago</span>
        <span class="x-label">30s ago</span>
        <span class="x-label">15s ago</span>
        <span class="x-label right">Now</span>
      </div>
    </section>
  {/if}

  <!-- Process List Table -->
  <section class="table-section">
    {#if loading}
      <div class="state-container">
        <div class="spinner"></div>
        <p>Retrieving telemetry records...</p>
      </div>
    {:else if errorMsg}
      <div class="state-container error">
        <p>⚠️ {errorMsg}</p>
        <button class="action-btn" onclick={loadStats}>Retry</button>
      </div>
    {:else}
      {#if sortedList.length === 0}
        <div class="state-container empty">
          <p>📁 No activity recorded yet for this session/period.</p>
          <small>Start loading pages or focusing windows to record statistics.</small>
        </div>
      {:else}
        <div class="table-scroll">
          <table class="stats-table">
            <thead>
              <tr>
                <th onclick={() => toggleSort('name')} class="sortable">
                  Application {sortField === 'name' ? (sortAscending ? '▲' : '▼') : ''}
                </th>
                {#if period === 'live'}
                  <th onclick={() => toggleSort('live_up')} class="sortable">
                    Live Up Speed {sortField === 'live_up' ? (sortAscending ? '▲' : '▼') : ''}
                  </th>
                  <th onclick={() => toggleSort('live_down')} class="sortable">
                    Live Down Speed {sortField === 'live_down' ? (sortAscending ? '▲' : '▼') : ''}
                  </th>
                {/if}
                <th onclick={() => toggleSort('upload')} class="sortable">
                  {period === 'live' ? 'Session Upload' : 'Uploaded'} {sortField === 'upload' ? (sortAscending ? '▲' : '▼') : ''}
                </th>
                <th onclick={() => toggleSort('download')} class="sortable">
                  {period === 'live' ? 'Session Download' : 'Downloaded'} {sortField === 'download' ? (sortAscending ? '▲' : '▼') : ''}
                </th>
                <th onclick={() => toggleSort('time')} class="sortable">
                  Active Session Time {sortField === 'time' ? (sortAscending ? '▲' : '▼') : ''}
                </th>
                <th onclick={() => toggleSort('share')} class="sortable">
                  Share of Usage {sortField === 'share' ? (sortAscending ? '▲' : '▼') : ''}
                </th>
              </tr>
            </thead>
            <tbody>
              {#each sortedList as item}
                {@const appTotal = (item.bytes_downloaded || 0) + (item.bytes_uploaded || 0)}
                {@const pct = grandTotalBytes > 0 ? (appTotal / grandTotalBytes) * 100 : 0}
                <tr>
                  <td class="app-cell">
                    <span class="app-icon">💻</span>
                    <span class="app-title">{item.process_name || 'System'}</span>
                  </td>
                  {#if period === 'live'}
                    <td class="up-val">{formatSpeed(item.current_upload_speed, $settings.unit)}</td>
                    <td class="down-val">{formatSpeed(item.current_download_speed, $settings.unit)}</td>
                  {/if}
                  <td class="up-val">{formatVolume(item.bytes_uploaded)}</td>
                  <td class="down-val">{formatVolume(item.bytes_downloaded)}</td>
                  <td class="time-val">{formatDuration(item.screen_time_seconds)}</td>
                  <td class="share-cell">
                    <div class="progress-wrapper">
                      <div class="progress-bar" style="width: {Math.min(pct, 100)}%"></div>
                    </div>
                    <span class="pct-text">{pct.toFixed(1)}%</span>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  </section>
</main>

{#if showSpeedTestModal}
  <div class="modal-backdrop" onclick={(e) => { if (e.target === e.currentTarget && !speedTestRunning) showSpeedTestModal = false; }}>
    <div class="speedtest-modal">
      <header class="modal-header">
        <div class="modal-title-group">
          <h2>⚡ Network Speedometer & Latency</h2>
          <span class="badge {qualityBadge.class}">{qualityBadge.icon} {qualityBadge.label}</span>
        </div>
        <button class="modal-close" onclick={() => showSpeedTestModal = false} disabled={speedTestRunning}>✕</button>
      </header>
      
      <div class="modal-body">
        <!-- SPEEDOMETER GAUGE -->
        <div class="speedometer-container">
          <div class="gauge-wrapper">
            <svg viewBox="0 0 300 220" class="speedometer-svg" class:theme-upload={speedTestStage === 'upload'}>
              <defs>
                <linearGradient id="gaugeGrad" x1="0%" y1="0%" x2="100%" y2="0%">
                  <stop offset="0%" stop-color={speedTestStage === 'upload' ? '#2563eb' : 'var(--accent-emerald)'} />
                  <stop offset="50%" stop-color={speedTestStage === 'upload' ? '#60a5fa' : '#3b82f6'} />
                  <stop offset="100%" stop-color={speedTestStage === 'upload' ? '#38bdf8' : '#ec4899'} />
                </linearGradient>
              </defs>

              <!-- Background Arc -->
              <path d="M 45 190 A 110 110 0 1 1 255 190" fill="none" stroke="var(--border-color)" stroke-width="16" stroke-linecap="round" />
              
              <!-- Active Progress Arc -->
              <path d="M 45 190 A 110 110 0 1 1 255 190" fill="none" stroke="url(#gaugeGrad)" stroke-width="16" stroke-linecap="round"
                    stroke-dasharray="576" stroke-dashoffset={576 - (576 * gaugeFillPct)} class="gauge-active-path" />

              <!-- Needle Group -->
              <g transform="rotate({needleAngle}, 150, 150)" class="needle-group">
                <line x1="150" y1="150" x2="150" y2="55" stroke={speedTestStage === 'upload' ? '#3b82f6' : 'var(--accent-emerald)'} stroke-width="4" stroke-linecap="round" />
                <circle cx="150" cy="150" r="10" fill="var(--card-bg)" stroke={speedTestStage === 'upload' ? '#3b82f6' : 'var(--accent-emerald)'} stroke-width="4" />
              </g>

              <!-- Dial Tick Labels -->
              <text x="32" y="210" class="dial-tick">0</text>
              <text x="25" y="130" class="dial-tick">10</text>
              <text x="65" y="65" class="dial-tick">50</text>
              <text x="150" y="38" class="dial-tick">100</text>
              <text x="235" y="65" class="dial-tick">250</text>
              <text x="275" y="130" class="dial-tick">500</text>
              <text x="268" y="210" class="dial-tick">1G</text>
            </svg>

            <div class="speed-readout">
              <span class="speed-val" class:upload-text-active={speedTestStage === 'upload'}>{displayedSpeedVal}</span>
              <span class="stage-pill stage-{speedTestStage}">{speedTestStage.toUpperCase()}</span>
            </div>
          </div>

          <!-- Progress Bar -->
          <div class="st-progress-bar-bg">
            <div class="st-progress-fill" style="width: {speedTestProgressPct}%"></div>
          </div>
        </div>

        <!-- SUMMARY STATS BAR -->
        <div class="speedtest-stats-row">
          <div class="st-card down">
            <span class="st-label">Peak Download</span>
            <span class="st-value down-text">↓ {formatSpeed(peakDownloadBps, $settings.unit)}</span>
          </div>
          <div class="st-card up">
            <span class="st-label">Peak Upload</span>
            <span class="st-value up-text">↑ {formatSpeed(peakUploadBps, $settings.unit)}</span>
          </div>
          <div class="st-card ping">
            <span class="st-label">Average Latency</span>
            <span class="st-value ping-text">⚡ {avgPing} ms</span>
          </div>
        </div>

        <!-- MULTI-DNS LATENCY GRID -->
        <div class="dns-section">
          <h3>🌐 Multi-DNS Server Latency Ping</h3>
          <div class="dns-grid">
            {#each dnsPings as ping}
              <div class="dns-card" class:online={ping.latency_ms > 0 && ping.latency_ms < 999}>
                <div class="dns-info">
                  <span class="dns-name">{ping.name}</span>
                  <span class="dns-ip">{ping.ip}</span>
                </div>
                <span class="dns-ping-val">
                  {ping.latency_ms > 0 && ping.latency_ms < 999 ? `${ping.latency_ms} ms` : (ping.latency_ms === 0 ? 'Testing...' : 'Timeout')}
                </span>
              </div>
            {/each}
          </div>
        </div>
      </div>

      <footer class="modal-footer">
        <span class="status-msg">{speedTestMsg}</span>
        <button class="action-btn run-test-btn" onclick={startSpeedTest} disabled={speedTestRunning}>
          {speedTestRunning ? '⏳ Testing Connection...' : (speedTestStage === 'complete' ? '🔄 Retest Speed' : '▶ Run Speed Test')}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  :global(html) {
    /* DEFAULT/LIGHT THEME VARIABLES */
    --bg-color: #f8fafc;
    --panel-bg: radial-gradient(circle at top right, rgba(16, 185, 129, 0.04), transparent 45%);
    --text-primary: #0f172a;
    --text-secondary: #475569;
    --border-color: rgba(15, 23, 42, 0.08);
    --card-bg: #ffffff;
    --card-border: rgba(15, 23, 42, 0.06);
    --card-shadow: 0 4px 12px rgba(15, 23, 42, 0.03);
    --btn-bg: rgba(15, 23, 42, 0.04);
    --btn-border: rgba(15, 23, 42, 0.08);
    --btn-hover: rgba(15, 23, 42, 0.08);
    --table-header: #f1f5f9;
    --table-border: rgba(15, 23, 42, 0.04);
    --table-row-hover: rgba(15, 23, 42, 0.01);
    --app-title: #1e293b;
    --progress-bg: rgba(15, 23, 42, 0.06);
    --accent-emerald: #059669;
    --accent-blue: #2563eb;
    --accent-yellow: #d97706;
    --chart-bg: rgba(15, 23, 42, 0.02);
  }

  @media (prefers-color-scheme: dark) {
    :global(html) {
      /* SYSTEM DEFAULT DARK THEME VARIABLES */
      --bg-color: #0c0d12;
      --panel-bg: radial-gradient(circle at top right, rgba(16, 185, 129, 0.05), transparent 40%);
      --text-primary: #ffffff;
      --text-secondary: #9ca3af;
      --border-color: rgba(255, 255, 255, 0.08);
      --card-bg: rgba(255, 255, 255, 0.03);
      --card-border: rgba(255, 255, 255, 0.05);
      --card-shadow: 0 8px 32px 0 rgba(0, 0, 0, 0.2);
      --btn-bg: rgba(255, 255, 255, 0.06);
      --btn-border: rgba(255, 255, 255, 0.1);
      --btn-hover: rgba(255, 255, 255, 0.12);
      --table-header: #15171e;
      --table-border: rgba(255, 255, 255, 0.06);
      --table-row-hover: rgba(255, 255, 255, 0.02);
      --app-title: #ffffff;
      --progress-bg: rgba(255, 255, 255, 0.06);
      --accent-emerald: #34d399;
      --accent-blue: #60a5fa;
      --accent-yellow: #f59e0b;
      --chart-bg: rgba(255, 255, 255, 0.01);
    }
  }

  :global(html[data-theme="light"]) {
    /* EXPLICIT LIGHT THEME OVERRIDES */
    --bg-color: #f8fafc;
    --panel-bg: radial-gradient(circle at top right, rgba(16, 185, 129, 0.04), transparent 45%);
    --text-primary: #0f172a;
    --text-secondary: #475569;
    --border-color: rgba(15, 23, 42, 0.08);
    --card-bg: #ffffff;
    --card-border: rgba(15, 23, 42, 0.06);
    --card-shadow: 0 4px 12px rgba(15, 23, 42, 0.03);
    --btn-bg: rgba(15, 23, 42, 0.04);
    --btn-border: rgba(15, 23, 42, 0.08);
    --btn-hover: rgba(15, 23, 42, 0.08);
    --table-header: #f1f5f9;
    --table-border: rgba(15, 23, 42, 0.04);
    --table-row-hover: rgba(15, 23, 42, 0.01);
    --app-title: #1e293b;
    --progress-bg: rgba(15, 23, 42, 0.06);
    --accent-emerald: #059669;
    --accent-blue: #2563eb;
    --accent-yellow: #d97706;
    --chart-bg: rgba(15, 23, 42, 0.02);
  }

  :global(html[data-theme="dark"]) {
    /* EXPLICIT DARK THEME OVERRIDES */
    --bg-color: #0c0d12;
    --panel-bg: radial-gradient(circle at top right, rgba(16, 185, 129, 0.05), transparent 40%);
    --text-primary: #ffffff;
    --text-secondary: #9ca3af;
    --border-color: rgba(255, 255, 255, 0.08);
    --card-bg: rgba(255, 255, 255, 0.03);
    --card-border: rgba(255, 255, 255, 0.05);
    --card-shadow: 0 8px 32px 0 rgba(0, 0, 0, 0.2);
    --btn-bg: rgba(255, 255, 255, 0.06);
    --btn-border: rgba(255, 255, 255, 0.1);
    --btn-hover: rgba(255, 255, 255, 0.12);
    --table-header: #15171e;
    --table-border: rgba(255, 255, 255, 0.06);
    --table-row-hover: rgba(255, 255, 255, 0.02);
    --app-title: #ffffff;
    --progress-bg: rgba(255, 255, 255, 0.06);
    --accent-emerald: #34d399;
    --accent-blue: #60a5fa;
    --accent-yellow: #f59e0b;
    --chart-bg: rgba(255, 255, 255, 0.01);
  }

  :global(body) {
    margin: 0;
    padding: 0;
    font-family: 'Outfit', 'Inter', -apple-system, sans-serif;
    background-color: var(--bg-color);
    color: var(--text-primary);
    height: 100vh;
    overflow: hidden;
    user-select: none;
    transition: background-color 0.2s, color 0.2s;
  }

  .dashboard-panel {
    display: flex;
    flex-direction: column;
    height: 100vh;
    box-sizing: border-box;
    padding: 20px;
    gap: 16px;
    background: var(--panel-bg);
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid var(--border-color);
    padding-bottom: 12px;
  }

  .title-group h1 {
    font-size: 18px;
    font-weight: 700;
    margin: 0;
    color: var(--text-primary);
  }

  .title-group p {
    font-size: 12px;
    color: var(--text-secondary);
    margin: 2px 0 0 0;
  }

  .actions-group {
    display: flex;
    gap: 8px;
  }

  .action-btn {
    background: var(--btn-bg);
    border: 1px solid var(--btn-border);
    color: var(--text-primary);
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.2s, border 0.2s, color 0.2s;
  }

  .action-btn:hover {
    background: var(--btn-hover);
  }

  .active-group {
    background: rgba(16, 185, 129, 0.15) !important;
    border-color: rgba(16, 185, 129, 0.3) !important;
    color: var(--accent-emerald) !important;
  }

  .close-btn {
    background: rgba(239, 68, 68, 0.15);
    border-color: rgba(239, 68, 68, 0.25);
    color: #ef4444;
  }

  .close-btn:hover {
    background: rgba(239, 68, 68, 0.25);
    color: #ef4444;
  }

  .period-bar {
    display: flex;
    gap: 6px;
    background: var(--card-bg);
    padding: 4px;
    border-radius: 8px;
    border: 1px solid var(--card-border);
    box-shadow: var(--card-shadow);
  }

  .period-bar button {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 600;
    padding: 6px 10px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s;
  }

  .period-bar button:hover {
    color: var(--text-primary);
    background: var(--btn-bg);
  }

  .period-bar button.active {
    background: var(--accent-emerald);
    color: #ffffff;
    box-shadow: 0 4px 12px rgba(16, 185, 129, 0.25);
  }

  .summary-cards {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
  }

  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: 10px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    box-shadow: var(--card-shadow);
  }

  .card-label {
    font-size: 11px;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
  }

  .card-val {
    font-size: 16px;
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .down-text { color: var(--accent-emerald); }
  .up-text { color: var(--accent-blue); }
  .time-text { color: var(--accent-yellow); }
  .app-focus-text { color: var(--text-primary); }

  /* Live Chart Styles */
  .live-chart-section {
    background: var(--chart-bg);
    border: 1px solid var(--card-border);
    border-radius: 10px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    box-shadow: var(--card-shadow);
  }

  .chart-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .chart-stats-badges {
    display: flex;
    align-items: center;
    gap: 8px;
    font-variant-numeric: tabular-nums;
  }

  .chart-stat-pill {
    font-size: 10.5px;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: 6px;
    line-height: 1.2;
  }

  .chart-stat-pill.down {
    color: var(--accent-emerald);
    background: rgba(16, 185, 129, 0.1);
    border: 1px solid rgba(16, 185, 129, 0.2);
  }

  .chart-stat-pill.down-peak {
    color: var(--accent-emerald);
    background: rgba(16, 185, 129, 0.05);
    border: 1px dashed rgba(16, 185, 129, 0.35);
  }

  .chart-stat-pill.up {
    color: var(--accent-blue);
    background: rgba(59, 130, 246, 0.1);
    border: 1px solid rgba(59, 130, 246, 0.2);
  }

  .chart-stat-pill.up-peak {
    color: var(--accent-blue);
    background: rgba(59, 130, 246, 0.05);
    border: 1px dashed rgba(59, 130, 246, 0.35);
  }

  .chart-body {
    position: relative;
    height: 110px;
    width: 100%;
    overflow: hidden;
  }

  .y-axis-container {
    position: absolute;
    top: 2px;
    left: 8px;
    bottom: 2px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    pointer-events: none;
    z-index: 5;
    font-size: 9px;
    font-weight: 600;
    color: var(--text-secondary);
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
  }

  .y-label.top {
    line-height: 1;
  }

  .y-label.mid {
    line-height: 1;
  }

  .y-label.bottom {
    line-height: 1;
  }

  .live-chart-svg {
    width: 100%;
    height: 100%;
    overflow: visible;
  }

  .x-axis-container {
    display: flex;
    justify-content: space-between;
    padding: 4px 4px 0 34px;
    font-size: 9px;
    font-weight: 600;
    color: var(--text-secondary);
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
  }

  .x-label {
    letter-spacing: -0.2px;
  }

  .x-label.right {
    color: var(--accent-emerald);
    font-weight: 700;
  }

  /* Table styling */
  .table-section {
    flex: 1;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: 10px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    box-shadow: var(--card-shadow);
    user-select: text;
  }

  .table-scroll {
    flex: 1;
    overflow-y: auto;
  }

  .stats-table {
    width: 100%;
    border-collapse: collapse;
    text-align: left;
    font-size: 12px;
  }

  .stats-table th {
    position: sticky;
    top: 0;
    background: var(--table-header);
    color: var(--text-secondary);
    font-weight: 600;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border-color);
  }

  .stats-table th.sortable {
    cursor: pointer;
    user-select: none;
    transition: background-color 0.2s, color 0.2s;
  }

  .stats-table th.sortable:hover {
    background: var(--btn-hover);
    color: var(--text-primary);
  }

  .stats-table td {
    padding: 10px 14px;
    border-bottom: 1px solid var(--table-border);
    color: var(--text-primary);
  }

  .stats-table tr:hover td {
    background: var(--table-row-hover);
  }

  .app-cell {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    color: var(--app-title);
  }

  .down-val { color: var(--accent-emerald); font-weight: 600; }
  .up-val { color: var(--accent-blue); font-weight: 600; }
  .time-val { color: var(--accent-yellow); }

  .share-cell {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .progress-wrapper {
    flex: 1;
    height: 6px;
    background: var(--progress-bg);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-bar {
    height: 100%;
    background: linear-gradient(90deg, var(--accent-emerald), var(--accent-blue));
    border-radius: 3px;
  }

  .pct-text {
    font-size: 11px;
    color: var(--text-secondary);
    width: 36px;
    text-align: right;
  }

  .state-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text-secondary);
    font-size: 13px;
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid var(--border-color);
    border-top-color: var(--accent-emerald);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Speed Test Modal & Speedometer Styling */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.65);
    backdrop-filter: blur(6px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: fadeIn 0.2s ease-out;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .speedtest-modal {
    width: 640px;
    max-width: 92vw;
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    border-radius: 16px;
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.4);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: var(--text-primary);
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 20px;
    border-bottom: 1px solid var(--border-color);
    background: var(--table-header);
  }

  .modal-title-group {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .modal-title-group h2 {
    font-size: 15px;
    font-weight: 700;
    margin: 0;
  }

  .badge {
    font-size: 11px;
    font-weight: 600;
    padding: 3px 9px;
    border-radius: 20px;
  }
  .badge.neutral { background: rgba(156, 163, 175, 0.15); color: var(--text-secondary); }
  .badge.excellent { background: rgba(16, 185, 129, 0.15); color: #10b981; border: 1px solid rgba(16, 185, 129, 0.3); }
  .badge.good { background: rgba(59, 130, 246, 0.15); color: #3b82f6; border: 1px solid rgba(59, 130, 246, 0.3); }
  .badge.fair { background: rgba(245, 158, 11, 0.15); color: #f59e0b; border: 1px solid rgba(245, 158, 11, 0.3); }
  .badge.poor { background: rgba(239, 68, 68, 0.15); color: #ef4444; border: 1px solid rgba(239, 68, 68, 0.3); }

  .modal-close {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    font-size: 16px;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
    transition: background 0.2s;
  }
  .modal-close:hover { background: var(--btn-hover); color: var(--text-primary); }

  .modal-body {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .speedometer-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    position: relative;
  }

  .gauge-wrapper {
    position: relative;
    width: 280px;
    height: 200px;
    display: flex;
    justify-content: center;
  }

  .speedometer-svg {
    width: 100%;
    height: 100%;
    overflow: visible;
  }

  .gauge-active-path {
    transition: stroke-dashoffset 0.25s cubic-bezier(0.4, 0, 0.2, 1);
  }

  .needle-group {
    transition: transform 0.35s cubic-bezier(0.25, 0.1, 0.25, 1);
  }

  .dial-tick {
    font-size: 11px;
    font-weight: 700;
    fill: var(--text-secondary);
    text-anchor: middle;
    user-select: none;
  }

  .speed-readout {
    position: absolute;
    bottom: 25px;
    left: 0;
    right: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  .speed-val {
    font-size: 26px;
    font-weight: 800;
    color: var(--text-primary);
    letter-spacing: -0.5px;
    font-variant-numeric: tabular-nums;
    transition: color 0.3s ease;
  }

  .speed-val.upload-text-active {
    color: #3b82f6;
  }

  .stage-pill {
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.5px;
    padding: 2px 8px;
    border-radius: 10px;
    margin-top: 4px;
    background: rgba(156, 163, 175, 0.15);
    color: var(--text-secondary);
  }
  .stage-pill.stage-download { background: rgba(16, 185, 129, 0.15); color: #10b981; }
  .stage-pill.stage-upload { background: rgba(59, 130, 246, 0.15); color: #3b82f6; }
  .stage-pill.stage-ping { background: rgba(245, 158, 11, 0.15); color: #f59e0b; }
  .stage-pill.stage-complete { background: rgba(16, 185, 129, 0.2); color: #10b981; }

  .st-progress-bar-bg {
    width: 100%;
    height: 4px;
    background: var(--border-color);
    border-radius: 2px;
    overflow: hidden;
    margin-top: 10px;
  }

  .st-progress-fill {
    height: 100%;
    background: linear-gradient(90deg, var(--accent-emerald), #3b82f6);
    transition: width 0.3s ease;
  }

  .speedtest-stats-row {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
  }

  .st-card {
    background: var(--table-header);
    border: 1px solid var(--border-color);
    border-radius: 10px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .st-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .st-value {
    font-size: 16px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .st-value.down-text { color: var(--accent-emerald); }
  .st-value.up-text { color: var(--accent-blue); }
  .st-value.ping-text { color: #f59e0b; }

  .dns-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .dns-section h3 {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-secondary);
    margin: 0;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .dns-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 10px;
  }

  .dns-card {
    background: var(--table-header);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 10px 12px;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .dns-info {
    display: flex;
    flex-direction: column;
  }

  .dns-name {
    font-size: 12px;
    font-weight: 600;
  }

  .dns-ip {
    font-size: 10px;
    color: var(--text-secondary);
  }

  .dns-ping-val {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .dns-card.online .dns-ping-val {
    color: var(--accent-emerald);
  }

  .modal-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 20px;
    border-top: 1px solid var(--border-color);
    background: var(--table-header);
  }

  .status-msg {
    font-size: 12px;
    color: var(--text-secondary);
    font-weight: 500;
  }

  .run-test-btn {
    background: var(--accent-emerald) !important;
    color: #ffffff !important;
    font-weight: 700 !important;
    padding: 8px 18px !important;
    border-radius: 8px !important;
    border: none !important;
    cursor: pointer;
    transition: opacity 0.2s;
  }

  .run-test-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
