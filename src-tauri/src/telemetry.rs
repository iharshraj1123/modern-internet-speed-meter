use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use chrono::Local;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::NetworkManagement::IpHelper::{GetIfTable2, FreeMibTable, MIB_IF_TABLE2};

// Telemetry state that will be updated in real-time
#[derive(serde::Serialize, Clone, Debug)]
pub struct RealtimeStats {
    pub download_speed: u64, // Bytes per second
    pub upload_speed: u64,   // Bytes per second
    pub battery_percentage: u8,
    pub is_charging: bool,
    pub active_app: String,
    pub ping_ms: u32,         // Latency in milliseconds (0 = offline)
}

// Local accumulator for process telemetry
struct ProcessStatsAccumulator {
    bytes_downloaded: u64,
    bytes_uploaded: u64,
    screen_time_seconds: u32,
}

extern "system" {
    fn GetTickCount64() -> u64;
}

// Helper: Get active foreground window process name
fn get_active_process_name() -> String {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return "Idle".to_string();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return "Idle".to_string();
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        if let Ok(handle) = handle {
            let mut buffer = [0u16; 260];
            let len = GetModuleFileNameExW(handle, None, &mut buffer);
            CloseHandle(handle).ok();
            if len > 0 {
                let path = String::from_utf16_lossy(&buffer[..len as usize]);
                if let Some(filename) = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|f| f.to_str()) {
                    return filename.to_string();
                }
            }
        }
    }
    "System".to_string()
}

// Helper: Check user idle time in milliseconds
fn get_idle_time_millis() -> u64 {
    unsafe {
        let mut lii = LASTINPUTINFO::default();
        lii.cbSize = std::mem::size_of::<LASTINPUTINFO>() as u32;
        if GetLastInputInfo(&mut lii).as_bool() {
            let tick_count = GetTickCount64();
            return tick_count.saturating_sub(lii.dwTime as u64);
        }
    }
    0
}

// Helper: Get total Rx and Tx bytes across active network adapters
fn get_total_network_octets() -> (u64, u64) {
    let mut rx = 0;
    let mut tx = 0;
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_ok() {
            let slice = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
            for row in slice {
                // Filter for operational physical interfaces (type 6=Ethernet, 71=WiFi, 23=Mobile, 244=WWAN)
                if row.OperStatus.0 == 1 && (row.Type == 6 || row.Type == 71 || row.Type == 244 || row.Type == 23) {
                    rx += row.InOctets;
                    tx += row.OutOctets;
                }
            }
            FreeMibTable(table as *const std::ffi::c_void);
        }
    }
    (rx, tx)
}

// Helper: Get battery stats
fn get_battery_info() -> (u8, bool) {
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_ok() {
            let percent = if status.BatteryLifePercent == 255 { 100 } else { status.BatteryLifePercent };
            let charging = (status.BatteryFlag & 8) != 0 || status.ACLineStatus == 1;
            return (percent, charging);
        }
    }
    (100, true)
}

// Helper: Measure TCP round-trip time to Cloudflare DNS (1.1.1.1:80)
fn measure_ping() -> u32 {
    let start = Instant::now();
    match TcpStream::connect_timeout(
        &"1.1.1.1:80".parse().unwrap(),
        Duration::from_secs(3),
    ) {
        Ok(_) => start.elapsed().as_millis() as u32,
        Err(_) => 0,
    }
}

pub struct TelemetryService {
    realtime_sender: broadcast::Sender<RealtimeStats>,
    db_path: String,
}

impl TelemetryService {
    pub fn new(db_path: String) -> (Self, broadcast::Receiver<RealtimeStats>) {
        let (tx, rx) = broadcast::channel(100);
        (
            TelemetryService {
                realtime_sender: tx,
                db_path,
            },
            rx,
        )
    }

    pub fn start(&self) {
        let tx = self.realtime_sender.clone();
        let db_path = self.db_path.clone();

        std::thread::spawn(move || {
            let mut last_octets = get_total_network_octets();
            let mut last_poll = Instant::now();
            let mut last_db_flush = Instant::now();
            let mut tick_count: u32 = 0;
            let mut current_ping_ms: u32 = 0;
            
            // Local map to buffer stats before writing to SQLite
            let mut accumulator: HashMap<String, ProcessStatsAccumulator> = HashMap::new();

            loop {
                std::thread::sleep(Duration::from_millis(1000));
                
                let now = Instant::now();
                let duration = now.duration_since(last_poll);
                last_poll = now;

                // 1. Calculate Network Speed
                let current_octets = get_total_network_octets();
                let rx_diff = current_octets.0.saturating_sub(last_octets.0);
                let tx_diff = current_octets.1.saturating_sub(last_octets.1);
                last_octets = current_octets;

                let secs = duration.as_secs_f64();
                let download_speed = if secs > 0.0 { (rx_diff as f64 / secs) as u64 } else { 0 };
                let upload_speed = if secs > 0.0 { (tx_diff as f64 / secs) as u64 } else { 0 };

                // 2. Query Active App & Screen Time
                let active_app = get_active_process_name();
                let idle_ms = get_idle_time_millis();
                
                // If idle for more than 5 minutes, we don't count screen time
                let is_idle = idle_ms > (5 * 60 * 1000);
                let active_time = if is_idle || active_app == "Idle" { 0 } else { 1 };

                // 3. Query Battery Info
                let (battery_pct, is_charging) = get_battery_info();

                // 4. Measure ping every 5 ticks (~5 seconds) in a separate thread to avoid blocking
                tick_count = tick_count.wrapping_add(1);
                if tick_count % 5 == 1 {
                    current_ping_ms = measure_ping();
                }

                // 5. Update the real-time structure
                let stats = RealtimeStats {
                    download_speed,
                    upload_speed,
                    battery_percentage: battery_pct,
                    is_charging,
                    active_app: active_app.clone(),
                    ping_ms: current_ping_ms,
                };
                
                // Send real-time updates to UI
                let _ = tx.send(stats);

                // 5. Accumulate telemetry data per process
                // Note: We attribute the system-wide network transfer delta to the active foreground process.
                // This is a highly efficient estimate for screen-time-active apps.
                let entry = accumulator.entry(active_app.clone()).or_insert(ProcessStatsAccumulator {
                    bytes_downloaded: 0,
                    bytes_uploaded: 0,
                    screen_time_seconds: 0,
                });
                entry.bytes_downloaded += rx_diff;
                entry.bytes_uploaded += tx_diff;
                entry.screen_time_seconds += active_time;

                // 6. Every 60 seconds, flush accumulated metrics to SQLite
                if now.duration_since(last_db_flush) >= Duration::from_secs(60) {
                    last_db_flush = now;
                    
                    if let Ok(conn) = crate::db::open_conn(&db_path) {
                        let epoch_now = Local::now().timestamp();
                        // Round down to the nearest 5-minute interval (300 seconds)
                        let interval_timestamp = (epoch_now / 300) * 300;

                        for (process_name, acc) in accumulator.drain() {
                            let _ = crate::db::log_interval(
                                &conn,
                                interval_timestamp,
                                &process_name,
                                acc.bytes_downloaded,
                                acc.bytes_uploaded,
                                acc.screen_time_seconds,
                            );
                        }
                        
                        // Run database rolls/cleanup
                        let _ = crate::db::aggregate_data(&conn);
                    }
                }
            }
        });
    }
}
