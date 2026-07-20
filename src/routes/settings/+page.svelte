<script>
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { settings } from "../../lib/settingsStore";

  let activeTab = $state("general");
  let autostartEnabled = $state(false);
  let clearSuccess = $state(false);

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
      // Revert UI toggle on error
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

  async function handleClearDb() {
    if (confirm("Are you sure you want to clear all historical network and screen time usage data?")) {
      try {
        await invoke("get_historical_stats", { period: "clear" });
        clearSuccess = true;
        setTimeout(() => clearSuccess = false, 3000);
      } catch (err) {
        console.error("Failed to clear database", err);
      }
    }
  }
</script>

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
        </section>

      {:else if activeTab === 'appearance'}
        <section class="section">
          <h2>Appearance & Theme</h2>

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
        </section>

      {:else if activeTab === 'data'}
        <section class="section">
          <h2>Storage Management</h2>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>Database Diagnostics</label>
              <span>Clear local statistics logs or purge cache files</span>
            </div>
            <button class="danger-btn" onclick={handleClearDb}>Clear History Data</button>
          </div>

          {#if clearSuccess}
            <div class="success-alert">
              Database stats cleared successfully!
            </div>
          {/if}
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
    background: rgba(16, 185, 129, 0.08);
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
</style>
