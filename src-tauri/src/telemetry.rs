use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use chrono::Local;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::Mutex;
use windows::core::GUID;
use windows::Win32::System::Diagnostics::Etw::{
    ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW, CloseTrace,
    CONTROLTRACE_HANDLE,
    EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP,
    EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
    PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{GetProcessIoCounters, IO_COUNTERS};

use windows::Win32::Foundation::{CloseHandle, BOOLEAN};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetBestInterface, GetIfTable2, MIB_IF_TABLE2,
    GetExtendedTcpTable, GetPerTcpConnectionEStats, SetPerTcpConnectionEStats,
    MIB_TCPROW_LH, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_CONNECTIONS,
    TcpConnectionEstatsData, TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum realistic consumer/enterprise internet speed threshold.
/// 1.5 GB/s = 12 Gbps — anything above this on a single NIC poll tick is a
/// measurement artifact (counter wrap, adapter reset race, etc.) and gets discarded.
const MAX_REALISTIC_BPS: u64 = 1_500_000_000;

/// ERROR_INSUFFICIENT_BUFFER Win32 error code — returned by GetExtendedTcpTable
/// when the caller-supplied buffer is too small; the required size is written back.
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

// ---------------------------------------------------------------------------
// Public types exposed to the rest of the crate
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, Clone, Debug)]
pub struct ProcessSpeed {
    pub name: String,
    pub download_speed: u64,
    pub upload_speed: u64,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct RealtimeStats {
    pub download_speed: u64, // bytes per second (NIC level)
    pub upload_speed: u64,   // bytes per second (NIC level)
    pub battery_percentage: u8,
    pub is_charging: bool,
    pub active_app: String,
    pub ping_ms: u32,        // TCP RTT latency in ms (0 = offline)
    pub process_speeds: Vec<ProcessSpeed>,
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Per-process byte accumulator buffer flushed to SQLite every 60 seconds.
struct ProcessStatsAccumulator {
    bytes_downloaded: u64,
    bytes_uploaded: u64,
    screen_time_seconds: u32,
}

/// Unique key for an IPv4 TCP connection (identifies a socket 4-tuple).
/// Ports are stored in the raw DWORD representation from the Windows API
/// (network byte order packed in a 32-bit value) — consistent across both
/// ends of each comparison so the key is always stable.
#[derive(Eq, PartialEq, Hash, Clone)]
struct TcpConnKey {
    local_addr: u32,
    local_port: u32,
    remote_addr: u32,
    remote_port: u32,
}

/// Snapshot of a TCP connection's *cumulative* DataBytesIn / DataBytesOut
/// at a single point in time, as reported by GetPerTcpConnectionEStats.
/// Deltas between consecutive snapshots give the bytes transferred per tick.
struct TcpConnSnapshot {
    pid: u32,
    bytes_in: u64,  // cumulative bytes received (payload, no IP/TCP headers)
    bytes_out: u64, // cumulative bytes sent    (payload, no IP/TCP headers)
}

// ---------------------------------------------------------------------------
// FFI helper
// ---------------------------------------------------------------------------

extern "system" {
    fn GetTickCount64() -> u64;
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Check if current process is running with elevated Administrator privileges.
pub fn is_elevated() -> bool {
    unsafe {
        use windows::Win32::System::Threading::GetCurrentProcess;
        let mut token = Default::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut size = 0u32;
            let res = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            );
            let _ = CloseHandle(token);
            if res.is_ok() {
                return elevation.TokenIsElevated != 0;
            }
        }
    }
    false
}

/// Enable a security privilege (e.g. "SeSystemProfilePrivilege") on current process token.
pub fn enable_privilege(privilege_name: &str) -> bool {
    unsafe {
        use windows::Win32::Foundation::{HANDLE, LUID};
        use windows::Win32::Security::{
            LookupPrivilegeValueW, AdjustTokenPrivileges, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;

        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let name_w: Vec<u16> = privilege_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut luid = LUID::default();
        if LookupPrivilegeValueW(None, windows::core::PCWSTR(name_w.as_ptr()), &mut luid).is_err() {
            let _ = CloseHandle(token);
            return false;
        }

        let mut tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        let res = AdjustTokenPrivileges(token, false, Some(&mut tp), 0, None, None);
        let _ = CloseHandle(token);
        res.is_ok()
    }
}

/// Relaunch the application with Administrator privileges via UAC prompt ("runas").
pub fn restart_as_admin() -> Result<(), String> {
    unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;

        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_wide: Vec<u16> = exe.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let verb_wide: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();

        let res = ShellExecuteW(
            None,
            PCWSTR(verb_wide.as_ptr()),
            PCWSTR(exe_wide.as_ptr()),
            PCWSTR(std::ptr::null()),
            PCWSTR(std::ptr::null()),
            SW_SHOW,
        );

        if (res.0 as usize) > 32 {
            std::process::exit(0);
        } else {
            Err("UAC elevation prompt was cancelled or failed".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// ETW Real-Time Kernel Tracer Module
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
pub struct EtwPidMetrics {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

static ETW_PID_STATS: Lazy<Mutex<HashMap<u32, EtwPidMetrics>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static ETW_ACTIVE: AtomicBool = AtomicBool::new(false);
static TELEMETRY_ENGINE_MODE: AtomicU8 = AtomicU8::new(0); // 0 = io (default), 1 = estats, 2 = etw
static ETW_EVENT_COUNT: AtomicU64 = AtomicU64::new(0);
static ETW_BYTE_COUNT: AtomicU64 = AtomicU64::new(0);

static LAST_DEBUG_INFO: Lazy<Mutex<TelemetryDebugInfo>> = Lazy::new(|| {
    Mutex::new(TelemetryDebugInfo {
        engine_mode: 0,
        engine_name: "Process I/O".to_string(),
        is_elevated: false,
        etw_active: false,
        etw_events_last_sec: 0,
        etw_bytes_last_sec: 0,
        nic_rx_bytes_last_sec: 0,
        nic_tx_bytes_last_sec: 0,
        active_etw_pids: 0,
        raw_etw_pid_samples: Vec::new(),
        etw_status_log: "ETW Not Started".to_string(),
    })
});

static ETW_STATUS_LOG: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new("ETW Not Started".to_string()));

fn log_etw_status(msg: &str) {
    if let Ok(mut lock) = ETW_STATUS_LOG.lock() {
        *lock = format!("[{}] {}", Local::now().format("%H:%M:%S"), msg);
    }
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct TelemetryDebugInfo {
    pub engine_mode: u8,
    pub engine_name: String,
    pub is_elevated: bool,
    pub etw_active: bool,
    pub etw_events_last_sec: u64,
    pub etw_bytes_last_sec: u64,
    pub nic_rx_bytes_last_sec: u64,
    pub nic_tx_bytes_last_sec: u64,
    pub active_etw_pids: usize,
    pub raw_etw_pid_samples: Vec<(u32, String, u64, u64)>,
    pub etw_status_log: String,
}

pub fn get_telemetry_debug_info() -> TelemetryDebugInfo {
    if let Ok(guard) = LAST_DEBUG_INFO.lock() {
        guard.clone()
    } else {
        TelemetryDebugInfo {
            engine_mode: get_telemetry_engine_mode(),
            engine_name: "Unknown".to_string(),
            is_elevated: is_elevated(),
            etw_active: ETW_ACTIVE.load(Ordering::Relaxed),
            etw_events_last_sec: 0,
            etw_bytes_last_sec: 0,
            nic_rx_bytes_last_sec: 0,
            nic_tx_bytes_last_sec: 0,
            active_etw_pids: 0,
            raw_etw_pid_samples: Vec::new(),
            etw_status_log: ETW_STATUS_LOG.lock().map(|s| s.clone()).unwrap_or_default(),
        }
    }
}

const KERNEL_NETWORK_PROVIDER_GUID: GUID = GUID::from_u128(0x7dd42a49_5329_4832_8dfd_43d979153a88);

pub fn set_telemetry_engine(engine: &str) {
    match engine {
        "io" => TELEMETRY_ENGINE_MODE.store(0, Ordering::Relaxed),
        "etw" => TELEMETRY_ENGINE_MODE.store(2, Ordering::Relaxed),
        _ => TELEMETRY_ENGINE_MODE.store(1, Ordering::Relaxed),
    }
}

pub fn get_telemetry_engine_mode() -> u8 {
    TELEMETRY_ENGINE_MODE.load(Ordering::Relaxed)
}

#[derive(Default, Clone, Copy)]
pub struct ProcessIoSnapshot {
    pub read_bytes: u64,
    pub write_bytes: u64,
}

pub fn snapshot_process_io_counters() -> HashMap<u32, ProcessIoSnapshot> {
    let mut map = HashMap::new();
    unsafe {
        let handle = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return map,
        };
        if handle.is_invalid() {
            return map;
        }

        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(handle, &mut entry).is_ok() {
            loop {
                let pid = entry.th32ProcessID;
                if pid > 4 {
                    if let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                        let mut io = IO_COUNTERS::default();
                        if GetProcessIoCounters(proc_handle, &mut io).is_ok() {
                            map.insert(pid, ProcessIoSnapshot {
                                read_bytes: io.ReadTransferCount,
                                write_bytes: io.WriteTransferCount,
                            });
                        }
                        let _ = CloseHandle(proc_handle);
                    }
                }
                if Process32NextW(handle, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(handle);
    }
    map
}

pub fn ensure_etw_tracer_running() {
    if ETW_ACTIVE.load(Ordering::Relaxed) || !is_elevated() {
        return;
    }
    ETW_ACTIVE.store(true, Ordering::Relaxed);

    std::thread::spawn(|| {
        run_etw_session();
    });
}

fn run_etw_session() {
    unsafe {
        let p1 = enable_privilege("SeSystemProfilePrivilege");
        let p2 = enable_privilege("SeDebugPrivilege");
        log_etw_status(&format!("Initializing ETW session (Privileges: Profile={}, Debug={})...", p1, p2));

        let session_name_raw = format!("ISM_ETW_{}\0", std::process::id());
        let session_name_w: Vec<u16> = session_name_raw.encode_utf16().collect();
        
        let header_size = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
        let name_bytes_len = session_name_w.len() * 2;
        let props_size = header_size + name_bytes_len + 256;
        let mut buffer = vec![0u8; props_size];
        let props = buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        
        (*props).Wnode.BufferSize = props_size as u32;
        (*props).Wnode.Flags = 0x00020000; // WNODE_FLAG_TRACED_GUID
        (*props).LoggerNameOffset = header_size as u32;
        (*props).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*props).FlushTimer = 1; // Flush ETW real-time buffers every 1 second!
        
        // Copy the UTF-16 session name into the buffer immediately following EVENT_TRACE_PROPERTIES struct
        std::ptr::copy_nonoverlapping(
            session_name_w.as_ptr() as *const u8,
            buffer.as_mut_ptr().add(header_size),
            name_bytes_len,
        );
        
        let mut handle_out = CONTROLTRACE_HANDLE::default();
        let _ = ControlTraceW(CONTROLTRACE_HANDLE::default(), windows::core::PCWSTR(session_name_w.as_ptr()), props, EVENT_TRACE_CONTROL_STOP);
        
        let mut status = StartTraceW(&mut handle_out, windows::core::PCWSTR(session_name_w.as_ptr()), props);
        if status.is_err() && status.0 as u32 == 183 {
            let _ = ControlTraceW(CONTROLTRACE_HANDLE::default(), windows::core::PCWSTR(session_name_w.as_ptr()), props, EVENT_TRACE_CONTROL_STOP);
            status = StartTraceW(&mut handle_out, windows::core::PCWSTR(session_name_w.as_ptr()), props);
        }

        if status.is_err() || handle_out.Value == 0 {
            log_etw_status(&format!("StartTraceW failed: status={:?}, handle=0x{:x}", status, handle_out.Value));
            ETW_ACTIVE.store(false, Ordering::Relaxed);
            return;
        }

        log_etw_status(&format!("StartTraceW OK! Handle: 0x{:x}", handle_out.Value));

        let enable_res = EnableTraceEx2(
            handle_out,
            &KERNEL_NETWORK_PROVIDER_GUID,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
            5, // TRACE_LEVEL_VERBOSE
            u64::MAX, // MatchAnyKeyword: Enable all keywords for network events!
            0,
            0,
            None,
        );

        if enable_res.is_err() {
            log_etw_status(&format!("EnableTraceEx2 failed: {:?}", enable_res));
        } else {
            log_etw_status("EnableTraceEx2 OK! Attached Kernel Network Provider");
        }

        let mut logfile = EVENT_TRACE_LOGFILEW::default();
        logfile.LoggerName = windows::core::PWSTR(session_name_w.as_ptr() as *mut _);
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(etw_event_callback);

        let trace_handle = OpenTraceW(&mut logfile);
        if trace_handle.Value == 0 || trace_handle.Value == u64::MAX {
            log_etw_status(&format!("OpenTraceW failed: handle=0x{:x}", trace_handle.Value));
            let _ = ControlTraceW(handle_out, windows::core::PCWSTR(session_name_w.as_ptr()), props, EVENT_TRACE_CONTROL_STOP);
            ETW_ACTIVE.store(false, Ordering::Relaxed);
            return;
        }

        log_etw_status(&format!("OpenTraceW OK! TraceHandle: 0x{:x}. Listening for packets...", trace_handle.Value));

        let handles = [trace_handle];
        while (get_telemetry_engine_mode() == 2) && is_elevated() {
            let proc_res = ProcessTrace(&handles, None, None);
            if proc_res.is_err() {
                log_etw_status(&format!("ProcessTrace exited with status: {:?}", proc_res));
                break;
            }
        }
        let _ = CloseTrace(trace_handle);
        
        let _ = ControlTraceW(handle_out, windows::core::PCWSTR(session_name_w.as_ptr()), props, EVENT_TRACE_CONTROL_STOP);
        log_etw_status("ETW Session stopped cleanly");
        ETW_ACTIVE.store(false, Ordering::Relaxed);
    }
}

unsafe extern "system" fn etw_event_callback(event_record: *mut EVENT_RECORD) {
    if event_record.is_null() {
        return;
    }
    ETW_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);

    let record = &*event_record;

    // Check payload data buffer validity
    if record.UserDataLength < 8 || record.UserData.is_null() {
        return;
    }

    let user_data = record.UserData as *const u8;

    // In Windows Kernel Network ETW events, incoming packet events fire in DPC interrupt context,
    // so record.EventHeader.ProcessId is 0 or 4 (System/Idle).
    // The actual socket-owning process ID is stored in the first 4 bytes of the UserData payload!
    let mut pid = u32::from_ne_bytes(*(user_data as *const [u8; 4]));
    if pid == 0 {
        pid = record.EventHeader.ProcessId;
    }
    if pid == 0 {
        return;
    }

    // Extract packet transfer byte size from offset 4 (4 bytes) or fallback to UserDataLength
    let packet_bytes = u32::from_ne_bytes(*(user_data.add(4) as *const [u8; 4])) as u64;
    let bytes = if packet_bytes > 0 && packet_bytes <= 65535 {
        packet_bytes
    } else {
        record.UserDataLength as u64
    };

    let id = record.EventHeader.EventDescriptor.Id;
    // RX Event IDs: 11 (TCP v4 Recv), 13 (TCP v4 Retransmit), 15 (TCP v6 Recv), 43 (UDP v4 Recv), 45 (UDP v6 Recv)
    let is_rx = id == 11 || id == 13 || id == 15 || id == 43 || id == 45;
    // TX Event IDs: 10 (TCP v4 Send), 12 (TCP v4 Disconnect), 14 (TCP v6 Send), 42 (UDP v4 Send), 44 (UDP v6 Send)
    let is_tx = id == 10 || id == 12 || id == 14 || id == 42 || id == 44;

    if (is_rx || is_tx) && bytes > 0 {
        ETW_BYTE_COUNT.fetch_add(bytes, Ordering::Relaxed);
        if let Ok(mut map) = ETW_PID_STATS.lock() {
            let entry = map.entry(pid).or_default();
            if is_rx {
                entry.rx_bytes = entry.rx_bytes.saturating_add(bytes);
            } else {
                entry.tx_bytes = entry.tx_bytes.saturating_add(bytes);
            }
        }
    }
}

pub fn drain_etw_deltas() -> (HashMap<u32, u64>, HashMap<u32, u64>) {
    let mut rx_map = HashMap::new();
    let mut tx_map = HashMap::new();

    if let Ok(mut map) = ETW_PID_STATS.lock() {
        for (pid, metrics) in map.drain() {
            if metrics.rx_bytes > 0 {
                rx_map.insert(pid, metrics.rx_bytes);
            }
            if metrics.tx_bytes > 0 {
                tx_map.insert(pid, metrics.tx_bytes);
            }
        }
    }
    (rx_map, tx_map)
}

/// Return the executable filename of the current foreground window's process.
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
        get_process_name_for_pid(pid)
    }
}

/// Return how many milliseconds since the user last provided keyboard/mouse input.
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

/// Resolve a PID to its executable filename. Returns "System" on failure
/// (e.g. for kernel/protected processes that deny PROCESS_QUERY_LIMITED_INFORMATION).
fn get_process_name_for_pid(pid: u32) -> String {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        if let Ok(handle) = handle {
            let mut buffer = [0u16; 260];
            let len = GetModuleFileNameExW(handle, None, &mut buffer);
            CloseHandle(handle).ok();
            if len > 0 {
                let path = String::from_utf16_lossy(&buffer[..len as usize]);
                if let Some(filename) = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|f| f.to_str())
                {
                    return filename.to_string();
                }
            }
        }

        // Fallback: Use Toolhelp32Snapshot for sandboxed / low-integrity child processes
        // (e.g. Firefox content/socket processes) where OpenProcess returns Access Denied.
        if let Ok(handle) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            if !handle.is_invalid() {
                let mut entry = PROCESSENTRY32W::default();
                entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                if Process32FirstW(handle, &mut entry).is_ok() {
                    loop {
                        if entry.th32ProcessID == pid {
                            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                            let _ = CloseHandle(handle);
                            if !name.is_empty() {
                                return name;
                            }
                            break;
                        }
                        if Process32NextW(handle, &mut entry).is_err() {
                            break;
                        }
                    }
                }
                let _ = CloseHandle(handle);
            }
        }
    }
    "System".to_string()
}

/// Return cumulative (rx_bytes, tx_bytes) for the best internet-facing NIC.
///
/// Uses GetBestInterface(1.1.1.1) to identify the routing adapter, then reads
/// its InOctets / OutOctets from GetIfTable2. Falls back to filtering by
/// OperStatus + adapter type (Ethernet / WiFi / PPP) when routing lookup fails.
///
/// NOTE: These are *lifetime* counters that reset when the adapter reconnects.
///       The caller must detect counter resets (current < previous) and skip
///       the affected tick to avoid spurious spikes.
fn get_total_network_octets() -> (u64, u64) {
    let mut rx = 0u64;
    let mut tx = 0u64;
    unsafe {
        // Identify the NIC that routes to the public internet
        let mut best_if_index = 0u32;
        let mut target_index: Option<u32> = None;
        // 0x01010101 = 1.1.1.1 in big-endian u32
        if GetBestInterface(0x01010101, &mut best_if_index) == 0 {
            target_index = Some(best_if_index);
        }

        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_ok() {
            let slice = std::slice::from_raw_parts(
                (*table).Table.as_ptr(),
                (*table).NumEntries as usize,
            );
            for row in slice {
                let is_match = match target_index {
                    Some(idx) => row.InterfaceIndex == idx,
                    // Fallback: active physical adapters only (Ethernet=6, WiFi=71, PPP=23, WWAN=244)
                    None => {
                        row.OperStatus.0 == 1
                            && (row.Type == 6
                                || row.Type == 71
                                || row.Type == 244
                                || row.Type == 23)
                    }
                };
                if is_match {
                    rx += row.InOctets;
                    tx += row.OutOctets;
                }
            }
            FreeMibTable(table as *const c_void);
        }
    }
    (rx, tx)
}

/// Return (battery_percent, is_charging).
fn get_battery_info() -> (u8, bool) {
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_ok() {
            let percent = if status.BatteryLifePercent == 255 {
                100
            } else {
                status.BatteryLifePercent
            };
            let charging = (status.BatteryFlag & 8) != 0 || status.ACLineStatus == 1;
            return (percent, charging);
        }
    }
    (100, true)
}

/// Measure TCP round-trip time to 1.1.1.1:80 (Cloudflare). Returns 0 on failure.
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

/// Snapshot all currently active IPv4 TCP connections (ESTABLISHED / CLOSE_WAIT)
/// with their owning PIDs and cumulative payload byte counters.
///
/// ## How it works
/// 1. `GetExtendedTcpTable(TCP_TABLE_OWNER_PID_CONNECTIONS)` enumerates every
///    non-listening TCP connection and its owning PID.
/// 2. `SetPerTcpConnectionEStats` (called once per new connection) arms the
///    kernel's per-connection statistics collector.
/// 3. `GetPerTcpConnectionEStats(TcpConnectionEstatsData)` reads the cumulative
///    `DataBytesIn` / `DataBytesOut` for each connection.
///
/// ## Limitations
/// - Payload bytes only (excludes IP+TCP headers, ~20-60 bytes per packet).
///   The sum of TCP payload across all connections will therefore be slightly
///   less than the NIC InOctets/OutOctets. The caller clamps accordingly.
/// - UDP traffic (DNS, QUIC, streaming, VoIP) is NOT captured here and will
///   appear in the "System" remainder bucket.
/// - System-owned connections (e.g. lsass, svchost with elevated ACL) may
///   fail SetPerTcpConnectionEStats and return 0 bytes; they roll into "System".
///
/// `stats_enabled`: caller-owned set of connection keys whose EStats have
/// already been armed. New keys not present here get SetPerTcpConnectionEStats
/// called before the first GetPerTcpConnectionEStats read.
fn snapshot_tcp_connections(
    stats_enabled: &mut HashSet<TcpConnKey>,
) -> HashMap<TcpConnKey, TcpConnSnapshot> {
    let mut snapshots: HashMap<TcpConnKey, TcpConnSnapshot> = HashMap::new();

    unsafe {
        // ── Two-pass buffer sizing ──────────────────────────────────────────
        // First call with a null buffer to determine the required size.
        let mut buf_size = 0u32;
        GetExtendedTcpTable(
            None,
            &mut buf_size,
            false,
            2u32, // AF_INET = 2
            TCP_TABLE_OWNER_PID_CONNECTIONS,
            0,
        );

        if buf_size == 0 {
            return snapshots; // No active TCP connections
        }

        // Add headroom for connections established between the two calls.
        let mut buf = vec![0u8; buf_size as usize + 512];

        // Second call — with retry on ERROR_INSUFFICIENT_BUFFER (new connections
        // may have appeared since the first call updated buf_size).
        let fill_err = loop {
            let err = GetExtendedTcpTable(
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut buf_size,
                false,
                2u32,
                TCP_TABLE_OWNER_PID_CONNECTIONS,
                0,
            );
            if err == ERROR_INSUFFICIENT_BUFFER {
                buf.resize(buf_size as usize + 512, 0);
            } else {
                break err;
            }
        };

        if fill_err != 0 {
            return snapshots; // API failed — return empty map
        }

        // ── Parse the MIB_TCPTABLE_OWNER_PID ──────────────────────────────
        // Safety: buffer was sized by the first GetExtendedTcpTable call.
        // Guard dwNumEntries against pathological values before slicing.
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let num_entries = table.dwNumEntries as usize;
        if num_entries == 0 || num_entries > 65_536 {
            return snapshots; // Zero entries or suspicious count — bail
        }

        let rows: &[MIB_TCPROW_OWNER_PID] = std::slice::from_raw_parts(
            table.table.as_ptr(),
            num_entries,
        );

        for entry in rows {
            let key = TcpConnKey {
                local_addr: entry.dwLocalAddr,
                local_port: entry.dwLocalPort,
                remote_addr: entry.dwRemoteAddr,
                remote_port: entry.dwRemotePort,
            };

            // MIB_TCPROW_LH (no PID field) is required by the EStats APIs.
            // Its `Anonymous` field is a union — we build via zeroed + transmute
            // to safely copy the raw dwState bytes without touching union internals.
            let mut tcp_row: MIB_TCPROW_LH = std::mem::zeroed();
            // The first 4 bytes of MIB_TCPROW_LH are the dwState union — write raw.
            let state_bytes = entry.dwState.to_ne_bytes();
            let row_ptr = &mut tcp_row as *mut MIB_TCPROW_LH as *mut u8;
            std::ptr::copy_nonoverlapping(state_bytes.as_ptr(), row_ptr, 4);
            tcp_row.dwLocalAddr  = entry.dwLocalAddr;
            tcp_row.dwLocalPort  = entry.dwLocalPort;
            tcp_row.dwRemoteAddr = entry.dwRemoteAddr;
            tcp_row.dwRemotePort = entry.dwRemotePort;

            // Arm EStats collection for connections we haven't seen before.
            // This may silently fail for system/privileged connections — that is
            // acceptable; their bytes will fall through to the System remainder.
            if !stats_enabled.contains(&key) {
                let rw = TCP_ESTATS_DATA_RW_v0 {
                    EnableCollection: BOOLEAN(1),
                };
                // Safety: rw_bytes is a valid byte slice over a stack-allocated struct
                let rw_bytes = std::slice::from_raw_parts(
                    &rw as *const TCP_ESTATS_DATA_RW_v0 as *const u8,
                    std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>(),
                );
                // Ignore return value — failure is normal for privileged connections
                let _ = SetPerTcpConnectionEStats(
                    &tcp_row,
                    TcpConnectionEstatsData,
                    rw_bytes,
                    0, // rwversion
                    0, // offset
                );
                stats_enabled.insert(key.clone());
            }

            // Read cumulative byte counters for this connection.
            // DataBytesIn  = total TCP payload bytes received since connection start
            // DataBytesOut = total TCP payload bytes sent     since connection start
            let mut rod: TCP_ESTATS_DATA_ROD_v0 = std::mem::zeroed();
            // Safety: rod_bytes is a valid mutable byte slice over a stack-allocated struct
            let rod_bytes = std::slice::from_raw_parts_mut(
                &mut rod as *mut TCP_ESTATS_DATA_ROD_v0 as *mut u8,
                std::mem::size_of::<TCP_ESTATS_DATA_ROD_v0>(),
            );
            let estat_err = GetPerTcpConnectionEStats(
                &tcp_row,
                TcpConnectionEstatsData,
                None,           // rw output — not needed for reading counters
                0,              // rwversion
                None,           // ros output — not needed
                0,              // rosversion
                Some(rod_bytes),
                0,              // rodversion
            );

            if estat_err == 0 {
                // ERROR_SUCCESS — store this connection's cumulative counters
                snapshots.insert(
                    key,
                    TcpConnSnapshot {
                        pid: entry.dwOwningPid,
                        bytes_in: rod.DataBytesIn,
                        bytes_out: rod.DataBytesOut,
                    },
                );
            }
            // On failure (e.g. ERROR_NOT_FOUND for untracked/privileged conn),
            // omit the connection — it flows into the System remainder.
        }

        // Prune dead connections from the EStats-enabled tracking set
        stats_enabled.retain(|k| snapshots.contains_key(k));
    }

    snapshots
}

// ---------------------------------------------------------------------------
// TelemetryService
// ---------------------------------------------------------------------------

pub struct TelemetryService {
    realtime_sender: broadcast::Sender<RealtimeStats>,
    db_path: String,
}

impl TelemetryService {
    pub fn new(db_path: String) -> (Self, broadcast::Receiver<RealtimeStats>) {
        let (tx, rx) = broadcast::channel(100);
        (TelemetryService { realtime_sender: tx, db_path }, rx)
    }

    pub fn start(&self) {
        let tx = self.realtime_sender.clone();
        let db_path = self.db_path.clone();

        std::thread::spawn(move || {
            // ── Per-loop state ───────────────────────────────────────────────
            let mut last_octets = get_total_network_octets();
            let mut last_poll = Instant::now();
            let mut last_db_flush = Instant::now();
            let mut tick_count: u32 = 0;
            let mut current_ping_ms: u32 = 0;

            // TCP connection byte-count tracking
            let mut last_tcp_snapshots: HashMap<TcpConnKey, TcpConnSnapshot> = HashMap::new();
            let mut estats_enabled: HashSet<TcpConnKey> = HashSet::new();

            // Process I/O byte-count tracking (Legacy IO engine)
            let mut last_io_snapshots: HashMap<u32, ProcessIoSnapshot> = HashMap::new();

            // PID → exe name cache (cleared every DB flush to evict reused PIDs)
            let mut process_name_cache: HashMap<u32, String> = HashMap::new();

            // Per-process byte accumulator (flushed to SQLite every 60 s)
            let mut accumulator: HashMap<String, ProcessStatsAccumulator> = HashMap::new();

            loop {
                std::thread::sleep(Duration::from_millis(1000));

                let now = Instant::now();
                let duration = now.duration_since(last_poll);
                last_poll = now;
                let secs = duration.as_secs_f64().max(0.001); // guard against division by zero

                // ── 1. NIC byte counters ─────────────────────────────────────
                let current_octets = get_total_network_octets();

                // Detect NIC counter reset (WiFi reconnect, adapter restart).
                // When the adapter reinitialises, InOctets/OutOctets reset to 0,
                // making current < previous. Skip this tick to avoid a false spike
                // and clear TCP state since the adapter context has changed.
                if current_octets.0 < last_octets.0 || current_octets.1 < last_octets.1 {
                    last_octets = current_octets;
                    last_tcp_snapshots.clear();
                    estats_enabled.clear();
                    continue;
                }

                let rx_diff = current_octets.0 - last_octets.0;
                let tx_diff = current_octets.1 - last_octets.1;
                last_octets = current_octets;

                let download_speed = (rx_diff as f64 / secs) as u64;
                let upload_speed   = (tx_diff as f64 / secs) as u64;

                // Spike guard: anything above 1.5 GB/s (12 Gbps) is a measurement
                // artifact, not a real internet speed. Discard and wait for next tick.
                if download_speed > MAX_REALISTIC_BPS || upload_speed > MAX_REALISTIC_BPS {
                    continue;
                }

            // ── 2. Per-process network attribution ──────────────────────
            let engine_mode = get_telemetry_engine_mode();

            let (pid_rx_delta, pid_tx_delta) = if engine_mode == 2 && is_elevated() {
                // Engine 2: Elevated ETW Kernel Tracing (100% TCP + UDP)
                ensure_etw_tracer_running();
                drain_etw_deltas()
            } else if engine_mode == 0 {
                // Engine 0: Process I/O Counters (Legacy commit 7303ecd method)
                let current_io = snapshot_process_io_counters();
                let mut rx_map = HashMap::new();
                let mut tx_map = HashMap::new();

                for (pid, io) in &current_io {
                    if let Some(prev) = last_io_snapshots.get(pid) {
                        let rd = io.read_bytes.saturating_sub(prev.read_bytes).min(MAX_REALISTIC_BPS);
                        let td = io.write_bytes.saturating_sub(prev.write_bytes).min(MAX_REALISTIC_BPS);

                        if rd > 0 { rx_map.insert(*pid, rd); }
                        if td > 0 { tx_map.insert(*pid, td); }
                    }
                }
                last_io_snapshots = current_io;
                (rx_map, tx_map)
            } else {
                // Engine 1: TCP EStats Snapshotting (User-mode, default)
                let current_tcp_snapshots = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    snapshot_tcp_connections(&mut estats_enabled)
                })).unwrap_or_else(|_| {
                    estats_enabled.clear();
                    last_tcp_snapshots.clear();
                    HashMap::new()
                });

                let mut rx_map: HashMap<u32, u64> = HashMap::new();
                let mut tx_map: HashMap<u32, u64> = HashMap::new();

                for (key, snap) in &current_tcp_snapshots {
                    if let Some(prev) = last_tcp_snapshots.get(key) {
                        let rd = snap.bytes_in.saturating_sub(prev.bytes_in).min(MAX_REALISTIC_BPS);
                        let td = snap.bytes_out.saturating_sub(prev.bytes_out).min(MAX_REALISTIC_BPS);

                        if rd > 0 {
                            *rx_map.entry(snap.pid).or_insert(0) =
                                rx_map.get(&snap.pid).copied().unwrap_or(0).saturating_add(rd);
                        }
                        if td > 0 {
                            *tx_map.entry(snap.pid).or_insert(0) =
                                tx_map.get(&snap.pid).copied().unwrap_or(0).saturating_add(td);
                        }
                    }
                }
                last_tcp_snapshots = current_tcp_snapshots;
                (rx_map, tx_map)
            };

            let total_tcp_rx: u64 = pid_rx_delta.values().sum();
            let total_tcp_tx: u64 = pid_tx_delta.values().sum();

                // Proportional clamp: scale down TCP deltas if they somehow exceed
                // the NIC byte delta (can happen with retransmitted segments or
                // sub-millisecond timing jitter between the two API calls).
                let rx_scale = if total_tcp_rx > rx_diff && total_tcp_rx > 0 {
                    rx_diff as f64 / total_tcp_rx as f64
                } else {
                    1.0
                };
                let tx_scale = if total_tcp_tx > tx_diff && total_tcp_tx > 0 {
                    tx_diff as f64 / total_tcp_tx as f64
                } else {
                    1.0
                };

                let mut process_speeds: Vec<ProcessSpeed> = Vec::new();
                let mut attributed_rx: u64 = 0;
                let mut attributed_tx: u64 = 0;

                // Union of all PIDs that have any RX or TX delta this tick
                let all_pids: HashSet<u32> = pid_rx_delta
                    .keys()
                    .chain(pid_tx_delta.keys())
                    .copied()
                    .collect();

                for pid in all_pids {
                    let proc_rx_raw = pid_rx_delta.get(&pid).copied().unwrap_or(0);
                    let proc_tx_raw = pid_tx_delta.get(&pid).copied().unwrap_or(0);

                    // Apply proportional clamp scaling
                    let proc_rx = (proc_rx_raw as f64 * rx_scale) as u64;
                    let proc_tx = (proc_tx_raw as f64 * tx_scale) as u64;

                    if proc_rx == 0 && proc_tx == 0 {
                        continue;
                    }

                    let down_speed = (proc_rx as f64 / secs) as u64;
                    let up_speed   = (proc_tx as f64 / secs) as u64;

                    // Resolve PID → process name (cached to avoid per-tick WinAPI call)
                    let name = process_name_cache
                        .entry(pid)
                        .or_insert_with(|| get_process_name_for_pid(pid))
                        .clone();

                    // Accumulate raw bytes (NOT derived speed) for accurate data totals.
                    // Using rx_diff bytes avoids the drift that occurs when adding a
                    // per-second rate value each tick regardless of actual elapsed time.
                    let entry = accumulator.entry(name.clone()).or_insert(ProcessStatsAccumulator {
                        bytes_downloaded: 0,
                        bytes_uploaded: 0,
                        screen_time_seconds: 0,
                    });
                    entry.bytes_downloaded += proc_rx;
                    entry.bytes_uploaded   += proc_tx;

                    attributed_rx = attributed_rx.saturating_add(proc_rx);
                    attributed_tx = attributed_tx.saturating_add(proc_tx);

                    process_speeds.push(ProcessSpeed {
                        name,
                        download_speed: down_speed,
                        upload_speed: up_speed,
                    });
                }

                // Remainder: NIC bytes not accounted for by any TCP process.
                // Covers UDP (DNS, QUIC/HTTP3, streaming, VoIP), ICMP, raw sockets,
                // and system-privileged TCP connections where EStats collection failed.
                let remainder_rx = rx_diff.saturating_sub(attributed_rx);
                let remainder_tx = tx_diff.saturating_sub(attributed_tx);

                if remainder_rx > 0 || remainder_tx > 0 {
                    let sys_down = (remainder_rx as f64 / secs) as u64;
                    let sys_up   = (remainder_tx as f64 / secs) as u64;
                    if sys_down > 0 || sys_up > 0 {
                        let entry = accumulator
                            .entry("System".to_string())
                            .or_insert(ProcessStatsAccumulator {
                                bytes_downloaded: 0,
                                bytes_uploaded: 0,
                                screen_time_seconds: 0,
                            });
                        entry.bytes_downloaded += remainder_rx;
                        entry.bytes_uploaded   += remainder_tx;
                        process_speeds.push(ProcessSpeed {
                            name: "System".to_string(),
                            download_speed: sys_down,
                            upload_speed: sys_up,
                        });
                    }
                }

                // Populate live telemetry debug info
                let etw_events_sec = ETW_EVENT_COUNT.swap(0, Ordering::Relaxed);
                let etw_bytes_sec = ETW_BYTE_COUNT.swap(0, Ordering::Relaxed);

                let mut debug_samples = Vec::new();
                for (pid, proc_rx_raw) in &pid_rx_delta {
                    let proc_tx_raw = pid_tx_delta.get(pid).copied().unwrap_or(0);
                    let exe = process_name_cache.get(pid).cloned().unwrap_or_else(|| "Unknown".to_string());
                    debug_samples.push((*pid, exe, *proc_rx_raw, proc_tx_raw));
                }

                if let Ok(mut guard) = LAST_DEBUG_INFO.lock() {
                    guard.engine_mode = engine_mode;
                    guard.engine_name = match engine_mode {
                        0 => "Process I/O",
                        2 => "Kernel ETW Tracing",
                        _ => "TCP EStats",
                    }.to_string();
                    guard.is_elevated = is_elevated();
                    guard.etw_active = ETW_ACTIVE.load(Ordering::Relaxed);
                    guard.etw_events_last_sec = etw_events_sec;
                    guard.etw_bytes_last_sec = etw_bytes_sec;
                    guard.nic_rx_bytes_last_sec = rx_diff;
                    guard.nic_tx_bytes_last_sec = tx_diff;
                    guard.active_etw_pids = pid_rx_delta.len();
                    guard.raw_etw_pid_samples = debug_samples;
                    guard.etw_status_log = ETW_STATUS_LOG.lock().map(|s| s.clone()).unwrap_or_default();
                }

                // ── 3. Active foreground app & screen-time tracking ──────────
                let active_app = get_active_process_name();
                let idle_ms    = get_idle_time_millis();
                let is_idle    = idle_ms > (5 * 60 * 1_000);
                let active_secs: u32 = if is_idle || active_app == "Idle" { 0 } else { 1 };

                accumulator
                    .entry(active_app.clone())
                    .or_insert(ProcessStatsAccumulator {
                        bytes_downloaded: 0,
                        bytes_uploaded: 0,
                        screen_time_seconds: 0,
                    })
                    .screen_time_seconds += active_secs;

                // ── 4. Battery info ──────────────────────────────────────────
                let (battery_pct, is_charging) = get_battery_info();

                // ── 5. Ping latency (every 5 ticks ≈ 5 seconds) ─────────────
                tick_count = tick_count.wrapping_add(1);
                if tick_count % 5 == 1 {
                    current_ping_ms = measure_ping();
                }

                // ── 6. Broadcast real-time stats to Svelte frontend ──────────
                let stats = RealtimeStats {
                    download_speed,
                    upload_speed,
                    battery_percentage: battery_pct,
                    is_charging,
                    active_app,
                    ping_ms: current_ping_ms,
                    process_speeds,
                };
                let _ = tx.send(stats);

                // ── 7. Flush accumulated bytes to SQLite every 60 seconds ────
                if now.duration_since(last_db_flush) >= Duration::from_secs(60) {
                    last_db_flush = now;

                    // Clear PID→name cache every flush cycle to evict reused PIDs.
                    // A process that exited and whose PID was reused would otherwise
                    // accumulate bytes under the wrong name indefinitely.
                    process_name_cache.clear();

                    if let Ok(conn) = crate::db::open_conn(&db_path) {
                        let epoch_now = Local::now().timestamp();
                        // Round to nearest 5-minute bucket for aggregation alignment
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
                        let _ = crate::db::aggregate_data(&conn);
                    }
                }
            } // loop
        });
    }
}
