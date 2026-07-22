<script>
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { settings, ACCENT_COLORS, formatSpeed } from "../../lib/settingsStore";

  let activeTab = $state("general");
  let appearanceSubTab = $state("theme"); // 'theme' or 'graph'
  let autostartEnabled = $state(false);
  let clearSuccess = $state(false);

  // Storage Analytics states
  let dbInfo = $state(null);
  let loadingDbInfo = $state(false);
  let vacuumSuccess = $state(false);
  let rawRetention = $state(7);
  let hourlyRetention = $state(90);

  // Live Debug Inspector states (10-tick rolling buffer)
  let debugHistory = $state([]);
  let selectedTickIndex = $state(-1); // -1 = Live Latest, 0..9 = past tick index
  let debugPaused = $state(false);
  let debugInterval;

  async function fetchDebugInfo() {
    if (debugPaused && selectedTickIndex !== -1) return;
    try {
      const info = await invoke("get_telemetry_debug_info");
      if (info) {
        const timeStr = new Date().toLocaleTimeString();
        const entry = { time: timeStr, info };
        debugHistory = [...debugHistory.slice(-9), entry];
      }
    } catch (err) {
      console.error("Failed to query debug info", err);
    }
  }

  let activeDebugEntry = $derived(
    (() => {
      if (debugHistory.length === 0) return null;
      if (selectedTickIndex >= 0 && selectedTickIndex < debugHistory.length) {
        return debugHistory[selectedTickIndex];
      }
      return debugHistory[debugHistory.length - 1];
    })()
  );

  // Hotkey states
  let recordingHotkey = $state(false);

  async function visitDonationLink(url) {
    try {
      await invoke("open_url", { url });
    } catch (e) {
      console.error("Failed to open link", e);
      try {
        await openUrl(url);
      } catch (err) {
        window.open(url, "_blank");
      }
    }
  }

  $effect(() => {
    const accent = $settings.accentColor || "emerald";
    const colors = ACCENT_COLORS[accent] || ACCENT_COLORS.emerald;
    
    // Dynamically apply accent color to CSS variables on HTML element
    document.documentElement.style.setProperty('--accent-emerald', colors.dark);
    document.documentElement.style.setProperty('--input-focus', colors.dark);
  });

  // Pull database metrics when Data tab becomes active
  $effect(() => {
    if (activeTab === 'data') {
      loadDbInfo();
    }
  });

  $effect(() => {
    const theme = $settings.dashboardTheme || $settings.theme || 'system';
    if (theme === 'system') {
      document.documentElement.removeAttribute('data-theme');
    } else {
      document.documentElement.setAttribute('data-theme', theme);
    }
  });

  let isElevated = $state(false);
  let showEtwModal = $state(false);
  let telemetrySubTab = $state('engine'); // 'engine', 'limits'

  let currentEngine = $derived($settings.telemetryEngine || 'io');

  let attributionTitle = $derived(
    currentEngine === 'etw'
      ? 'Process Attribution Accounting (Kernel ETW Mode)'
      : currentEngine === 'estats'
        ? 'Process Attribution Accounting (TCP EStats Mode)'
        : 'Process Attribution Accounting (Process I/O Mode)'
  );

  let exactBadgeLabel = $derived(
    currentEngine === 'etw'
      ? 'Task Manager / ETW'
      : currentEngine === 'estats'
        ? 'Resource Monitor'
        : 'Process I/O Raw'
  );

  let exactDesc = $derived(
    currentEngine === 'etw'
      ? 'Reports raw payload bytes per process as captured by ETW kernel tracing without scaling for packet headers or ACKs.'
      : currentEngine === 'estats'
        ? 'Reports raw TCP socket payload bytes per process as captured by TCP EStats (matches Windows Resource Monitor).'
        : 'Reports raw process activity read/write bytes per process as reported by Windows I/O counters.'
  );

  let proportionalDesc = $derived(
    currentEngine === 'io'
      ? 'Scales active process I/O counters proportionally so per-process stats match 100% of physical NIC hardware bandwidth.'
      : 'Scales active process speeds proportionally up/down so all process totals equal 100% of physical NIC hardware bandwidth. Eliminates "System" rows while staying 100% accurate to your ISP bill.'
  );

  function selectTelemetryEngine(mode) {
    updateSetting("telemetryEngine", mode);
    updateSetting("useEtwTelemetry", mode === "etw");
    try {
      invoke("set_telemetry_engine", { engine: mode });
    } catch (e) {
      console.error("Failed to set telemetry engine", e);
    }

    if (mode === "etw" && !isElevated) {
      showEtwModal = true;
    }
  }

  async function confirmRestartAsAdmin() {
    showEtwModal = false;
    try {
      updateSetting("telemetryEngine", "etw");
      updateSetting("useEtwTelemetry", true);
      await settings.syncWithBackend({ ...$settings, telemetryEngine: "etw", useEtwTelemetry: true });
      await invoke("set_telemetry_engine", { engine: "etw" });
      await invoke("restart_as_admin");
    } catch (err) {
      console.error("Failed to restart as Admin", err);
      alert("Elevation cancelled or failed: " + err);
    }
  }

  onMount(async () => {
    try {
      autostartEnabled = await invoke("plugin:autostart|is_enabled");
    } catch (e) {
      console.error("Autostart query failed", e);
    }
    try {
      isElevated = await invoke("is_process_elevated");
    } catch (e) {
      console.error("Elevation check failed", e);
    }
    fetchDebugInfo();
    debugInterval = setInterval(fetchDebugInfo, 1000);
  });

  onDestroy(() => {
    if (debugInterval) clearInterval(debugInterval);
  });

  async function handleAutostartToggle(e) {
    const checked = e.target.checked;
    try {
      if (checked) {
        await invoke("plugin:autostart|enable");
      } else {
        await invoke("plugin:autostart|disable");
      }
      autostartEnabled = checked;
    } catch (err) {
      console.error("Failed to toggle autostart", err);
      e.target.checked = !checked;
    }
  }

  function updateSetting(key, value) {
    settings.update(s => {
      const next = { ...s, [key]: value };
      settings.syncWithBackend(next);
      return next;
    });
  }

  async function loadDbInfo() {
    loadingDbInfo = true;
    try {
      const info = await invoke("get_db_info");
      dbInfo = info;
      rawRetention = info.raw_retention_days;
      hourlyRetention = info.hourly_retention_days;
    } catch (err) {
      console.error("Failed to fetch database info", err);
    } finally {
      loadingDbInfo = false;
    }
  }

  async function saveRetentionPolicy() {
    try {
      await invoke("set_retention_policy", {
        rawDays: parseInt(rawRetention),
        hourlyDays: parseInt(hourlyRetention)
      });
      loadDbInfo();
    } catch (err) {
      console.error("Failed to save retention policy", err);
    }
  }

  async function handleVacuumDb() {
    try {
      await invoke("vacuum_db");
      vacuumSuccess = true;
      setTimeout(() => vacuumSuccess = false, 3000);
      loadDbInfo();
    } catch (err) {
      console.error("Failed to vacuum database", err);
    }
  }

  async function handleClearDb() {
    if (confirm("Are you sure you want to clear all historical network and screen time usage data?")) {
      try {
        await invoke("get_historical_stats", { period: "clear" });
        clearSuccess = true;
        setTimeout(() => clearSuccess = false, 3000);
        loadDbInfo();
      } catch (err) {
        console.error("Failed to clear database", err);
      }
    }
  }

  function startRecordingHotkey() {
    recordingHotkey = true;
  }

  function handleKeyDown(event) {
    if (!recordingHotkey) return;
    event.preventDefault();

    const key = event.key;
    if (["Control", "Shift", "Alt", "Meta"].includes(key)) {
      return;
    }

    const parts = [];
    if (event.ctrlKey) parts.push("Ctrl");
    if (event.shiftKey) parts.push("Shift");
    if (event.altKey) parts.push("Alt");
    
    let keyName = key;
    if (keyName === " ") keyName = "Space";
    if (keyName.length === 1) keyName = keyName.toUpperCase();
    parts.push(keyName);

    const shortcut = parts.join("+");
    updateSetting("globalHotkey", shortcut);
    recordingHotkey = false;

    invoke("register_hotkey", { shortcut }).catch(err => {
      console.error("Failed to register recorded hotkey", err);
    });
  }

  function formatBytes(bytes) {
    if (bytes === undefined || isNaN(bytes)) return '0 B';
    if (bytes === 0) return '0 B';
    const k = 1000;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  }
</script>

<svelte:window onkeydown={handleKeyDown} />

<main class="settings-panel">
  <header class="header">
    <h1>Settings</h1>
    <p>Configure speed widget & monitoring options</p>
  </header>

  <div class="content-wrapper">
    <!-- Sidebar navigation -->
    <nav class="sidebar">
      <button 
        class="nav-btn" 
        class:active={activeTab === 'general'} 
        onclick={() => activeTab = 'general'}
      >
        General
      </button>
      <nav class="nav-divider"></nav>
      <button 
        class="nav-btn" 
        class:active={activeTab === 'appearance'} 
        onclick={() => activeTab = 'appearance'}
      >
        Appearance
      </button>
      <nav class="nav-divider"></nav>
      <button 
        class="nav-btn" 
        class:active={activeTab === 'telemetry'} 
        onclick={() => activeTab = 'telemetry'}
      >
        Telemetry
      </button>
      <nav class="nav-divider"></nav>
      <button 
        class="nav-btn" 
        class:active={activeTab === 'data'} 
        onclick={() => activeTab = 'data'}
      >
        Data & Storage
      </button>
      <nav class="nav-divider"></nav>
      <button 
        class="nav-btn support-tab-btn" 
        class:active={activeTab === 'support'} 
        onclick={() => activeTab = 'support'}
      >
        Support Us
      </button>
    </nav>

    <!-- Main setting inputs -->
    <div class="main-settings">
      {#if activeTab === 'general'}
        <section class="section">
          <h2>System Integration</h2>
          
          <div class="setting-item">
            <div class="setting-info">
              <label for="autostart">Start with PC</label>
              <span>Launch the speed meter automatically when your computer starts</span>
            </div>
            <label class="switch">
              <input 
                id="autostart" 
                type="checkbox" 
                checked={autostartEnabled} 
                onchange={handleAutostartToggle} 
              />
              <span class="slider"></span>
            </label>
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label for="minimizeTray">Minimize to Tray on Close</label>
              <span>Keep the app running in the tray when the close button is clicked</span>
            </div>
            <label class="switch">
              <input 
                id="minimizeTray" 
                type="checkbox" 
                checked={$settings.minimizeToTray ?? true} 
                onchange={(e) => updateSetting("minimizeToTray", e.target.checked)} 
              />
              <span class="slider"></span>
            </label>
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>Global Show/Hide Shortcut</label>
              <span>Toggle widget visibility from anywhere using keyboard keys</span>
            </div>
            <button 
              class="hotkey-btn" 
              class:recording={recordingHotkey}
              onclick={startRecordingHotkey}
            >
              {recordingHotkey ? "Press key combo..." : ($settings.globalHotkey || "Click to assign")}
            </button>
          </div>
        </section>

      {:else if activeTab === 'appearance'}
        <section class="section">
          <h2>Appearance & Theme</h2>

          <div class="sub-tab-bar">
            <button 
              class="sub-tab-btn" 
              class:active={appearanceSubTab === 'theme'}
              onclick={() => appearanceSubTab = 'theme'}
            >
              Theme & Interface
            </button>
            <button 
              class="sub-tab-btn" 
              class:active={appearanceSubTab === 'graph'}
              onclick={() => appearanceSubTab = 'graph'}
            >
              Widget & Graphs
            </button>
          </div>

          {#if appearanceSubTab === 'theme'}
            <div class="setting-item">
              <div class="setting-info">
                <label>Accent Color Theme</label>
                <span>Choose your personal styling highlights across all views</span>
              </div>
              <div class="accent-picker">
                {#each Object.entries(ACCENT_COLORS) as [colorKey, val]}
                  <button 
                    class="accent-swatch {colorKey}" 
                    class:active={$settings.accentColor === colorKey}
                    style="background-color: {val.dark};" 
                    onclick={() => updateSetting("accentColor", colorKey)}
                    title={val.name}
                  ></button>
                {/each}
              </div>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="widgetThemeSelect">Desktop Widget Theme</label>
                <span>Independent color theme for floating overlay widget</span>
              </div>
              <select 
                id="widgetThemeSelect" 
                class="select-input"
                value={$settings.widgetTheme || $settings.theme || 'system'} 
                onchange={(e) => updateSetting("widgetTheme", e.target.value)}
              >
                <option value="system">System Default</option>
                <option value="dark">Dark Theme</option>
                <option value="light">Light Theme</option>
              </select>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="dashboardThemeSelect">Dashboard & Settings Theme</label>
                <span>Color theme for main analytics and settings windows</span>
              </div>
              <select 
                id="dashboardThemeSelect" 
                class="select-input"
                value={$settings.dashboardTheme || $settings.theme || 'system'} 
                onchange={(e) => updateSetting("dashboardTheme", e.target.value)}
              >
                <option value="system">System Default</option>
                <option value="dark">Dark Theme</option>
                <option value="light">Light Theme</option>
              </select>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="unitType">Measurement Unit</label>
                <span>Format speed metrics standard</span>
              </div>
              <select 
                id="unitType" 
                class="select-input"
                value={$settings.unit} 
                onchange={(e) => updateSetting("unit", e.target.value)}
              >
                <option value="B">Decimal Bytes (KB/s, MB/s)</option>
                <option value="iB">Binary Bytes (KiB/s, MiB/s)</option>
                <option value="b">Decimal Bits (Kbps, Mbps)</option>
                <option value="ib">Binary Bits (Kibps, Mibps)</option>
              </select>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="opacity">Widget Opacity ({Math.round($settings.opacity * 100)}%)</label>
                <span>Adjust overlay transparency on the desktop</span>
              </div>
              <input 
                id="opacity"
                type="range" 
                min="0.1" 
                max="1.0" 
                step="0.05"
                value={$settings.opacity} 
                oninput={(e) => updateSetting("opacity", parseFloat(e.target.value))}
                class="range-input"
              />
            </div>

          {:else if appearanceSubTab === 'graph'}
            <div class="setting-item">
              <div class="setting-info">
                <label for="graphType">Graph Representation</label>
                <span>Select the real-time speed chart overlay layout</span>
              </div>
              <select 
                id="graphType" 
                class="select-input"
                value={$settings.graphType} 
                onchange={(e) => updateSetting("graphType", e.target.value)}
              >
                <option value="combined">Combined Graph (Dual Upload & Download Overlay)</option>
                <option value="separate">Separate Graphs (Stacked Upload & Download Boxes)</option>
                <option value="down_only">Download Only (Single Graph)</option>
                <option value="up_only">Upload Only (Single Graph)</option>
                <option value="hidden">Hidden (Metrics text only, saves CPU)</option>
              </select>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="downGraphStyle">Widget Download Graph Style</label>
                <span>Choose line style for widget download speed visualization</span>
              </div>
              <select 
                id="downGraphStyle" 
                class="select-input"
                value={$settings.downGraphStyle || 'dashed'} 
                onchange={(e) => updateSetting("downGraphStyle", e.target.value)}
              >
                <option value="dashed">Dashed Line (Classic)</option>
                <option value="solid">Smooth Solid Line (Modern)</option>
              </select>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="upGraphStyle">Widget Upload Graph Style</label>
                <span>Choose line style for widget upload speed visualization</span>
              </div>
              <select 
                id="upGraphStyle" 
                class="select-input"
                value={$settings.upGraphStyle || 'dashed'} 
                onchange={(e) => updateSetting("upGraphStyle", e.target.value)}
              >
                <option value="dashed">Dashed Line (Classic)</option>
                <option value="solid">Smooth Solid Line (Modern)</option>
              </select>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="filterWidgetNoise">Filter Sub-Unit Background Noise in Widget</label>
                <span>Suppress minor background telemetry (&lt; 1 KB/s or &lt; 1 Kbps) to keep widget UI clean and distraction-free</span>
              </div>
              <label class="switch">
                <input 
                  id="filterWidgetNoise" 
                  type="checkbox" 
                  checked={$settings.filterWidgetNoise ?? true} 
                  onchange={(e) => updateSetting("filterWidgetNoise", e.target.checked)} 
                />
                <span class="slider"></span>
              </label>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="showWidgetPeak">Show Peak Label in Widget</label>
                <span>Display real-time peak speed overlay tag on desktop widget graph</span>
              </div>
              <label class="switch">
                <input 
                  id="showWidgetPeak" 
                  type="checkbox" 
                  checked={$settings.showWidgetPeak ?? true} 
                  onchange={(e) => updateSetting("showWidgetPeak", e.target.checked)} 
                />
                <span class="slider"></span>
              </label>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="showPing">Show Ping / Latency in Widget</label>
                <span>Display real-time network latency indicator alongside speeds in widget</span>
              </div>
              <label class="switch">
                <input 
                  id="showPing" 
                  type="checkbox" 
                  checked={$settings.showPing ?? true} 
                  onchange={(e) => updateSetting("showPing", e.target.checked)} 
                />
                <span class="slider"></span>
              </label>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="widgetLocked">Lock Widget Position</label>
                <span>Disable overlay dragging and fix it in place</span>
              </div>
              <label class="switch">
                <input 
                  id="widgetLocked"
                  type="checkbox" 
                  checked={$settings.locked} 
                  onchange={(e) => updateSetting("locked", e.target.checked)} 
                />
                <span class="slider"></span>
              </label>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="clickThrough">Click-Through Mode</label>
                <span>Locks widget and allows clicking through it directly</span>
              </div>
              <label class="switch">
                <input 
                  id="clickThrough"
                  type="checkbox" 
                  checked={$settings.clickThrough} 
                  onchange={(e) => updateSetting("clickThrough", e.target.checked)} 
                />
                <span class="slider"></span>
              </label>
            </div>
          {/if}
        </section>

      {:else if activeTab === 'telemetry'}
        <section class="section">
          <h2>Telemetry Options</h2>

          <div class="sub-tab-bar">
            <button 
              type="button"
              class="sub-tab-btn" 
              class:active={telemetrySubTab === 'engine'} 
              onclick={() => telemetrySubTab = 'engine'}
            >
              Measurement Engine
            </button>
            <button 
              type="button"
              class="sub-tab-btn" 
              class:active={telemetrySubTab === 'limits'} 
              onclick={() => telemetrySubTab = 'limits'}
            >
              Performance & Limits
            </button>
          </div>

          {#if telemetrySubTab === 'engine'}
            <div class="setting-item" style="flex-direction: column; align-items: stretch; gap: 12px; border-bottom: none;">
              <div class="setting-info">
                <label>Telemetry Measurement Engine</label>
                <span>Select how Internet Speed Meter measures and attributes per-process network traffic</span>
              </div>
              
              <div class="engine-cards">
                <!-- Option 1: Process I/O (Default) -->
                <button 
                  type="button"
                  class="engine-card" 
                  class:active={($settings.telemetryEngine || 'io') === 'io'}
                  onclick={() => selectTelemetryEngine('io')}
                >
                  <div class="engine-card-header">
                    <strong>1. Process Activity Mode</strong>
                    <span class="badge default">Standard Default</span>
                  </div>
                  <p>Lightweight app activity monitoring. Extremely low CPU usage, reliable, requires no Administrator permission.</p>
                </button>

                <!-- Option 2: TCP EStats -->
                <button 
                  type="button"
                  class="engine-card" 
                  class:active={($settings.telemetryEngine || 'io') === 'estats'}
                  onclick={() => selectTelemetryEngine('estats')}
                >
                  <div class="engine-card-header">
                    <strong>2. Network Socket Mode</strong>
                    <span class="badge">Standard User</span>
                  </div>
                  <p>Direct web socket tracking. High accuracy for web browsing and downloads, requires no Administrator permission.</p>
                </button>

                <!-- Option 3: Kernel ETW Tracing -->
                <button 
                  type="button"
                  class="engine-card" 
                  class:active={($settings.telemetryEngine || 'estats') === 'etw'}
                  onclick={() => selectTelemetryEngine('etw')}
                >
                  <div class="engine-card-header">
                    <strong>3. Kernel Driver Mode</strong>
                    <span class="badge admin">Requires Admin</span>
                  </div>
                  <p>Maximum precision kernel tracking. 100% complete accuracy for all apps, web streams, and video calls.</p>
                  {#if ($settings.telemetryEngine || 'estats') === 'etw'}
                    <div style="margin-top: 6px; font-size: 11px; font-weight: 600;">
                      {#if isElevated}
                        <span style="color: var(--accent-emerald);">🛡️ Admin Active — Kernel Mode Running</span>
                      {:else}
                        <span style="color: var(--accent-yellow);">
                          Requires Admin. 
                          <span style="text-decoration: underline; cursor: pointer; color: var(--accent-emerald);" onclick={confirmRestartAsAdmin}>Restart as Admin</span>
                        </span>
                      {/if}
                    </div>
                  {/if}
                </button>
              </div>

              <div class="attribution-selector-box" style="margin-top: 24px;">
                <h4 style="font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; opacity: 0.8; margin-bottom: 12px;">{attributionTitle}</h4>
                
                <div class="engine-cards">
                  <!-- Option 1: Proportional Allocation (Recommended Default) -->
                  <button
                    type="button"
                    class="engine-card"
                    class:active={($settings.attributionMode || 'proportional') === 'proportional'}
                    onclick={() => updateSetting("attributionMode", "proportional")}
                  >
                    <div class="engine-card-header">
                      <div class="engine-card-title">
                        <strong>Proportional ISP Allocation (Recommended Default)</strong>
                        <span class="badge etw">100% ISP Match</span>
                      </div>
                      <div class="radio-indicator">
                        <div class="radio-dot"></div>
                      </div>
                    </div>
                    <p class="engine-card-desc">
                      {proportionalDesc}
                    </p>
                  </button>

                  <!-- Option 2: Exact Payload Only -->
                  <button
                    type="button"
                    class="engine-card"
                    class:active={$settings.attributionMode === 'exact'}
                    onclick={() => updateSetting("attributionMode", "exact")}
                  >
                    <div class="engine-card-header">
                      <div class="engine-card-title">
                        <strong>Exact Payload Only</strong>
                        <span class="badge user">{exactBadgeLabel}</span>
                      </div>
                      <div class="radio-indicator">
                        <div class="radio-dot"></div>
                      </div>
                    </div>
                    <p class="engine-card-desc">
                      {exactDesc}
                    </p>
                  </button>

                  <!-- Option 3: Separate System Overhead Row -->
                  <button
                    type="button"
                    class="engine-card"
                    class:active={$settings.attributionMode === 'system_row'}
                    onclick={() => updateSetting("attributionMode", "system_row")}
                  >
                    <div class="engine-card-header">
                      <div class="engine-card-title">
                        <strong>Separate System Overhead Row</strong>
                        <span class="badge etw">Explicit Remainder</span>
                      </div>
                      <div class="radio-indicator">
                        <div class="radio-dot"></div>
                      </div>
                    </div>
                    <p class="engine-card-desc">
                      Reports exact payloads per process, and groups all unallocated network overhead and kernel traffic into a dedicated "System / Unattributed" process row.
                    </p>
                  </button>
                </div>
              </div>
            </div>

            <!-- Live Telemetry Engine Debug Inspector -->
            <div class="engine-explanation-box" style="margin-top: 16px; border-color: var(--accent-emerald);">
              <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;">
                <h4 style="margin: 0; color: var(--accent-emerald);">🔍 Live Telemetry Engine Debugger</h4>
                <div style="display: flex; gap: 8px; align-items: center;">
                  <button class="action-btn-styled" style="padding: 4px 10px; font-size: 11px;" onclick={() => debugPaused = !debugPaused}>
                    {debugPaused ? "▶️ Resume Stream" : "⏸️ Pause Stream"}
                  </button>
                  <button class="action-btn-styled" style="padding: 4px 10px; font-size: 11px;" onclick={() => { debugPaused = false; selectedTickIndex = -1; fetchDebugInfo(); }}>
                    Jump to Live
                  </button>
                </div>
              </div>

              {#if debugHistory.length > 0}
                <!-- 10-Tick Rolling History Timeline -->
                <div style="display: flex; gap: 6px; align-items: center; margin-bottom: 14px; overflow-x: auto; padding-bottom: 4px;">
                  <span style="font-size: 11px; font-weight: 600; color: var(--text-secondary); white-space: nowrap;">10-Tick History:</span>
                  {#each debugHistory as entry, idx}
                    <button
                      type="button"
                      class="subtab-btn"
                      style="padding: 3px 8px; font-size: 10.5px; border: 1px solid var(--border-color);"
                      class:active={(selectedTickIndex === idx) || (selectedTickIndex === -1 && idx === debugHistory.length - 1)}
                      onclick={() => { selectedTickIndex = idx; debugPaused = true; }}
                    >
                      #{idx + 1} ({entry.time})
                    </button>
                  {/each}
                </div>
              {/if}

              {#if activeDebugEntry && activeDebugEntry.info}
                {@const info = activeDebugEntry.info}
                <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 10px; margin-bottom: 14px;">
                  <div class="debug-metric-card">
                    <span class="debug-label">Engine Mode</span>
                    <strong class="debug-value">Mode {info.engine_mode} ({info.engine_name})</strong>
                  </div>
                  <div class="debug-metric-card">
                    <span class="debug-label">Process Elevation</span>
                    <strong class="debug-value">{info.is_elevated ? "🛡️ ADMIN (Elevated)" : "👤 USER (Non-Admin)"}</strong>
                  </div>
                  <div class="debug-metric-card">
                    <span class="debug-label">ETW Thread State</span>
                    <strong class="debug-value">{info.etw_active ? "🟢 ACTIVE (Running)" : "🔴 INACTIVE"}</strong>
                  </div>
                  <div class="debug-metric-card">
                    <span class="debug-label">ETW Packets / Sec</span>
                    <strong class="debug-value">{info.etw_events_last_sec.toLocaleString()} pkts/s</strong>
                  </div>
                  <div class="debug-metric-card">
                    <span class="debug-label">ETW Captured Speed</span>
                    <strong class="debug-value">{formatSpeed(info.etw_bytes_last_sec, 'B')}</strong>
                  </div>
                  <div class="debug-metric-card">
                    <span class="debug-label">NIC Hardware Speed</span>
                    <strong class="debug-value">{formatSpeed(info.nic_rx_bytes_last_sec, 'B')}</strong>
                  </div>
                </div>

                <div style="margin: 8px 0 12px 0; padding: 8px 12px; background: rgba(0, 0, 0, 0.25); border-radius: 6px; font-family: monospace; font-size: 11px; color: var(--accent-emerald); border: 1px solid var(--border-color);">
                  <strong>ETW Kernel Status Log:</strong> {info.etw_status_log || "No log status available"}
                </div>

                {#if info.raw_etw_pid_samples && info.raw_etw_pid_samples.length > 0}
                  <h5 style="margin: 10px 0 6px 0; font-size: 12px; color: var(--text-primary);">
                    Active Captured Processes at {activeDebugEntry.time} ({info.active_etw_pids})
                  </h5>
                  <div style="max-height: 180px; overflow-y: auto; background: var(--widget-hover-bg); border-radius: 6px; padding: 8px;">
                    <table style="width: 100%; font-size: 11px; text-align: left; border-collapse: collapse;">
                      <thead>
                        <tr style="border-bottom: 1px solid var(--border-color); color: var(--text-secondary);">
                          <th style="padding: 4px;">PID</th>
                          <th style="padding: 4px;">Executable</th>
                          <th style="padding: 4px;">Rx Bytes / s</th>
                          <th style="padding: 4px;">Tx Bytes / s</th>
                        </tr>
                      </thead>
                      <tbody>
                        {#each info.raw_etw_pid_samples as [pid, exe, rx, tx]}
                          <tr style="border-bottom: 1px solid var(--border-color);">
                            <td style="padding: 4px; font-family: monospace;">{pid}</td>
                            <td style="padding: 4px; font-weight: 600; color: var(--text-primary);">{exe}</td>
                            <td style="padding: 4px;">{formatSpeed(rx, 'B')}</td>
                            <td style="padding: 4px;">{formatSpeed(tx, 'B')}</td>
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  </div>
                {:else}
                  <div style="font-size: 11px; color: var(--text-secondary); font-style: italic; margin-top: 8px;">
                    No active process deltas captured at {activeDebugEntry.time}. (Stream video or browse web to inspect packet streams)
                  </div>
                {/if}
              {/if}
            </div>

          {:else if telemetrySubTab === 'limits'}
            <div class="setting-item">
              <div class="setting-info">
                <label for="idleTimeout">Idle Inactivity Timeout ({$settings.idleTimeout} mins)</label>
                <span>Pause active screen time logging after user inactivity</span>
              </div>
              <input 
                id="idleTimeout"
                type="range" 
                min="1" 
                max="30" 
                step="1"
                value={$settings.idleTimeout} 
                oninput={(e) => updateSetting("idleTimeout", parseInt(e.target.value))}
                class="range-input"
              />
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="batterySaver">Battery Saver Mode</label>
                <span>Automatically pause logging if system battery drops below 20%</span>
              </div>
              <label class="switch">
                <input 
                  id="batterySaver"
                  type="checkbox" 
                  checked={$settings.batterySaver} 
                  onchange={(e) => updateSetting("batterySaver", e.target.checked)} 
                />
                <span class="slider"></span>
              </label>
            </div>

            <div class="setting-item" style="margin-top: 10px; border-top: 1px solid var(--border-color); padding-top: 15px;">
              <div class="setting-info">
                <label for="dailyLimit">Daily Data Cap (GB)</label>
                <span>Monitor and alert when network usage exceeds daily budget</span>
              </div>
              <div style="display: flex; align-items: center; gap: 8px;">
                <input 
                  id="dailyLimit"
                  type="number" 
                  min="1" 
                  max="500" 
                  value={$settings.dailyLimitGB} 
                  disabled={!$settings.dailyLimitEnabled}
                  onchange={(e) => updateSetting("dailyLimitGB", parseFloat(e.target.value))}
                  class="num-input"
                />
                <label class="switch">
                  <input 
                    type="checkbox" 
                    checked={$settings.dailyLimitEnabled} 
                    onchange={(e) => updateSetting("dailyLimitEnabled", e.target.checked)} 
                  />
                  <span class="slider"></span>
                </label>
              </div>
            </div>

            <div class="setting-item">
              <div class="setting-info">
                <label for="monthlyLimit">Monthly Data Cap (GB)</label>
                <span>Monitor and alert when network usage exceeds monthly budget</span>
              </div>
              <div style="display: flex; align-items: center; gap: 8px;">
                <input 
                  id="monthlyLimit"
                  type="number" 
                  min="1" 
                  max="5000" 
                  value={$settings.monthlyLimitGB} 
                  disabled={!$settings.monthlyLimitEnabled}
                  onchange={(e) => updateSetting("monthlyLimitGB", parseFloat(e.target.value))}
                  class="num-input"
                />
                <label class="switch">
                  <input 
                    type="checkbox" 
                    checked={$settings.monthlyLimitEnabled} 
                    onchange={(e) => updateSetting("monthlyLimitEnabled", e.target.checked)} 
                  />
                  <span class="slider"></span>
                </label>
              </div>
            </div>
          {/if}
        </section>

      {:else if activeTab === 'data'}
        <section class="section">
          <h2>Storage Management</h2>
          
          {#if loadingDbInfo}
            <div class="db-loader">
              <div class="spinner"></div>
              <span>Fetching database telemetry diagnostics...</span>
            </div>
          {:else if dbInfo}
            <div class="storage-card">
              <div class="storage-stat">
                <span class="stat-label">Database File Size</span>
                <span class="stat-val">{formatBytes(dbInfo.total_size_bytes)}</span>
              </div>
              <div class="storage-breakdown">
                <div class="breakdown-item">
                  <span>Raw Telemetry Rows</span>
                  <strong>{dbInfo.raw_rows.toLocaleString()}</strong>
                </div>
                <div class="breakdown-item">
                  <span>Hourly Rollup Rows</span>
                  <strong>{dbInfo.hourly_rows.toLocaleString()}</strong>
                </div>
                <div class="breakdown-item">
                  <span>Daily Rollup Rows</span>
                  <strong>{dbInfo.daily_rows.toLocaleString()}</strong>
                </div>
              </div>
            </div>

            <section class="section-sub" style="margin-top: 16px;">
              <h3>Retention Policies</h3>
              
              <div class="setting-item">
                <div class="setting-info">
                  <label for="rawDays">Purge Raw Telemetry ({rawRetention} days)</label>
                  <span>Store high-frequency 5-minute telemetry intervals before rollup</span>
                </div>
                <input 
                  id="rawDays"
                  type="range" 
                  min="1" 
                  max="30" 
                  value={rawRetention}
                  oninput={(e) => rawRetention = parseInt(e.target.value)}
                  onchange={saveRetentionPolicy}
                  class="range-input"
                />
              </div>

              <div class="setting-item">
                <div class="setting-info">
                  <label for="hourlyDays">Purge Hourly Rollups ({hourlyRetention} days)</label>
                  <span>Store medium-frequency hourly aggregated usage statistics</span>
                </div>
                <input 
                  id="hourlyDays"
                  type="range" 
                  min="7" 
                  max="365" 
                  value={hourlyRetention}
                  oninput={(e) => hourlyRetention = parseInt(e.target.value)}
                  onchange={saveRetentionPolicy}
                  class="range-input"
                />
              </div>
            </section>

            <section class="section-sub" style="margin-top: 16px;">
              <h3>Database Maintenance</h3>
              
              <div class="setting-item">
                <div class="setting-info">
                  <label>Compact Database (Vacuum)</label>
                  <span>Reclaim unused database file storage pages and optimize reads</span>
                </div>
                <button class="action-btn-styled" onclick={handleVacuumDb}>Optimize Now</button>
              </div>

              <div class="setting-item">
                <div class="setting-info">
                  <label>Clear Telemetry History</label>
                  <span>Reset all usage records and start fresh</span>
                </div>
                <button class="danger-btn" onclick={handleClearDb}>Clear History Data</button>
              </div>
            </section>

            {#if clearSuccess}
              <div class="success-alert">Database stats cleared successfully!</div>
            {/if}
            {#if vacuumSuccess}
              <div class="success-alert">Database compaction and optimization completed!</div>
            {/if}
          {/if}
        </section>
      {:else if activeTab === 'support'}
        <section class="section support-section">
          <h2>Support the Developer</h2>
          <p class="support-intro">
            Internet Speed Meter is 100% free and open-source. If the app helps you monitor your bandwidth or customize your desktop, consider supporting its development!
          </p>

          <div class="donation-options">
            <button class="donation-card kofi" onclick={() => visitDonationLink('https://ko-fi.com/dekki')}>
              <div class="donation-icon">☕</div>
              <div class="donation-text">
                <h3>Support on Ko-fi</h3>
                <span>Support development at ko-fi.com/dekki</span>
              </div>
              <span class="donation-arrow">→</span>
            </button>
          </div>

          <div class="app-version-info">
            <span class="version-label">Internet Speed Meter</span>
            <span class="version-number">Version 1.2.0</span>
            <span class="version-copy">Made with care for everyone</span>
          </div>
        </section>
      {/if}
    </div>
  </div>
</main>

{#if showEtwModal}
  <div class="modal-backdrop" onclick={(e) => { if (e.target === e.currentTarget) showEtwModal = false; }}>
    <div class="speedtest-modal" style="max-width: 440px; padding: 24px; text-align: center;">
      <h3 style="margin-top: 0; font-size: 16px; font-weight: 700; color: var(--text-primary);">Administrator Rights Required</h3>
      <p style="font-size: 13px; color: var(--text-secondary); line-height: 1.5; margin: 12px 0 20px 0;">
        Enabling ETW Kernel Tracing requires Administrator privileges to monitor system network events. Would you like to restart Internet Speed Meter as Administrator now?
      </p>
      <div style="display: flex; gap: 10px; justify-content: center;">
        <button class="action-btn" onclick={() => showEtwModal = false}>Keep Current (Non-Admin)</button>
        <button class="action-btn run-test-btn" onclick={confirmRestartAsAdmin}>Restart as Admin</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .debug-metric-card {
    background: var(--widget-hover-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .debug-label {
    font-size: 10px;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .debug-value {
    font-size: 12px;
    color: var(--text-primary);
  }

  .subtabs-bar {
    display: flex;
    gap: 8px;
    margin-bottom: 16px;
    border-bottom: 1px solid var(--border-color);
    padding-bottom: 10px;
  }

  .subtab-btn {
    background: none;
    border: none;
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 600;
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .subtab-btn:hover {
    color: var(--text-primary);
    background: var(--btn-bg);
  }

  .subtab-btn.active {
    color: var(--accent-emerald);
    background: rgba(16, 185, 129, 0.12);
  }

  .engine-explanation-box {
    margin-top: 16px;
    padding: 16px;
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    border-radius: 10px;
  }

  .engine-explanation-box h4 {
    margin: 0 0 14px 0;
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .explanation-item {
    margin-bottom: 12px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border-color);
  }

  .explanation-item:last-child {
    margin-bottom: 0;
    padding-bottom: 0;
    border-bottom: none;
  }

  .explanation-title {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }

  .explanation-title strong {
    font-size: 12px;
    color: var(--text-primary);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
  }

  .dot.legacy { background: #94a3b8; }
  .dot.standard { background: #38bdf8; }
  .dot.admin { background: #f59e0b; }

  .explanation-item p {
    font-size: 11px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin: 0;
    padding-left: 16px;
  }

  .explanation-item code {
    background: rgba(148, 163, 184, 0.15);
    padding: 1px 5px;
    border-radius: 4px;
    font-family: monospace;
    font-size: 10.5px;
  }

  .engine-cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 10px;
    margin-top: 4px;
  }

  .engine-card {
    background: var(--widget-hover-bg, var(--card-bg));
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 12px 14px;
    text-align: left;
    cursor: pointer;
    transition: all 0.2s ease;
    color: var(--text-primary);
  }

  .engine-card:hover {
    border-color: var(--accent-emerald);
    transform: translateY(-1px);
  }

  .engine-card.active {
    border-color: var(--accent-emerald);
    background: rgba(16, 185, 129, 0.08);
    box-shadow: 0 0 12px rgba(16, 185, 129, 0.15);
  }

  .engine-card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 6px;
  }

  .engine-card-header strong {
    font-size: 13px;
    font-weight: 700;
  }

  .engine-card p {
    font-size: 11px;
    color: var(--text-secondary);
    line-height: 1.4;
    margin: 0;
  }

  .badge {
    font-size: 10px;
    font-weight: 700;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(148, 163, 184, 0.2);
    color: var(--text-secondary);
  }

  .badge.default {
    background: rgba(56, 189, 248, 0.2);
    color: #38bdf8;
  }

  .badge.admin {
    background: rgba(245, 158, 11, 0.2);
    color: #f59e0b;
  }

  :global(html) {
    /* DEFAULT/LIGHT THEME VARIABLES */
    --bg-color: #f1f5f9;
    --text-primary: #0f172a;
    --text-secondary: #475569;
    --border-color: rgba(15, 23, 42, 0.08);
    --card-bg: #f8fafc;
    --card-border: rgba(15, 23, 42, 0.06);
    --sidebar-border: rgba(15, 23, 42, 0.08);
    --btn-bg: rgba(15, 23, 42, 0.04);
    --btn-border: rgba(15, 23, 42, 0.08);
    --btn-hover: rgba(15, 23, 42, 0.08);
    --input-bg: #f8fafc;
    --input-border: rgba(15, 23, 42, 0.15);
    --input-focus: #059669;
    --switch-bg: rgba(15, 23, 42, 0.1);
    --switch-thumb: #475569;
    --accent-emerald: #059669;
  }

  @media (prefers-color-scheme: dark) {
    :global(html) {
      /* SYSTEM DEFAULT DARK THEME VARIABLES */
      --bg-color: #0c0d12;
      --text-primary: #e2e8f0;
      --text-secondary: #94a3b8;
      --border-color: rgba(255, 255, 255, 0.08);
      --card-bg: rgba(255, 255, 255, 0.03);
      --card-border: rgba(255, 255, 255, 0.05);
      --sidebar-border: rgba(255, 255, 255, 0.08);
      --btn-bg: rgba(255, 255, 255, 0.06);
      --btn-border: rgba(255, 255, 255, 0.1);
      --btn-hover: rgba(255, 255, 255, 0.12);
      --input-bg: #171821;
      --input-border: rgba(255, 255, 255, 0.1);
      --input-focus: #10b981;
      --switch-bg: rgba(255, 255, 255, 0.1);
      --switch-thumb: #c5c6c7;
      --accent-emerald: #10b981;
    }
  }

  :global(html[data-theme="light"]) {
    /* EXPLICIT LIGHT THEME OVERRIDES */
    --bg-color: #f1f5f9;
    --text-primary: #0f172a;
    --text-secondary: #475569;
    --border-color: rgba(15, 23, 42, 0.08);
    --card-bg: #f8fafc;
    --card-border: rgba(15, 23, 42, 0.06);
    --sidebar-border: rgba(15, 23, 42, 0.08);
    --btn-bg: rgba(15, 23, 42, 0.04);
    --btn-border: rgba(15, 23, 42, 0.08);
    --btn-hover: rgba(15, 23, 42, 0.08);
    --input-bg: #f8fafc;
    --input-border: rgba(15, 23, 42, 0.15);
    --input-focus: #059669;
    --switch-bg: rgba(15, 23, 42, 0.1);
    --switch-thumb: #475569;
    --accent-emerald: #059669;
  }

  :global(html[data-theme="dark"]) {
    /* EXPLICIT DARK THEME OVERRIDES */
    --bg-color: #0c0d12;
    --text-primary: #e2e8f0;
    --text-secondary: #94a3b8;
    --border-color: rgba(255, 255, 255, 0.08);
    --card-bg: rgba(255, 255, 255, 0.03);
    --card-border: rgba(255, 255, 255, 0.05);
    --sidebar-border: rgba(255, 255, 255, 0.08);
    --btn-bg: rgba(255, 255, 255, 0.06);
    --btn-border: rgba(255, 255, 255, 0.1);
    --btn-hover: rgba(255, 255, 255, 0.12);
    --input-bg: #171821;
    --input-border: rgba(255, 255, 255, 0.1);
    --input-focus: #10b981;
    --switch-bg: rgba(255, 255, 255, 0.1);
    --switch-thumb: #c5c6c7;
    --accent-emerald: #10b981;
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

  .settings-panel {
    display: flex;
    flex-direction: column;
    height: 100vh;
    box-sizing: border-box;
    padding: 20px;
    background-color: var(--bg-color);
  }

  .header {
    margin-bottom: 16px;
    border-bottom: 1px solid var(--border-color);
    padding-bottom: 10px;
  }

  .header h1 {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
  }

  .header p {
    font-size: 11px;
    color: var(--text-secondary);
    margin: 4px 0 0 0;
  }

  .content-wrapper {
    display: flex;
    flex: 1;
    gap: 16px;
    min-height: 0;
    overflow: hidden;
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    width: 130px;
    gap: 4px;
    border-right: 1px solid var(--sidebar-border);
    padding-right: 12px;
    flex-shrink: 0;
  }

  .nav-btn {
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }

  .nav-btn:hover {
    color: var(--text-primary);
    background: var(--btn-bg);
  }

  .nav-btn.active {
    color: var(--accent-emerald);
    background: var(--chart-down-fill, rgba(16, 185, 129, 0.12));
  }

  .main-settings {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding-right: 8px;
  }

  .section h2 {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0 0 16px 0;
  }

  .sub-tab-bar {
    display: flex;
    gap: 8px;
    margin-bottom: 16px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--border-color);
  }

  .sub-tab-btn {
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    color: var(--text-secondary);
    padding: 6px 14px;
    border-radius: 8px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .sub-tab-btn:hover {
    background: var(--btn-bg);
    color: var(--text-primary);
  }

  .sub-tab-btn.active {
    background: var(--accent-emerald);
    color: #ffffff;
    border-color: var(--accent-emerald);
    box-shadow: 0 2px 8px rgba(16, 185, 129, 0.25);
  }

  .setting-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 0;
    border-bottom: 1px solid var(--border-color);
    gap: 16px;
  }

  .setting-info {
    display: flex;
    flex-direction: column;
    flex: 1;
  }

  .setting-info label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .setting-info span {
    font-size: 10px;
    color: var(--text-secondary);
    margin-top: 2px;
    line-height: 1.3;
  }

  /* Dropdown styling */
  .select-input {
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    color: var(--text-primary);
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 11px;
    font-family: inherit;
    outline: none;
    cursor: pointer;
    transition: border-color 0.2s;
  }

  .select-input:focus {
    border-color: var(--input-focus);
  }

  /* Slider Range style */
  .range-input {
    -webkit-appearance: none;
    width: 110px;
    background: var(--btn-border);
    height: 4px;
    border-radius: 2px;
    outline: none;
  }

  .range-input::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent-emerald);
    cursor: pointer;
  }

  /* Toggle Switch styles */
  .switch {
    position: relative;
    display: inline-block;
    width: 32px;
    height: 18px;
    flex-shrink: 0;
  }

  .switch input {
    opacity: 0;
    width: 0;
    height: 0;
  }

  .slider {
    position: absolute;
    cursor: pointer;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: var(--switch-bg);
    transition: .3s;
    border-radius: 9px;
  }

  .slider:before {
    position: absolute;
    content: "";
    height: 14px;
    width: 14px;
    left: 2px;
    bottom: 2px;
    background-color: var(--switch-thumb);
    transition: .3s;
    border-radius: 50%;
  }

  input:checked + .slider {
    background-color: var(--accent-emerald);
  }

  input:checked + .slider:before {
    transform: translateX(14px);
    background-color: #ffffff;
  }

  .danger-btn {
    background: #ef4444;
    color: #ffffff;
    border: none;
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.2s;
  }

  .danger-btn:hover {
    background: #dc2626;
  }

  .success-alert {
    background: rgba(16, 185, 129, 0.1);
    border: 1px solid rgba(16, 185, 129, 0.2);
    color: var(--accent-emerald);
    font-size: 11px;
    padding: 8px 12px;
    border-radius: 6px;
    margin-top: 12px;
    text-align: center;
  }

  /* Accent Swatches */
  .accent-picker {
    display: flex;
    gap: 8px;
  }

  .accent-swatch {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    transition: transform 0.15s, border-color 0.15s;
    box-shadow: 0 2px 4px rgba(0,0,0,0.12);
  }

  .accent-swatch:hover {
    transform: scale(1.15);
  }

  .accent-swatch.active {
    border-color: var(--text-primary);
  }

  /* Hotkey recorder button */
  .hotkey-btn {
    background: var(--btn-bg);
    border: 1px solid var(--btn-border);
    color: var(--text-primary);
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    min-width: 120px;
    transition: all 0.2s;
  }

  .hotkey-btn:hover {
    background: var(--btn-hover);
  }

  .hotkey-btn.recording {
    background: rgba(239, 68, 68, 0.1);
    border-color: #ef4444;
    color: #ef4444;
    animation: pulse 1.5s infinite;
  }

  @keyframes pulse {
    0% { opacity: 0.6; }
    50% { opacity: 1.0; }
    100% { opacity: 0.6; }
  }

  /* Number inputs */
  .num-input {
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    color: var(--text-primary);
    padding: 4px 8px;
    border-radius: 6px;
    font-size: 11px;
    width: 60px;
    outline: none;
    text-align: center;
  }

  .num-input:focus {
    border-color: var(--input-focus);
  }

  .num-input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Storage Diagnostics View */
  .storage-card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: 8px;
    padding: 14px;
    margin-top: 8px;
    box-shadow: 0 4px 12px rgba(0,0,0,0.02);
  }

  .storage-stat {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid var(--border-color);
    padding-bottom: 8px;
    margin-bottom: 10px;
  }

  .stat-label {
    font-size: 11px;
    color: var(--text-secondary);
    text-transform: uppercase;
    font-weight: 600;
  }

  .stat-val {
    font-size: 15px;
    font-weight: 700;
    color: var(--accent-emerald);
  }

  .storage-breakdown {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .breakdown-item {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--text-secondary);
  }

  .breakdown-item strong {
    color: var(--text-primary);
  }

  .section-sub h3 {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 12px 0 6px 0;
  }

  .db-loader {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 40px 0;
    gap: 12px;
    color: var(--text-secondary);
    font-size: 11px;
  }

  .action-btn-styled {
    background: var(--btn-bg);
    border: 1px solid var(--btn-border);
    color: var(--text-primary);
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }

  .action-btn-styled:hover {
    background: var(--btn-hover);
  }

  /* Support Us tab styling */
  .support-intro {
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-secondary);
    margin-bottom: 20px;
  }

  .donation-options {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-bottom: 24px;
  }

  .donation-card {
    display: flex;
    align-items: center;
    width: 100%;
    padding: 12px 16px;
    border-radius: 8px;
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    text-align: left;
    cursor: pointer;
    transition: transform 0.2s, border-color 0.2s, background 0.2s;
    box-sizing: border-box;
  }

  .donation-card:hover {
    transform: translateY(-2px);
    background: var(--btn-hover);
  }

  .donation-card.kofi:hover {
    border-color: #ff5e5b;
  }

  .donation-card.github:hover {
    border-color: #ff5a79;
  }

  .donation-icon {
    font-size: 24px;
    margin-right: 16px;
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .donation-text {
    flex-grow: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .donation-text h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .donation-text span {
    font-size: 11px;
    color: var(--text-secondary);
  }

  .donation-arrow {
    font-size: 14px;
    color: var(--text-secondary);
    transition: transform 0.2s;
  }

  .donation-card:hover .donation-arrow {
    transform: translateX(4px);
    color: var(--text-primary);
  }

  .app-version-info {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    border-top: 1px solid var(--border-color);
    padding-top: 20px;
    gap: 4px;
    color: var(--text-secondary);
  }

  .version-label {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .version-number {
    font-size: 10px;
    font-weight: 500;
    background: var(--btn-bg);
    padding: 2px 8px;
    border-radius: 20px;
    border: 1px solid var(--btn-border);
  }

  .version-copy {
    font-size: 10px;
    color: var(--text-secondary);
  }
</style>

