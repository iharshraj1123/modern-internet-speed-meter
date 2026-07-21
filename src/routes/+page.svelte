<script>
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { settings, formatSpeed } from "../lib/settingsStore";

  // State cache for speeds
  let downloadSpeed = $state(0);
  let uploadSpeed = $state(0);
  let batteryPct = $state(100);
  let isCharging = $state(true);
  let activeApp = $state("System");
  let pingMs = $state(0);
  let todayTotalBytes = $state(0);
  let ticks = 0;

  // JS-driven window drag — works the same as data-tauri-drag-region but
  // respects JS event propagation, so resize handles can stopPropagation to block it.
  async function startDrag(event) {
    if (event.button !== 0) return;       // left click only
    if (event.detail >= 2) return;        // skip on double-click — let ondblclick fire
    event.preventDefault();
    try {
      await getCurrentWindow().startDragging();
    } catch (err) {
      console.error("Failed to start window drag", err);
    }
  }

  // Native resize trigger — stopPropagation so startDrag on parent never fires.
  async function startResize(direction, event) {
    event.preventDefault();
    event.stopPropagation();
    try {
      const appWindow = getCurrentWindow();
      await appWindow.startResizeDragging(direction);
    } catch (err) {
      console.error("Failed to start native window resize", err);
    }
  }

  // Historical speed readings for rendering graphs (max 30 points)
  let downloadHistory = $state(Array(30).fill(0));
  let uploadHistory = $state(Array(30).fill(0));

  let unlistenStats;

  // Compute SVG path string from history
  function getPathData(history, height, maxVal) {
    if (maxVal === 0) maxVal = 1;
    const points = history.map((val, idx) => {
      const x = (idx / (history.length - 1)) * 230; // SVG width is ~230
      const y = height - (val / maxVal) * (height - 4);
      return `${x},${y}`;
    });
    return `M ${points.join(" L ")}`;
  }

  // Compute SVG closed path for fill area
  function getAreaPathData(history, height, maxVal) {
    if (maxVal === 0) maxVal = 1;
    const points = history.map((val, idx) => {
      const x = (idx / (history.length - 1)) * 230;
      const y = height - (val / maxVal) * (height - 4);
      return `${x},${y}`;
    });
    return `M 0,${height} L ${points.join(" L ")} L 230,${height} Z`;
  }

  // Curated color themes mapping
  const ACCENT_COLORS = {
    emerald: { light: "#059669", dark: "#10b981" },
    violet: { light: "#7c3aed", dark: "#8b5cf6" },
    sky: { light: "#0284c7", dark: "#38bdf8" },
    amber: { light: "#d97706", dark: "#f59e0b" },
    rose: { light: "#e11d48", dark: "#f43f5e" },
    coral: { light: "#ea580c", dark: "#f97316" }
  };

  $effect(() => {
    const accent = $settings.accentColor || "emerald";
    const colors = ACCENT_COLORS[accent] || ACCENT_COLORS.emerald;
    const op = $settings.opacity ?? 0.85;

    // Dynamically apply accent color to CSS variables on HTML element
    document.documentElement.style.setProperty('--metric-down', colors.dark);
    document.documentElement.style.setProperty('--accent-emerald', colors.dark);
    document.documentElement.style.setProperty('--widget-hover-border', `${colors.dark}55`);
    document.documentElement.style.setProperty('--chart-down-fill', `${colors.dark}22`);

    // Solid background when opacity is set to 1.0 (or scaled linearly)
    if ($settings.theme === 'light') {
      document.documentElement.style.setProperty('--widget-bg', `rgba(255, 255, 255, ${op})`);
      document.documentElement.style.setProperty('--widget-hover-bg', `rgba(255, 255, 255, ${Math.min(1.0, op + 0.05)})`);
    } else {
      document.documentElement.style.setProperty('--widget-bg', `rgba(10, 10, 12, ${op})`);
      document.documentElement.style.setProperty('--widget-hover-bg', `rgba(15, 15, 18, ${Math.min(1.0, op + 0.05)})`);
    }
  });

  // Calculate daily limit usage percentage
  let dailyUsagePct = $derived(
    $settings.dailyLimitEnabled && $settings.dailyLimitGB > 0
      ? (todayTotalBytes / ($settings.dailyLimitGB * 1024 * 1024 * 1024)) * 100
      : 0
  );

  function handleContextMenu(event) {
    event.preventDefault();
    event.stopPropagation();
    invoke("show_context_menu");
  }

  onMount(async () => {
    // Custom context menu listener
    document.addEventListener("contextmenu", handleContextMenu);

    // 1. Fetch initial stats
    try {
      const initial = await invoke("get_realtime_stats");
      downloadSpeed = initial.download_speed;
      uploadSpeed = initial.upload_speed;
      batteryPct = initial.battery_percentage;
      isCharging = initial.is_charging;
      activeApp = initial.active_app;
      pingMs = initial.ping_ms || 0;
    } catch (e) {
      console.error("Failed to query initial stats", e);
    }

    // 2. Fetch initial daily limit usage
    if ($settings.dailyLimitEnabled) {
      try {
        const res = await invoke("get_today_usage");
        if (Array.isArray(res)) {
          todayTotalBytes = res[0] + res[1];
        }
      } catch (err) {
        console.error("Failed to load initial today total", err);
      }
    }

    // 3. Register global hotkey
    if ($settings.globalHotkey) {
      try {
        await invoke("register_hotkey", { shortcut: $settings.globalHotkey });
      } catch (err) {
        console.error("Failed to register global hotkey", err);
      }
    }

    // 4. Setup real-time listener from Rust backend
    unlistenStats = await listen("realtime-stats", (event) => {
      const data = event.payload;
      downloadSpeed = data.download_speed;
      uploadSpeed = data.upload_speed;
      batteryPct = data.battery_percentage;
      isCharging = data.is_charging;
      activeApp = data.active_app;
      pingMs = data.ping_ms || 0;

      // Update graphs history
      downloadHistory = [...downloadHistory.slice(1), downloadSpeed];
      uploadHistory = [...uploadHistory.slice(1), uploadSpeed];

      // Run threshold checking and fetch database counts every 30 seconds
      ticks++;
      if (ticks % 30 === 1) {
        if ($settings.dailyLimitEnabled || $settings.monthlyLimitEnabled) {
          const dailyBytes = $settings.dailyLimitEnabled ? ($settings.dailyLimitGB * 1024 * 1024 * 1024) : 0;
          const monthlyBytes = $settings.monthlyLimitEnabled ? ($settings.monthlyLimitGB * 1024 * 1024 * 1024) : 0;
          invoke("check_data_limits", {
            dailyLimitBytes: dailyBytes,
            monthlyLimitBytes: monthlyBytes
          }).catch(console.error);
        }

        if ($settings.dailyLimitEnabled) {
          invoke("get_today_usage").then(res => {
            if (Array.isArray(res)) {
              todayTotalBytes = res[0] + res[1];
            }
          }).catch(console.error);
        }
      }
    });

    // Sync initial settings to backend
    settings.syncWithBackend($settings);
  });

  onDestroy(() => {
    document.removeEventListener("contextmenu", handleContextMenu);
    if (unlistenStats) unlistenStats();
  });

  // Calculate dynamic smooth peak scaling for graphs to prevent scale popping
  let smoothMaxDown = $state(1024);
  let smoothMaxUp = $state(1024);
  let smoothMaxCombined = $state(1024);

  $effect(() => {
    const rawDown = Math.max(...downloadHistory, 1024);
    const rawUp = Math.max(...uploadHistory, 1024);
    const rawComb = Math.max(rawDown, rawUp);

    smoothMaxDown = rawDown >= smoothMaxDown ? rawDown : Math.max(rawDown, smoothMaxDown * 0.95);
    smoothMaxUp = rawUp >= smoothMaxUp ? rawUp : Math.max(rawUp, smoothMaxUp * 0.95);
    smoothMaxCombined = rawComb >= smoothMaxCombined ? rawComb : Math.max(rawComb, smoothMaxCombined * 0.95);
  });

  // Graph paths
  let downPath = $derived(getPathData(downloadHistory, 24, smoothMaxDown));
  let downAreaPath = $derived(getAreaPathData(downloadHistory, 24, smoothMaxDown));
  let upPath = $derived(getPathData(uploadHistory, 24, smoothMaxUp));
  let upAreaPath = $derived(getAreaPathData(uploadHistory, 24, smoothMaxUp));

  let combinedDownAreaPath = $derived(getAreaPathData(downloadHistory, 28, smoothMaxCombined));
  let combinedDownPath = $derived(getPathData(downloadHistory, 28, smoothMaxCombined));
  let combinedUpAreaPath = $derived(getAreaPathData(uploadHistory, 28, smoothMaxCombined));
  let combinedUpPath = $derived(getPathData(uploadHistory, 28, smoothMaxCombined));

  function handleDoubleClick() {
    invoke("open_dashboard");
  }

  function handleClose() {
    invoke("toggle_click_through", { enabled: true }); // Prevent click interactions
  }
</script>

<!-- The widget has two modes: Draggable (if not locked) and Non-draggable (if locked) -->
{#if !$settings.locked}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div 
    class="widget" 
    style="opacity: {$settings.opacity};"
    ondblclick={handleDoubleClick}
    onmousedown={startDrag}
  >
    <div class="metrics">
      <div class="metric down">
        <span class="arrow">↓</span>
        <span class="value">{formatSpeed(downloadSpeed, $settings.unit)}</span>
      </div>
      <div class="metric up">
        <span class="arrow">↑</span>
        <span class="value">{formatSpeed(uploadSpeed, $settings.unit)}</span>
      </div>
      {#if $settings.showPing}
        <div class="ping-pill" title="Network Latency">
          <span class="ping-icon">🌐</span>
          <span class="ping-val">{pingMs > 0 ? `${pingMs}ms` : '--'}</span>
        </div>
      {/if}
    </div>

    <!-- Chart rendering -->
    {#if $settings.graphType === 'separate'}
      <div class="chart-container split">
        <div class="chart-svg-wrapper">
          <div class="widget-y-axis">
            <span class="widget-y-label top">{formatSpeed(smoothMaxDown, $settings.unit)}</span>
            <span class="widget-y-label bottom">0 {$settings?.unit === 'b' ? 'b/s' : 'B/s'}</span>
          </div>
          <svg viewBox="0 0 230 12" class="chart-svg" preserveAspectRatio="none">
            <line x1="0" y1="1" x2="230" y2="1" stroke="var(--widget-border)" stroke-dasharray="3 3" opacity="0.3" vector-effect="non-scaling-stroke" />
            <path d={downAreaPath} class="chart-area down-area" />
            <path d={downPath} class="chart-line down-line" class:solid-line={$settings.graphLineStyle === 'solid'} />
          </svg>
        </div>
        <div class="chart-svg-wrapper">
          <div class="widget-y-axis">
            <span class="widget-y-label top">{formatSpeed(smoothMaxUp, $settings.unit)}</span>
            <span class="widget-y-label bottom">0 {$settings?.unit === 'b' ? 'b/s' : 'B/s'}</span>
          </div>
          <svg viewBox="0 0 230 12" class="chart-svg" preserveAspectRatio="none">
            <line x1="0" y1="1" x2="230" y2="1" stroke="var(--widget-border)" stroke-dasharray="3 3" opacity="0.3" vector-effect="non-scaling-stroke" />
            <path d={upAreaPath} class="chart-area up-area" />
            <path d={upPath} class="chart-line up-line" class:solid-line={$settings.graphLineStyle === 'solid'} />
          </svg>
        </div>
      </div>
    {:else if $settings.graphType === 'hidden'}
      <!-- Hidden graph -->
    {:else}
      <!-- Combined Graph (Default) -->
      <div class="chart-container combined">
        <div class="widget-y-axis">
          <span class="widget-y-label top">{formatSpeed(smoothMaxCombined, $settings.unit)}</span>
          <span class="widget-y-label mid">{formatSpeed(smoothMaxCombined / 2, $settings.unit)}</span>
          <span class="widget-y-label bottom">0 {$settings?.unit === 'b' ? 'b/s' : 'B/s'}</span>
        </div>
        <svg viewBox="0 0 230 28" class="chart-svg" preserveAspectRatio="none">
          <line x1="0" y1="1" x2="230" y2="1" stroke="var(--widget-border)" stroke-dasharray="3 3" opacity="0.4" vector-effect="non-scaling-stroke" />
          <line x1="0" y1="14" x2="230" y2="14" stroke="var(--widget-border)" stroke-dasharray="3 3" opacity="0.25" vector-effect="non-scaling-stroke" />
          <line x1="0" y1="27" x2="230" y2="27" stroke="var(--widget-border)" opacity="0.4" vector-effect="non-scaling-stroke" />
          <path d={combinedDownAreaPath} class="chart-area down-area" />
          <path d={combinedUpAreaPath} class="chart-area up-area" />
          <path d={combinedDownPath} class="chart-line down-line" class:solid-line={$settings.graphLineStyle === 'solid'} />
          <path d={combinedUpPath} class="chart-line up-line" class:solid-line={$settings.graphLineStyle === 'solid'} />
        </svg>
      </div>
    {/if}

    {#if $settings.dailyLimitEnabled && dailyUsagePct > 0}
      <div class="quota-bar-container" title="Daily Quota: {dailyUsagePct.toFixed(1)}% Used">
        <div class="quota-bar-fill" style="width: {Math.min(dailyUsagePct, 100)}%;"></div>
      </div>
    {/if}

    <!-- Corners (36×36px, very easy to grab) -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize-corner se" onmousedown={(e) => startResize('SouthEast', e)}>
      <svg viewBox="0 0 10 10" width="10" height="10" class="corner-icon">
        <line x1="8" y1="2" x2="2" y2="8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        <line x1="8" y1="6" x2="6" y2="8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize-corner sw" onmousedown={(e) => startResize('SouthWest', e)}>
      <svg viewBox="0 0 10 10" width="10" height="10" class="corner-icon">
        <line x1="2" y1="2" x2="8" y2="8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        <line x1="2" y1="6" x2="4" y2="8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    </div>

    <!-- Edges (14px thick strips between corners) -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize-edge s"  onmousedown={(e) => startResize('South', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize-edge e"  onmousedown={(e) => startResize('East', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize-edge w"  onmousedown={(e) => startResize('West', e)}></div>
  </div>
{:else}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div 
    class="widget locked" 
    style="opacity: {$settings.opacity};"
    ondblclick={handleDoubleClick}
  >
    <div class="metrics">
      <div class="metric down">
        <span class="arrow">↓</span>
        <span class="value">{formatSpeed(downloadSpeed, $settings.unit)}</span>
      </div>
      <div class="metric up">
        <span class="arrow">↑</span>
        <span class="value">{formatSpeed(uploadSpeed, $settings.unit)}</span>
      </div>
      {#if $settings.showPing}
        <div class="ping-pill" title="Network Latency">
          <span class="ping-icon">🌐</span>
          <span class="ping-val">{pingMs > 0 ? `${pingMs}ms` : '--'}</span>
        </div>
      {/if}
    </div>

    <!-- Chart rendering -->
    {#if $settings.graphType === 'combined'}
      <div class="chart-container combined">
        <div class="widget-y-axis">
          <span class="widget-y-label top">{formatSpeed(smoothMaxCombined, $settings.unit)}</span>
          <span class="widget-y-label mid">{formatSpeed(smoothMaxCombined / 2, $settings.unit)}</span>
          <span class="widget-y-label bottom">0 {$settings?.unit === 'b' ? 'b/s' : 'B/s'}</span>
        </div>
        <svg viewBox="0 0 230 28" class="chart-svg" preserveAspectRatio="none">
          <line x1="0" y1="1" x2="230" y2="1" stroke="var(--widget-border)" stroke-dasharray="3 3" opacity="0.4" vector-effect="non-scaling-stroke" />
          <line x1="0" y1="14" x2="230" y2="14" stroke="var(--widget-border)" stroke-dasharray="3 3" opacity="0.25" vector-effect="non-scaling-stroke" />
          <line x1="0" y1="27" x2="230" y2="27" stroke="var(--widget-border)" opacity="0.4" vector-effect="non-scaling-stroke" />
          <path d={combinedDownAreaPath} class="chart-area down-area" />
          <path d={combinedUpAreaPath} class="chart-area up-area" />
          <path d={combinedDownPath} class="chart-line down-line" class:solid-line={$settings.graphLineStyle === 'solid'} />
          <path d={combinedUpPath} class="chart-line up-line" class:solid-line={$settings.graphLineStyle === 'solid'} />
        </svg>
      </div>
    {:else if $settings.graphType === 'separate'}
      <div class="chart-container split">
        <div class="chart-svg-wrapper">
          <div class="widget-y-axis">
            <span class="widget-y-label top">{formatSpeed(smoothMaxDown, $settings.unit)}</span>
            <span class="widget-y-label bottom">0 {$settings?.unit === 'b' ? 'b/s' : 'B/s'}</span>
          </div>
          <svg viewBox="0 0 230 12" class="chart-svg" preserveAspectRatio="none">
            <line x1="0" y1="1" x2="230" y2="1" stroke="var(--widget-border)" stroke-dasharray="3 3" opacity="0.3" vector-effect="non-scaling-stroke" />
            <path d={downAreaPath} class="chart-area down-area" />
            <path d={downPath} class="chart-line down-line" class:solid-line={$settings.graphLineStyle === 'solid'} />
          </svg>
        </div>
        <div class="chart-svg-wrapper">
          <div class="widget-y-axis">
            <span class="widget-y-label top">{formatSpeed(smoothMaxUp, $settings.unit)}</span>
            <span class="widget-y-label bottom">0 {$settings?.unit === 'b' ? 'b/s' : 'B/s'}</span>
          </div>
          <svg viewBox="0 0 230 12" class="chart-svg" preserveAspectRatio="none">
            <line x1="0" y1="1" x2="230" y2="1" stroke="var(--widget-border)" stroke-dasharray="3 3" opacity="0.3" vector-effect="non-scaling-stroke" />
            <path d={upAreaPath} class="chart-area up-area" />
            <path d={upPath} class="chart-line up-line" class:solid-line={$settings.graphLineStyle === 'solid'} />
          </svg>
        </div>
      </div>
    {/if}

    {#if $settings.dailyLimitEnabled && dailyUsagePct > 0}
      <div class="quota-bar-container" title="Daily Quota: {dailyUsagePct.toFixed(1)}% Used">
        <div class="quota-bar-fill" style="width: {Math.min(dailyUsagePct, 100)}%;"></div>
      </div>
    {/if}
  </div>
{/if}



<style>
  :global(html) {
    /* DEFAULT/LIGHT THEME VARIABLES FOR WIDGET */
    --widget-bg: rgba(255, 255, 255, 0.85);
    --widget-hover-bg: rgba(255, 255, 255, 0.92);
    --widget-border: rgba(15, 23, 42, 0.12);
    --widget-hover-border: rgba(5, 150, 105, 0.35);
    --text-color: #0f172a;
    --text-sec: #475569;
    --metric-down: #059669;
    --metric-up: #1d4ed8;
    --battery-bg: rgba(15, 23, 42, 0.06);
    --chart-down-fill: rgba(5, 150, 105, 0.12);
    --chart-up-fill: rgba(37, 99, 235, 0.08);
    --chart-up-stroke: #2563eb;
  }

  @media (prefers-color-scheme: dark) {
    :global(html) {
      /* SYSTEM DEFAULT DARK THEME VARIABLES FOR WIDGET */
      --widget-bg: rgba(10, 10, 12, 0.75);
      --widget-hover-bg: rgba(15, 15, 18, 0.82);
      --widget-border: rgba(255, 255, 255, 0.08);
      --widget-hover-border: rgba(16, 185, 129, 0.25);
      --text-color: #f3f4f6;
      --text-sec: #9ca3af;
      --metric-down: #10b981;    /* green = download */
      --metric-up: #60a5fa;      /* blue  = upload   */
      --battery-bg: rgba(255, 255, 255, 0.06);
      --chart-down-fill: rgba(16, 185, 129, 0.18);
      --chart-up-fill: rgba(96, 165, 250, 0.12);
      --chart-up-stroke: #60a5fa;
    }
  }

  :global(html[data-theme="light"]) {
    --widget-bg: rgba(255, 255, 255, 0.85);
    --widget-hover-bg: rgba(255, 255, 255, 0.92);
    --widget-border: rgba(15, 23, 42, 0.12);
    --widget-hover-border: rgba(5, 150, 105, 0.35);
    --text-color: #0f172a;
    --text-sec: #475569;
    --metric-down: #059669;
    --metric-up: #1d4ed8;
    --battery-bg: rgba(15, 23, 42, 0.06);
    --chart-down-fill: rgba(5, 150, 105, 0.12);
    --chart-up-fill: rgba(37, 99, 235, 0.08);
    --chart-up-stroke: #2563eb;
  }

  :global(html[data-theme="dark"]) {
    --widget-bg: rgba(10, 10, 12, 0.75);
    --widget-hover-bg: rgba(15, 15, 18, 0.82);
    --widget-border: rgba(255, 255, 255, 0.08);
    --widget-hover-border: rgba(16, 185, 129, 0.25);
    --text-color: #f3f4f6;
    --text-sec: #9ca3af;
    --metric-down: #10b981;    /* green = download */
    --metric-up: #60a5fa;      /* blue  = upload   */
    --battery-bg: rgba(255, 255, 255, 0.06);
    --chart-down-fill: rgba(16, 185, 129, 0.18);
    --chart-up-fill: rgba(96, 165, 250, 0.12);
    --chart-up-stroke: #60a5fa;
  }

  :global(html), :global(body) {
    margin: 0;
    padding: 0;
    overflow: hidden;
    background: transparent !important;
    background-color: transparent !important;
    font-family: 'Outfit', 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
    color: var(--text-color);
    user-select: none;
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    transition: color 0.3s;
  }

  .widget {
    position: relative;
    box-sizing: border-box;
    width: 100%;
    height: 100%;
    background: var(--widget-bg);
    border: 1px solid var(--widget-border);
    border-radius: 12px;
    backdrop-filter: blur(12px) saturate(180%);
    -webkit-backdrop-filter: blur(12px) saturate(180%);
    padding: 6px 10px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    box-shadow: 0 8px 32px 0 rgba(0, 0, 0, 0.2);
    transition: background 0.3s, border 0.3s, box-shadow 0.3s;
  }

  .widget:hover {
    background: var(--widget-hover-bg);
    border: 1px solid var(--widget-hover-border);
  }

  .metrics {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 20px;
    font-size: 11px;
    font-weight: 600;
  }

  .metric {
    display: flex;
    align-items: center;
    gap: 3px;
  }

  .metric.down {
    color: var(--metric-down);
    text-shadow: 0 0 8px rgba(16, 185, 129, 0.15);
  }

  .metric.up {
    color: var(--metric-up);
  }

  .arrow {
    font-size: 12px;
  }

  .value {
    letter-spacing: -0.2px;
  }

  .ping-pill {
    display: flex;
    align-items: center;
    gap: 3px;
    background: var(--battery-bg);
    padding: 2px 5px;
    border-radius: 5px;
    color: var(--text-sec);
    font-size: 9px;
  }

  @media (max-width: 189px) {
    .ping-pill {
      display: none !important;
    }
  }

  .quota-bar-container {
    position: absolute;
    bottom: 3px;
    left: 10px;
    right: 10px;
    height: 2px;
    background: rgba(128, 128, 128, 0.1);
    border-radius: 1px;
    overflow: hidden;
  }

  .quota-bar-fill {
    height: 100%;
    background: var(--metric-down);
    border-radius: 1px;
    transition: width 0.3s ease;
  }

  .chart-container {
    position: relative;
    width: 100%;
    margin-top: 4px;
    display: flex;
    overflow: hidden;
    flex: 1;
    min-height: 16px;
  }

  .chart-svg-wrapper {
    position: relative;
    width: 100%;
    height: 100%;
    flex: 1;
  }

  .widget-y-axis {
    position: absolute;
    top: 0;
    left: 2px;
    bottom: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    pointer-events: none;
    z-index: 5;
    font-size: 7px;
    font-weight: 600;
    color: var(--text-sec);
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
  }

  .widget-y-label {
    line-height: 1;
    letter-spacing: -0.2px;
  }

  .widget-y-label.top {
    transform: translateY(1px);
  }

  .widget-y-label.bottom {
    transform: translateY(-1px);
  }

  .chart-peak-tag {
    position: absolute;
    top: 0px;
    right: 2px;
    font-size: 7.5px;
    font-weight: 600;
    color: var(--text-sec);
    opacity: 0.55;
    pointer-events: none;
    letter-spacing: -0.2px;
    z-index: 5;
    font-variant-numeric: tabular-nums;
  }

  .chart-container.combined {
  }

  .chart-container.split {
    flex-direction: column;
    justify-content: space-between;
  }

  .chart-svg {
    width: 100%;
    height: 100%;
    overflow: visible;
    pointer-events: none;
  }

  /* SVG Graphics styles */
  .chart-area {
    stroke-width: 0;
    pointer-events: none;
  }

  .down-area {
    fill: var(--chart-down-fill);
  }

  .up-area {
    fill: var(--chart-up-fill);
  }

  .chart-line {
    fill: none;
    stroke-width: 1.5;
    pointer-events: none;
    vector-effect: non-scaling-stroke;
  }

  .down-line {
    stroke: var(--metric-down);
    stroke-dasharray: 2 2;
  }

  .up-line {
    stroke: var(--chart-up-stroke);
    stroke-dasharray: 2 2;
  }

  .chart-line.solid-line {
    stroke-dasharray: none !important;
  }

  /* ─────────────────────────────────────────────────────────
     Resize zones — all use JS stopPropagation so the widget's
     onmousedown (startDrag) never fires when resizing.
  ───────────────────────────────────────────────────────── */

  /* Corners: 36×36px — large, easy to grab */
  .resize-corner {
    position: absolute;
    width: 18px;
    height: 18px;
    z-index: 110;
    display: flex;
  }

  .corner-icon {
    opacity: 0.35;
    color: var(--text-sec);
    transition: opacity 0.2s, color 0.2s;
    pointer-events: none; /* icon is visual only, parent div handles clicks */
  }

  .resize-corner:hover .corner-icon {
    opacity: 1;
    color: var(--metric-down);
  }

  .resize-corner.se {
    right: 0;
    bottom: 0;
    cursor: se-resize;
    align-items: flex-end;
    justify-content: flex-end;
    padding: 4px;
    box-sizing: border-box;
  }

  .resize-corner.sw {
    left: 0;
    bottom: 0;
    cursor: sw-resize;
    align-items: flex-end;
    justify-content: flex-start;
    padding: 4px;
    box-sizing: border-box;
  }

  /* Edge strips: 14px thick, inset from corners */
  .resize-edge {
    position: absolute;
    z-index: 100;
  }

  /* Bottom edge: full width minus corner zones */
  .resize-edge.s {
    bottom: 0;
    left: 18px;
    right: 18px;
    height: 6px;
    cursor: s-resize;
  }

  /* Right edge: full height minus top bar and corner zones */
  .resize-edge.e {
    right: 0;
    top: 0;
    bottom: 18px;
    width: 6px;
    cursor: e-resize;
  }

  /* Left edge: full height minus top bar and corner zones */
  .resize-edge.w {
    left: 0;
    top: 0;
    bottom: 18px;
    width: 6px;
    cursor: w-resize;
  }

  /* Custom Right-Click Context Menu */
  .menu-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    z-index: 9998;
    background: transparent;
  }

  .custom-context-menu {
    position: fixed;
    z-index: 9999;
    width: 145px;
    background: rgba(15, 23, 42, 0.92);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 10px;
    padding: 4px;
    box-shadow: 0 10px 25px -5px rgba(0, 0, 0, 0.5), 0 8px 10px -6px rgba(0, 0, 0, 0.4);
    display: flex;
    flex-direction: column;
    gap: 2px;
    animation: menuFadeIn 0.15s cubic-bezier(0.16, 1, 0.3, 1);
  }

  :global(html[data-theme="light"]) .custom-context-menu {
    background: rgba(255, 255, 255, 0.94);
    border: 1px solid rgba(15, 23, 42, 0.12);
    box-shadow: 0 10px 25px -5px rgba(15, 23, 42, 0.15);
  }

  @keyframes menuFadeIn {
    from { opacity: 0; transform: scale(0.94); }
    to { opacity: 1; transform: scale(1); }
  }

  .menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--text-color);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    transition: background 0.15s, color 0.15s;
    box-sizing: border-box;
  }

  .menu-item:hover {
    background: var(--widget-hover-bg);
  }

  .menu-item.danger:hover {
    background: rgba(239, 68, 68, 0.2);
    color: #ef4444;
  }

  .menu-icon {
    font-size: 13px;
    line-height: 1;
  }

  .menu-divider {
    height: 1px;
    background: var(--widget-border);
    margin: 2px 4px;
  }
</style>
