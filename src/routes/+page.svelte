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

  onMount(async () => {
    // Disable right-click context menu
    document.addEventListener("contextmenu", (e) => e.preventDefault());

    // 1. Fetch initial stats
    try {
      const initial = await invoke("get_realtime_stats");
      downloadSpeed = initial.download_speed;
      uploadSpeed = initial.upload_speed;
      batteryPct = initial.battery_percentage;
      isCharging = initial.is_charging;
      activeApp = initial.active_app;
    } catch (e) {
      console.error("Failed to query initial stats", e);
    }

    // 2. Setup real-time listener from Rust backend
    unlistenStats = await listen("realtime-stats", (event) => {
      const data = event.payload;
      downloadSpeed = data.download_speed;
      uploadSpeed = data.upload_speed;
      batteryPct = data.battery_percentage;
      isCharging = data.is_charging;
      activeApp = data.active_app;

      // Update graphs history
      downloadHistory = [...downloadHistory.slice(1), downloadSpeed];
      uploadHistory = [...uploadHistory.slice(1), uploadSpeed];
    });

    // Sync initial settings to backend
    settings.syncWithBackend($settings);
  });

  onDestroy(() => {
    if (unlistenStats) unlistenStats();
  });

  // Calculate scaling for graphs
  let maxDownload = $derived(Math.max(...downloadHistory, 1024));
  let maxUpload = $derived(Math.max(...uploadHistory, 1024));
  let maxCombined = $derived(Math.max(maxDownload, maxUpload));

  // Graph paths
  let downPath = $derived(getPathData(downloadHistory, 24, maxDownload));
  let downAreaPath = $derived(getAreaPathData(downloadHistory, 24, maxDownload));
  let upPath = $derived(getPathData(uploadHistory, 24, maxUpload));
  let upAreaPath = $derived(getAreaPathData(uploadHistory, 24, maxUpload));

  let combinedDownAreaPath = $derived(getAreaPathData(downloadHistory, 28, maxCombined));
  let combinedDownPath = $derived(getPathData(downloadHistory, 28, maxCombined));
  let combinedUpAreaPath = $derived(getAreaPathData(uploadHistory, 28, maxCombined));
  let combinedUpPath = $derived(getPathData(uploadHistory, 28, maxCombined));

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
      <div class="battery" title={isCharging ? "Charging" : "Discharging"}>
        <span class="bat-icon">{isCharging ? "⚡" : "🔋"}</span>
        <span class="bat-val">{batteryPct}%</span>
      </div>
    </div>

    <!-- Chart rendering -->
    {#if $settings.graphType === 'combined'}
      <div class="chart-container combined">
        <svg viewBox="0 0 230 28" class="chart-svg" preserveAspectRatio="none">
          <path d={combinedDownAreaPath} class="chart-area down-area" />
          <path d={combinedUpAreaPath} class="chart-area up-area" />
          <path d={combinedDownPath} class="chart-line down-line" />
          <path d={combinedUpPath} class="chart-line up-line" />
        </svg>
      </div>
    {:else if $settings.graphType === 'separate'}
      <div class="chart-container split">
        <svg viewBox="0 0 230 12" class="chart-svg" preserveAspectRatio="none">
          <path d={downAreaPath} class="chart-area down-area" />
          <path d={downPath} class="chart-line down-line" />
        </svg>
        <svg viewBox="0 0 230 12" class="chart-svg" preserveAspectRatio="none">
          <path d={upAreaPath} class="chart-area up-area" />
          <path d={upPath} class="chart-line up-line" />
        </svg>
      </div>
    {/if}

    <!--
      Resize zones — placed AFTER charts so they sit on top in the stacking order.
      They all call stopPropagation() so the widget's onmousedown (startDrag) never fires.
      z-index ensures they intercept clicks before the widget background does.
    -->

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
      <div class="battery">
        <span class="bat-icon">{isCharging ? "⚡" : "🔋"}</span>
        <span class="bat-val">{batteryPct}%</span>
      </div>
    </div>

    <!-- Chart rendering -->
    {#if $settings.graphType === 'combined'}
      <div class="chart-container combined">
        <svg viewBox="0 0 230 28" class="chart-svg" preserveAspectRatio="none">
          <!-- Download Area fill (green) -->
          <path d={combinedDownAreaPath} class="chart-area down-area" />
          <!-- Upload Area fill (blue) -->
          <path d={combinedUpAreaPath} class="chart-area up-area" />
          <!-- Download stroke line (green dotted) -->
          <path d={combinedDownPath} class="chart-line down-line" />
          <!-- Upload stroke line (blue dotted) -->
          <path d={combinedUpPath} class="chart-line up-line" />
        </svg>
      </div>
    {:else if $settings.graphType === 'separate'}
      <div class="chart-container split">
        <svg viewBox="0 0 230 12" class="chart-svg" preserveAspectRatio="none">
          <path d={downAreaPath} class="chart-area down-area" />
        </svg>
        <svg viewBox="0 0 230 12" class="chart-svg" preserveAspectRatio="none">
          <path d={upAreaPath} class="chart-area up-area" />
        </svg>
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

  .battery {
    display: flex;
    align-items: center;
    gap: 2px;
    background: var(--battery-bg);
    padding: 2px 5px;
    border-radius: 5px;
    color: var(--text-sec);
    font-size: 9px;
  }

  .chart-container {
    width: 100%;
    margin-top: 4px;
    display: flex;
    overflow: hidden;
    flex: 1;
    min-height: 16px;
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
    transition: d 0.1s ease;
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
    transition: d 0.1s ease;
    pointer-events: none;
  }

  .down-line {
    stroke: var(--metric-down);
    stroke-dasharray: 2 2;
  }

  .up-line {
    stroke: var(--chart-up-stroke);
    stroke-dasharray: 2 2;
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
</style>
