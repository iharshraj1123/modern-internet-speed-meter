mod db;
mod telemetry;

use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindowBuilder, WebviewUrl, Emitter};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
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
    })
});

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
        // Handle database reset request safely
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

// Tauri command: Toggle widget position lock (draggable vs fixed)
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
        w.show().unwrap();
        w.set_focus().unwrap();
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
        w.show().unwrap();
        w.set_focus().unwrap();
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Force rebuild to bundle updated icons
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--start-minimized"]),
        ))
        .setup(|app| {
            // 1. Setup SQLite Database in App Local Directory
            let app_dir = app.path().app_data_dir().unwrap();
            std::fs::create_dir_all(&app_dir).unwrap();
            let db_path = app_dir.join("speed_meter.db").to_string_lossy().into_owned();
            
            // Store DB Path globally
            *DB_PATH.lock().unwrap() = db_path.clone();

            // Initialize SQLite schema
            let conn = db::init_db(&db_path).unwrap();
            db::aggregate_data(&conn).ok(); // Initial aggregation run

            // 2. Initialize Telemetry Service
            let (telemetry_service, mut stats_rx) = telemetry::TelemetryService::new(db_path);
            telemetry_service.start();

            // 3. Broadcast real-time stats to all frontend windows
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(stats) = stats_rx.recv().await {
                    // Update global state cache
                    *LATEST_STATS.lock().unwrap() = stats.clone();
                    // Emit update to all active webview windows
                    let _ = app_handle.emit("realtime-stats", stats);
                }
            });

            // 4. Construct Tray Icon & Menu
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
            open_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
