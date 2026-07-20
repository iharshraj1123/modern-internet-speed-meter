mod db;
mod telemetry;

use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindowBuilder, WebviewUrl, Emitter};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_autostart::ManagerExt;
use once_cell::sync::Lazy;

// Global SQLite connection path
static DB_PATH: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
// Global state to track real-time metrics
static LATEST_STATS: Lazy<Mutex<telemetry::RealtimeStats>> = Lazy::new(|| {
    Mutex::new(telemetry::RealtimeStats {
        download_speed: 0,
        upload_speed: 0,
        battery_percentage: 100,
        is_charging: true,
        active_app: "System".to_string(),
        ping_ms: 0,
        process_speeds: Vec::new(),
    })
});
// Track which data-limit alert thresholds have already been sent (to avoid spam)
static ALERTED_THRESHOLDS: Lazy<Mutex<std::collections::HashSet<String>>> =
    Lazy::new(|| Mutex::new(std::collections::HashSet::new()));

// Tauri command: Get current real-time stats
#[tauri::command]
fn get_realtime_stats() -> telemetry::RealtimeStats {
    LATEST_STATS.lock().unwrap().clone()
}

// Tauri command: Get historical telemetry metrics (hourly, daily, weekly, monthly, yearly)
#[tauri::command]
async fn get_historical_stats(period: String) -> Result<Vec<db::ProcessStat>, String> {
    let path = DB_PATH.lock().unwrap().clone();
    if period == "clear" {
        if let Ok(conn) = db::open_conn(&path) {
            let _ = conn.execute("DELETE FROM process_telemetry", []);
            let _ = conn.execute("DELETE FROM hourly_stats", []);
            let _ = conn.execute("DELETE FROM daily_stats", []);
        }
        return Ok(Vec::new());
    }
    let conn = db::open_conn(&path).map_err(|e| e.to_string())?;
    db::get_stats_for_period(&conn, &period).map_err(|e| e.to_string())
}

// Tauri command: Toggle widget position lock
#[tauri::command]
async fn set_widget_locked(app: AppHandle, locked: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.emit("widget-lock-changed", locked);
    }
    Ok(())
}

// Tauri command: Toggle click-through mode
#[tauri::command]
async fn toggle_click_through(app: AppHandle, enabled: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        w.set_ignore_cursor_events(enabled).map_err(|e| e.to_string())?;
        let _ = w.emit("click-through-changed", enabled);
    }
    Ok(())
}

// Tauri command: Open Analytics Dashboard Window
#[tauri::command]
async fn open_dashboard(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("dashboard") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    } else {
        let _dashboard = WebviewWindowBuilder::new(&app, "dashboard", WebviewUrl::App("/dashboard".into()))
            .title("Analytics Dashboard")
            .inner_size(850.0, 600.0)
            .min_inner_size(600.0, 450.0)
            .resizable(true)
            .build()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Tauri command: Open Settings Window
#[tauri::command]
async fn open_settings(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    } else {
        let _settings = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("/settings".into()))
            .title("Settings")
            .inner_size(680.0, 500.0)
            .min_inner_size(550.0, 400.0)
            .resizable(true)
            .build()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Tauri command: Hide main widget window
#[tauri::command]
async fn hide_widget(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    Ok(())
}

// Tauri command: Close application completely
#[tauri::command]
async fn close_app(app: AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

// Tauri command: Open URL in default system browser without cmd flash
#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .spawn();
    }
    Ok(())
}


// Tauri command: Get database info (size, row counts, retention settings)
#[tauri::command]
async fn get_db_info() -> Result<db::DbInfo, String> {
    let path = DB_PATH.lock().unwrap().clone();
    let conn = db::open_conn(&path).map_err(|e| e.to_string())?;
    db::get_db_info(&conn, &path).map_err(|e| e.to_string())
}

// Tauri command: Update retention policy
#[tauri::command]
async fn set_retention_policy(raw_days: i64, hourly_days: i64) -> Result<(), String> {
    let path = DB_PATH.lock().unwrap().clone();
    let conn = db::open_conn(&path).map_err(|e| e.to_string())?;
    db::set_retention_policy(&conn, raw_days, hourly_days).map_err(|e| e.to_string())
}

// Tauri command: Vacuum / optimize the database
#[tauri::command]
async fn vacuum_db() -> Result<(), String> {
    let path = DB_PATH.lock().unwrap().clone();
    let conn = db::open_conn(&path).map_err(|e| e.to_string())?;
    db::vacuum_db(&conn).map_err(|e| e.to_string())
}

// Tauri command: Get today's total bandwidth usage in bytes
#[tauri::command]
async fn get_today_usage() -> Result<(u64, u64), String> {
    let path = DB_PATH.lock().unwrap().clone();
    let conn = db::open_conn(&path).map_err(|e| e.to_string())?;
    db::get_today_total_bytes(&conn).map_err(|e| e.to_string())
}

// Tauri command: Get this month's total bandwidth usage in bytes
#[tauri::command]
async fn get_month_usage() -> Result<(u64, u64), String> {
    let path = DB_PATH.lock().unwrap().clone();
    let conn = db::open_conn(&path).map_err(|e| e.to_string())?;
    db::get_month_total_bytes(&conn).map_err(|e| e.to_string())
}

// Tauri command: Check data limits and fire a notification if thresholds are crossed
#[tauri::command]
async fn check_data_limits(
    app: AppHandle,
    daily_limit_bytes: u64,
    monthly_limit_bytes: u64,
) -> Result<(), String> {
    let path = DB_PATH.lock().unwrap().clone();
    let conn = db::open_conn(&path).map_err(|e| e.to_string())?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    if daily_limit_bytes > 0 {
        let (dl, ul) = db::get_today_total_bytes(&conn).map_err(|e| e.to_string())?;
        let total = dl + ul;
        let pct = (total as f64 / daily_limit_bytes as f64 * 100.0) as u64;

        for threshold in [100u64, 80] {
            let key = format!("daily_{}_{}", threshold, today);
            if pct >= threshold && !ALERTED_THRESHOLDS.lock().unwrap().contains(&key) {
                ALERTED_THRESHOLDS.lock().unwrap().insert(key);
                let msg = if threshold == 100 {
                    "You have reached your daily data limit.".to_string()
                } else {
                    format!("You have used {}% of your daily data limit.", threshold)
                };
                let _ = tauri_plugin_notification::NotificationExt::notification(&app)
                    .builder()
                    .title("Data Limit Alert")
                    .body(&msg)
                    .show();
                break;
            }
        }
    }

    if monthly_limit_bytes > 0 {
        let (dl, ul) = db::get_month_total_bytes(&conn).map_err(|e| e.to_string())?;
        let total = dl + ul;
        let pct = (total as f64 / monthly_limit_bytes as f64 * 100.0) as u64;
        let month = chrono::Local::now().format("%Y-%m").to_string();

        for threshold in [100u64, 80] {
            let key = format!("monthly_{}_{}", threshold, month);
            if pct >= threshold && !ALERTED_THRESHOLDS.lock().unwrap().contains(&key) {
                ALERTED_THRESHOLDS.lock().unwrap().insert(key);
                let msg = if threshold == 100 {
                    "You have reached your monthly data limit.".to_string()
                } else {
                    format!("You have used {}% of your monthly data limit.", threshold)
                };
                let _ = tauri_plugin_notification::NotificationExt::notification(&app)
                    .builder()
                    .title("Data Limit Alert")
                    .body(&msg)
                    .show();
                break;
            }
        }
    }

    Ok(())
}

// Tauri command: Register a global hotkey shortcut
#[tauri::command]
async fn register_hotkey(app: AppHandle, shortcut: String) -> Result<(), String> {
    // Unregister all existing shortcuts first
    let _ = app.global_shortcut().unregister_all();

    let app_clone = app.clone();
    app.global_shortcut()
        .on_shortcut(shortcut.as_str(), move |_app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                if let Some(w) = app_clone.get_webview_window("main") {
                    if let Ok(visible) = w.is_visible() {
                        if visible {
                            let _ = w.hide();
                        } else {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;

    Ok(())
}

// Tauri command: Unregister global hotkey
#[tauri::command]
async fn unregister_hotkey(app: AppHandle) -> Result<(), String> {
    app.global_shortcut().unregister_all().map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--start-minimized"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // Enable autostart with PC by default on first launch
            let autostart_manager = app.autolaunch();
            if let Ok(enabled) = autostart_manager.is_enabled() {
                if !enabled {
                    let _ = autostart_manager.enable();
                }
            }

            // 1. Setup SQLite Database in App Local Directory
            let app_dir = app.path().app_data_dir().unwrap();
            std::fs::create_dir_all(&app_dir).unwrap();
            let db_path = app_dir.join("speed_meter.db").to_string_lossy().into_owned();
            
            // Store DB Path globally
            *DB_PATH.lock().unwrap() = db_path.clone();

            // Initialize SQLite schema
            let conn = db::init_db(&db_path).unwrap();
            db::aggregate_data(&conn).ok();

            // 2. Initialize Telemetry Service
            let (telemetry_service, mut stats_rx) = telemetry::TelemetryService::new(db_path);
            telemetry_service.start();

            // 3. Broadcast real-time stats to all frontend windows
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(stats) = stats_rx.recv().await {
                    *LATEST_STATS.lock().unwrap() = stats.clone();
                    let _ = app_handle.emit("realtime-stats", stats);
                }
            });

            // 4. Register default global hotkey (Ctrl+Shift+S)
            let app_handle2 = app.handle().clone();
            let _ = app.global_shortcut().on_shortcut("Ctrl+Shift+S", move |_app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    if let Some(w) = app_handle2.get_webview_window("main") {
                        if let Ok(visible) = w.is_visible() {
                            if visible { let _ = w.hide(); } else { let _ = w.show(); let _ = w.set_focus(); }
                        }
                    }
                }
            });

            // 5. Construct Tray Icon & Menu
            let toggle_widget = MenuItem::with_id(app, "toggle_widget", "Show/Hide Widget", true, None::<&str>).unwrap();
            let open_dash = MenuItem::with_id(app, "open_dashboard", "Open Dashboard", true, None::<&str>).unwrap();
            let open_set = MenuItem::with_id(app, "open_settings", "Settings", true, None::<&str>).unwrap();
            let quit_app = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>).unwrap();

            let tray_menu = Menu::with_items(app, &[
                &toggle_widget,
                &open_dash,
                &open_set,
                &PredefinedMenuItem::separator(app).unwrap(),
                &quit_app,
            ]).unwrap();

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event {
                        let app_handle = tray.app_handle();
                        if let Some(w) = app_handle.get_webview_window("main") {
                            if let Ok(visible) = w.is_visible() {
                                if visible {
                                    w.hide().unwrap();
                                } else {
                                    w.show().unwrap();
                                    w.set_focus().unwrap();
                                }
                            }
                        }
                    }
                })
                .on_menu_event(|app_handle, event| {
                    match event.id.as_ref() {
                        "toggle_widget" => {
                            if let Some(w) = app_handle.get_webview_window("main") {
                                if let Ok(visible) = w.is_visible() {
                                    if visible {
                                        w.hide().unwrap();
                                    } else {
                                        w.show().unwrap();
                                        w.set_focus().unwrap();
                                    }
                                }
                            }
                        }
                        "open_dashboard" => {
                            let handle = app_handle.clone();
                            tauri::async_runtime::spawn(async move {
                                  let _ = open_dashboard(handle).await;
                            });
                        }
                        "open_settings" => {
                            let handle = app_handle.clone();
                            tauri::async_runtime::spawn(async move {
                                  let _ = open_settings(handle).await;
                            });
                        }
                        "quit" => {
                            app_handle.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app).unwrap();

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_realtime_stats,
            get_historical_stats,
            set_widget_locked,
            toggle_click_through,
            open_dashboard,
            open_settings,
            hide_widget,
            close_app,
            open_url,
            get_db_info,
            set_retention_policy,
            vacuum_db,
            get_today_usage,
            get_month_usage,
            check_data_limits,
            register_hotkey,
            unregister_hotkey,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
