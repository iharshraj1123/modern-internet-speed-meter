<script>
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { settings, ACCENT_COLORS } from "../../lib/settingsStore";

  let activeTab = $state("general");
  let autostartEnabled = $state(false);
  let clearSuccess = $state(false);

  // Storage Analytics states
  let dbInfo = $state(null);
  let loadingDbInfo = $state(false);
  let vacuumSuccess = $state(false);
  let rawRetention = $state(7);
  let hourlyRetention = $state(90);

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

  onMount(async () => {
    try {
      autostartEnabled = await invoke("plugin:autostart|is_enabled");
    } catch (e) {
      console.error("Autostart query failed", e);
    }
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
        ❤️ Support Us
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
              <label for="themeSelect">Application Theme</label>
              <span>Choose between System, Dark, or Light interface themes</span>
            </div>
            <select 
              id="themeSelect" 
              class="select-input"
              value={$settings.theme || 'system'} 
              onchange={(e) => updateSetting("theme", e.target.value)}
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
              <label for="graphType">Graph Representation</label>
              <span>Select the real-time speed chart overlay layout</span>
            </div>
            <select 
              id="graphType" 
              class="select-input"
              value={$settings.graphType} 
              onchange={(e) => updateSetting("graphType", e.target.value)}
            >
              <option value="combined">Combined Graph (Green overlapping glow)</option>
              <option value="separate">Separate Graphs (Stacked Upload/Download)</option>
              <option value="hidden">Hidden (Metrics text only, saves CPU)</option>
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
        </section>

      {:else if activeTab === 'telemetry'}
        <section class="section">
          <h2>Performance & Logic</h2>

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
            <span class="version-number">Version 1.0.0</span>
            <span class="version-copy">Made with ❤️ for everyone</span>
          </div>
        </section>
      {/if}
    </div>
  </div>
</main>

<style>
  :global(html) {
    /* DEFAULT/LIGHT THEME VARIABLES */
    --bg-color: #f8fafc;
    --text-primary: #0f172a;
    --text-secondary: #475569;
    --border-color: rgba(15, 23, 42, 0.08);
    --card-bg: #ffffff;
    --card-border: rgba(15, 23, 42, 0.06);
    --sidebar-border: rgba(15, 23, 42, 0.08);
    --btn-bg: rgba(15, 23, 42, 0.04);
    --btn-border: rgba(15, 23, 42, 0.08);
    --btn-hover: rgba(15, 23, 42, 0.08);
    --input-bg: #ffffff;
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
      --text-primary: #ffffff;
      --text-secondary: #9ca3af;
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
    --bg-color: #f8fafc;
    --text-primary: #0f172a;
    --text-secondary: #475569;
    --border-color: rgba(15, 23, 42, 0.08);
    --card-bg: #ffffff;
    --card-border: rgba(15, 23, 42, 0.06);
    --sidebar-border: rgba(15, 23, 42, 0.08);
    --btn-bg: rgba(15, 23, 42, 0.04);
    --btn-border: rgba(15, 23, 42, 0.08);
    --btn-hover: rgba(15, 23, 42, 0.08);
    --input-bg: #ffffff;
    --input-border: rgba(15, 23, 42, 0.15);
    --input-focus: #059669;
    --switch-bg: rgba(15, 23, 42, 0.1);
    --switch-thumb: #475569;
    --accent-emerald: #059669;
  }

  :global(html[data-theme="dark"]) {
    /* EXPLICIT DARK THEME OVERRIDES */
    --bg-color: #0c0d12;
    --text-primary: #ffffff;
    --text-secondary: #9ca3af;
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
    margin: 0 0 12px 0;
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

