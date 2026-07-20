use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use chrono::Local;
use once_cell::sync::Lazy;

static SELF_EXE_NAME: Lazy<String> = Lazy::new(|| {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|f| f.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tauri-app.exe".to_string())
});


use windows::Win32::Foundation::CloseHandle;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, GetProcessIoCounters, IO_COUNTERS};
use windows::Win32::System::ProcessStatus::{GetModuleFileNameExW, EnumProcesses};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::NetworkManagement::IpHelper::{GetIfTable2, FreeMibTable, MIB_IF_TABLE2};

#[derive(serde::Serialize, Clone, Debug)]
pub struct ProcessSpeed {
    pub name: String,
    pub download_speed: u64,
    pub upload_speed: u64,
}

// Telemetry state that will be updated in real-time
#[derive(serde::Serialize, Clone, Debug)]
pub struct RealtimeStats {
    pub download_speed: u64, // Bytes per second
    pub upload_speed: u64,   // Bytes per second
    pub battery_percentage: u8,
    pub is_charging: bool,
    pub active_app: String,
    pub ping_ms: u32,         // Latency in milliseconds (0 = offline)
    pub process_speeds: Vec<ProcessSpeed>,
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

struct ProcessIoSnapshot {
    pid: u32,
    name: String,
    read_bytes: u64,
    write_bytes: u64,
}

fn get_all_processes_io() -> Vec<ProcessIoSnapshot> {
    let mut result = Vec::new();
    unsafe {
        let mut pids = [0u32; 1024];
        let mut cb_needed = 0u32;
        if EnumProcesses(
            pids.as_mut_ptr(),
            std::mem::size_of_val(&pids) as u32,
            &mut cb_needed,
        ).is_ok() {
            let count = (cb_needed as usize / std::mem::size_of::<u32>()).min(1024);
            for &pid in &pids[..count] {
                if pid == 0 {
                    continue;
                }
                let handle = OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION,
                    false,
                    pid,
                );
                if let Ok(handle) = handle {
                    let mut io_counters = IO_COUNTERS::default();
                    if GetProcessIoCounters(handle, &mut io_counters).is_ok() {
                        // Get process name
                        let mut buffer = [0u16; 260];
                        let len = GetModuleFileNameExW(handle, None, &mut buffer);
                        if len > 0 {
                            let path = String::from_utf16_lossy(&buffer[..len as usize]);
                            if let Some(filename) = std::path::Path::new(&path)
                                .file_name()
                                .and_then(|f| f.to_str()) {
                                result.push(ProcessIoSnapshot {
                                    pid,
                                    name: filename.to_string(),
                                    read_bytes: io_counters.ReadTransferCount,
                                    write_bytes: io_counters.WriteTransferCount,
                                });
                            }
                        }
                    }
                    CloseHandle(handle).ok();
                }
            }
        }
    }
    result
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
            
            // For tracking PIDs to their previous IO stats
            let mut last_process_io: HashMap<u32, (u64, u64)> = HashMap::new();

            // Local map to buffer stats before writing to SQLite
            let mut accumulator: HashMap<String, ProcessStatsAccumulator> = HashMap::new();

            loop {
                std::thread::sleep(Duration::from_millis(1000));
                
                let now = Instant::now();
                let duration = now.duration_since(last_poll);
                last_poll = now;

                // 1. Calculate system-wide network interface speed
                let current_octets = get_total_network_octets();
                let rx_diff = current_octets.0.saturating_sub(last_octets.0);
                let tx_diff = current_octets.1.saturating_sub(last_octets.1);
                last_octets = current_octets;

                let secs = duration.as_secs_f64();
                let download_speed = if secs > 0.0 { (rx_diff as f64 / secs) as u64 } else { 0 };
                let upload_speed = if secs > 0.0 { (tx_diff as f64 / secs) as u64 } else { 0 };

                // 2. Query active foreground app (for screen time tracking)
                let active_app = get_active_process_name();
                let idle_ms = get_idle_time_millis();
                
                // If idle for more than 5 minutes, we don't count screen time
                let is_idle = idle_ms > (5 * 60 * 1000);
                let active_time = if is_idle || active_app == "Idle" { 0 } else { 1 };

                // 3. Query I/O stats for all running processes
                let current_proc_snapshots = get_all_processes_io();
                let mut active_io_procs = Vec::new();
                let mut total_read_delta = 0u64;
                let mut total_write_delta = 0u64;

                let mut current_pids = std::collections::HashSet::new();

                for proc in &current_proc_snapshots {
                    current_pids.insert(proc.pid);
                    
                    let prev_io = last_process_io.get(&proc.pid);
                    let (read_delta, write_delta) = match prev_io {
                        Some(&(prev_r, prev_w)) => {
                            let r_delta = proc.read_bytes.saturating_sub(prev_r);
                            let w_delta = proc.write_bytes.saturating_sub(prev_w);
                            (r_delta, w_delta)
                        }
                        None => (0, 0),
                    };

                    // Update last known process IO stats
                    last_process_io.insert(proc.pid, (proc.read_bytes, proc.write_bytes));

                    if read_delta > 0 || write_delta > 0 {
                        active_io_procs.push((proc.name.clone(), read_delta, write_delta));
                        total_read_delta += read_delta;
                        total_write_delta += write_delta;
                    }
                }

                // Clean up dead PIDs from our tracking map to prevent memory leaks
                last_process_io.retain(|pid, _| current_pids.contains(pid));

                // 4. Distribute system network speeds across active I/O processes proportionally (calibration)
                let mut process_speeds = Vec::new();

                if total_read_delta > 0 || total_write_delta > 0 {
                    let mut speed_map: HashMap<String, (u64, u64)> = HashMap::new();

                    for (name, r_delta, w_delta) in active_io_procs {
                        let proc_down_speed = if total_read_delta > 0 {
                            (r_delta as f64 / total_read_delta as f64 * download_speed as f64) as u64
                        } else {
                            0
                        };

                        let proc_up_speed = if total_write_delta > 0 {
                            (w_delta as f64 / total_write_delta as f64 * upload_speed as f64) as u64
                        } else {
                            0
                        };

                        if proc_down_speed > 0 || proc_up_speed > 0 {
                            let speed_entry = speed_map.entry(name.clone()).or_insert((0, 0));
                            speed_entry.0 += proc_down_speed;
                            speed_entry.1 += proc_up_speed;

                            let entry = accumulator.entry(name).or_insert(ProcessStatsAccumulator {
                                bytes_downloaded: 0,
                                bytes_uploaded: 0,
                                screen_time_seconds: 0,
                            });
                            entry.bytes_downloaded += proc_down_speed;
                            entry.bytes_uploaded += proc_up_speed;
                        }
                    }

                    for (name, (down, up)) in speed_map {
                        process_speeds.push(ProcessSpeed {
                            name,
                            download_speed: down,
                            upload_speed: up,
                        });
                    }
                } else if download_speed > 0 || upload_speed > 0 {
                    // Fallback: If no process was captured having I/O we attribute network speed to foreground
                    let fallback_app = active_app.clone();
                    process_speeds.push(ProcessSpeed {
                        name: fallback_app.clone(),
                        download_speed,
                        upload_speed,
                    });

                    let entry = accumulator.entry(fallback_app).or_insert(ProcessStatsAccumulator {
                        bytes_downloaded: 0,
                        bytes_uploaded: 0,
                        screen_time_seconds: 0,
                    });
                    entry.bytes_downloaded += rx_diff;
                    entry.bytes_uploaded += tx_diff;
                }

                // Accumulate screen time seconds for the actual active foreground app
                let screen_entry = accumulator.entry(active_app.clone()).or_insert(ProcessStatsAccumulator {
                    bytes_downloaded: 0,
                    bytes_uploaded: 0,
                    screen_time_seconds: 0,
                });
                screen_entry.screen_time_seconds += active_time;

                // 5. Query Battery Info
                let (battery_pct, is_charging) = get_battery_info();

                // 6. Measure ping latency every 5 ticks (~5 seconds)
                tick_count = tick_count.wrapping_add(1);
                if tick_count % 5 == 1 {
                    current_ping_ms = measure_ping();
                }

                // 7. Update real-time stats struct and broadcast it to the Svelte UI
                let stats = RealtimeStats {
                    download_speed,
                    upload_speed,
                    battery_percentage: battery_pct,
                    is_charging,
                    active_app: active_app.clone(), // Widget displays actual active app
                    ping_ms: current_ping_ms,
                    process_speeds,
                };
                
                let _ = tx.send(stats);

                // 8. Every 60 seconds, flush accumulated telemetry metrics to SQLite
                if now.duration_since(last_db_flush) >= Duration::from_secs(60) {
                    last_db_flush = now;
                    
                    if let Ok(conn) = crate::db::open_conn(&db_path) {
                        let epoch_now = Local::now().timestamp();
                        let interval_timestamp = (epoch_now / 300) * 300; // Round to 5-minute bucket

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
                        
                        let _ = crate::db::aggregate_data(&conn);
                    }
                }
            }
        });
    }
}
