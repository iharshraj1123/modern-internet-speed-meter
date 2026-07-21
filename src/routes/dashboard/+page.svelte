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
  });

  onDestroy(() => {
    if (unlistenStats) unlistenStats();
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
      <button class="action-btn refresh-btn" onclick={loadStats} title="Refresh data">
        🔄 Refresh
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
      <div class="card down-card">
        <span class="card-label">Live Download Speed</span>
        <span class="card-val down-text">↓ {formatSpeed(liveDownloadSpeed, $settings.unit)}</span>
      </div>
      <div class="card up-card">
        <span class="card-label">Live Upload Speed</span>
        <span class="card-val up-text">↑ {formatSpeed(liveUploadSpeed, $settings.unit)}</span>
      </div>
      <div class="card time-card">
        <span class="card-label">Active Focus Application</span>
        <span class="card-val app-focus-text">🖥️ {liveActiveApp}</span>
      </div>
    {:else}
      <div class="card down-card">
        <span class="card-label">Total Downloaded</span>
        <span class="card-val down-text">↓ {formatVolume(totalDownload)}</span>
      </div>
      <div class="card up-card">
        <span class="card-label">Total Uploaded</span>
        <span class="card-val up-text">↑ {formatVolume(totalUpload)}</span>
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
          <span class="chart-stat-pill down" title="Average Download Speed">Avg ↓: {formatSpeed(avgLiveDown, $settings.unit)}</span>
          <span class="chart-stat-pill up" title="Average Upload Speed">Avg ↑: {formatSpeed(avgLiveUp, $settings.unit)}</span>
          <span class="chart-stat-pill down-peak" title="Peak Download Speed">Peak ↓: {formatSpeed(peakLiveDown, $settings.unit)}</span>
          <span class="chart-stat-pill up-peak" title="Peak Upload Speed">Peak ↑: {formatSpeed(peakLiveUp, $settings.unit)}</span>
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
          <path d={liveUpPath} fill="none" stroke="var(--accent-blue)" stroke-width="1.5" stroke-dasharray="3 3" vector-effect="non-scaling-stroke" />
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
                  <th onclick={() => toggleSort('live_down')} class="sortable">
                    Live Down Speed {sortField === 'live_down' ? (sortAscending ? '▲' : '▼') : ''}
                  </th>
                  <th onclick={() => toggleSort('live_up')} class="sortable">
                    Live Up Speed {sortField === 'live_up' ? (sortAscending ? '▲' : '▼') : ''}
                  </th>
                {/if}
                <th onclick={() => toggleSort('download')} class="sortable">
                  {period === 'live' ? 'Session Download' : 'Downloaded'} {sortField === 'download' ? (sortAscending ? '▲' : '▼') : ''}
                </th>
                <th onclick={() => toggleSort('upload')} class="sortable">
                  {period === 'live' ? 'Session Upload' : 'Uploaded'} {sortField === 'upload' ? (sortAscending ? '▲' : '▼') : ''}
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
                    <td class="down-val">{formatSpeed(item.current_download_speed, $settings.unit)}</td>
                    <td class="up-val">{formatSpeed(item.current_upload_speed, $settings.unit)}</td>
                  {/if}
                  <td class="down-val">{formatVolume(item.bytes_downloaded)}</td>
                  <td class="up-val">{formatVolume(item.bytes_uploaded)}</td>
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
</style>
