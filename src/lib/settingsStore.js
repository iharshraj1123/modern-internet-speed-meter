import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';

// Settings defaults
const DEFAULT_SETTINGS = {
    unit: 'B',           // 'iB', 'B', 'b'
    graphType: 'combined', // 'combined', 'separate', 'hidden'
    downGraphStyle: 'dashed', // 'dashed', 'solid'
    upGraphStyle: 'dashed',   // 'dashed', 'solid'
    opacity: 0.85,       // 0.1 to 1.0
    locked: false,       // position locked (disable drag)
    clickThrough: false, // click-through mode
    samplingRate: 1000,  // ms
    idleTimeout: 5,      // minutes
    batterySaver: true,  // disable on low battery
    widgetTheme: 'system',    // 'system', 'dark', 'light'
    dashboardTheme: 'system', // 'system', 'dark', 'light'
    theme: 'system',          // legacy fallback
    accentColor: 'emerald',
    dailyLimitEnabled: false,
    dailyLimitGB: 5,
    monthlyLimitEnabled: false,
    monthlyLimitGB: 50,
    globalHotkey: 'Ctrl+Shift+S',
    showPing: true,
    showWidgetPeak: true,
    filterWidgetNoise: true,
    useEtwTelemetry: false,
    filterNisTraffic: false, // Legacy fallback flag
    attributionMode: 'proportional', // Options: 'proportional' (Default), 'exact', 'system_row'
    telemetryEngine: 'estats' // Options: 'estats' (Default), 'io', 'etw'
};

// Accent color palette definitions
export const ACCENT_COLORS = {
    emerald: { light: "#059669", dark: "#10b981", name: "Emerald" },
    violet: { light: "#7c3aed", dark: "#8b5cf6", name: "Violet" },
    sky: { light: "#0284c7", dark: "#38bdf8", name: "Sky Blue" },
    amber: { light: "#d97706", dark: "#f59e0b", name: "Amber" },
    rose: { light: "#e11d48", dark: "#f43f5e", name: "Rose" },
    coral: { light: "#ea580c", dark: "#f97316", name: "Coral" }
};

export function applyAccentTheme(accentName, theme) {
    if (typeof document === 'undefined') return;
    const palette = ACCENT_COLORS[accentName] || ACCENT_COLORS.emerald;
    const isDark = theme === 'dark' || (theme !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    const color = isDark ? palette.dark : palette.light;

    document.documentElement.style.setProperty('--metric-down', color);
    document.documentElement.style.setProperty('--accent-emerald', color);
    document.documentElement.style.setProperty('--input-focus', color);
    document.documentElement.style.setProperty('--widget-hover-border', `${color}55`);
    document.documentElement.style.setProperty('--chart-down-fill', `${color}22`);
}

function createSettingsStore() {
    let initial = { ...DEFAULT_SETTINGS };

    const store = writable(initial);
    const { subscribe, set, update } = store;

    if (typeof window !== 'undefined') {
        subscribe(value => {
            if (value) {
                if (value.theme) {
                    document.documentElement.setAttribute('data-theme', value.theme);
                }
                applyAccentTheme(value.accentColor, value.theme);
            }
        });

        // Cross-window synchronization: Listen for setting changes from other WebView windows
        listen('settings-changed', (event) => {
            if (event.payload) {
                store.set({ ...DEFAULT_SETTINGS, ...event.payload });
            }
        }).catch(err => console.error("Failed to listen for settings-changed", err));
    }

    const saveToDb = async (value) => {
        if (typeof window === 'undefined') return;
        try {

            await invoke('save_app_setting', { key: 'speed_meter_settings', value: JSON.stringify(value) });
            await emit('settings-changed', value);
        } catch (err) {
            console.error("Failed to save settings to SQLite DB", err);
        }
    };

    return {
        subscribe,
        set: (value) => {
            set(value);
            saveToDb(value);
        },
        update: (updater) => {
            update(current => {
                const next = updater(current);
                saveToDb(next);
                return next;
            });
        },
        // Primary loader: reads directly from SQLite DB
        loadFromDb: async () => {
            if (typeof window === 'undefined') return DEFAULT_SETTINGS;
            try {
                const dbValue = await invoke('get_app_setting', { key: 'speed_meter_settings' });
                if (dbValue) {
                    const parsed = JSON.parse(dbValue);
                    const merged = { ...DEFAULT_SETTINGS, ...parsed };
                    store.set(merged);
                    return merged;
                } else {
                    // One-time migration: If SQLite DB is empty, check legacy localStorage
                    const legacy = localStorage.getItem('speed_meter_settings');
                    if (legacy) {
                        try {
                            const parsed = JSON.parse(legacy);
                            const merged = { ...DEFAULT_SETTINGS, ...parsed };
                            store.set(merged);
                            await saveToDb(merged);
                            return merged;
                        } catch (e) {}
                    }
                }
            } catch (e) {
                console.error("Failed to load settings from SQLite DB", e);
            }
            return DEFAULT_SETTINGS;
        },
        // Sync specific runtime configuration with Rust backend & persist to SQLite DB
        syncWithBackend: async (settings) => {
            try {
                await invoke('set_widget_locked', { locked: settings.locked });
                await invoke('toggle_click_through', { enabled: settings.clickThrough });
                const engine = settings.telemetryEngine || (settings.useEtwTelemetry ? 'etw' : 'estats');
                await invoke('set_telemetry_engine', { engine });
                await invoke('set_attribution_mode', { mode: settings.attributionMode || 'proportional' });
                await invoke('set_filter_nis_traffic', { enabled: settings.attributionMode === 'system_row' || (settings.filterNisTraffic ?? false) });
                // Primary persistence to SQLite DB
                await saveToDb(settings);
            } catch (e) {
                console.error("Backend sync failed", e);
            }
        }
    };
}

export const settings = createSettingsStore();

// Utility for speed formatting
export function formatSpeed(bytesPerSec, unitType) {
    if (bytesPerSec === undefined || isNaN(bytesPerSec)) return '0 B/s';

    if (unitType === 'b') {
        // Bits/sec (decimal)
        const bits = bytesPerSec * 8;
        if (bits < 1000) return `${Math.round(bits)} bps`;
        const kbps = bits / 1000;
        if (kbps < 1000) return `${Math.round(kbps)} Kbps`;
        const mbps = kbps / 1000;
        if (mbps < 1000) return `${mbps.toFixed(1)} Mbps`;
        return `${(mbps / 1000).toFixed(1)} Gbps`;
    } else if (unitType === 'ib') {
        // Binary Bits/sec (kibps, mibps)
        const bits = bytesPerSec * 8;
        if (bits < 1024) return `${Math.round(bits)} bps`;
        const kibps = bits / 1024;
        if (kibps < 1024) return `${Math.round(kibps)} Kibps`;
        const mibps = kibps / 1024;
        if (mibps < 1024) return `${mibps.toFixed(1)} Mibps`;
        return `${(mibps / 1024).toFixed(1)} Gibps`;
    } else if (unitType === 'iB') {
        // Binary Bytes/sec (KiB, MiB)
        if (bytesPerSec < 1024) return `${Math.round(bytesPerSec)} B/s`;
        const kib = bytesPerSec / 1024;
        if (kib < 1024) return `${Math.round(kib)} KiB/s`;
        const mib = kib / 1024;
        if (mib < 1024) return `${mib.toFixed(1)} MiB/s`;
        return `${(mib / 1024).toFixed(1)} GiB/s`;
    } else {
        // Decimal Bytes/sec (KB, MB) - DEFAULT
        if (bytesPerSec < 1000) return `${Math.round(bytesPerSec)} B/s`;
        const kb = bytesPerSec / 1000;
        if (kb < 1000) return `${Math.round(kb)} KB/s`;
        const mb = kb / 1000;
        if (mb < 1000) return `${mb.toFixed(1)} MB/s`;
        return `${(mb / 1000).toFixed(1)} GB/s`;
    }
}
