use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use chrono::Local;

use windows::Win32::Foundation::{CloseHandle, BOOLEAN};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
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

            // ── 2. Per-process TCP attribution ───────────────────────────
            //
            // snapshot_tcp_connections uses unsafe Windows APIs. We wrap it in
            // catch_unwind so that any access violation or panic in the unsafe
            // block cannot kill the telemetry thread — NIC speed always flows.
            let current_tcp_snapshots = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                snapshot_tcp_connections(&mut estats_enabled)
            })).unwrap_or_else(|_| {
                // If the TCP snapshot panicked, reset tracking state so the next
                // tick starts fresh rather than using stale/corrupt data.
                estats_enabled.clear();
                last_tcp_snapshots.clear();
                HashMap::new()
            });

                let mut pid_rx_delta: HashMap<u32, u64> = HashMap::new();
                let mut pid_tx_delta: HashMap<u32, u64> = HashMap::new();
                let mut total_tcp_rx: u64 = 0;
                let mut total_tcp_tx: u64 = 0;

                for (key, snap) in &current_tcp_snapshots {
                    if let Some(prev) = last_tcp_snapshots.get(key) {
                        // Saturating subtraction handles counter wraps / key reuse.
                        let rd = snap.bytes_in.saturating_sub(prev.bytes_in);
                        let td = snap.bytes_out.saturating_sub(prev.bytes_out);

                        // Guard: if EStats collection wasn't armed (SetPerTcpConnectionEStats
                        // failed for a privileged process), GetPerTcpConnectionEStats may
                        // still return ERROR_SUCCESS with garbage in DataBytesIn/Out (e.g.
                        // 0xFFFFFFFFFFFFFFFF). Cap each per-connection delta at the same
                        // ceiling used for NIC spikes — no real connection can exceed 1.5 GB/s.
                        let rd = rd.min(MAX_REALISTIC_BPS);
                        let td = td.min(MAX_REALISTIC_BPS);

                        if rd > 0 {
                            *pid_rx_delta.entry(snap.pid).or_insert(0) =
                                pid_rx_delta.get(&snap.pid).copied().unwrap_or(0).saturating_add(rd);
                            total_tcp_rx = total_tcp_rx.saturating_add(rd);
                        }
                        if td > 0 {
                            *pid_tx_delta.entry(snap.pid).or_insert(0) =
                                pid_tx_delta.get(&snap.pid).copied().unwrap_or(0).saturating_add(td);
                            total_tcp_tx = total_tcp_tx.saturating_add(td);
                        }
                    }
                    // New connections (not in last_tcp_snapshots) produce no delta
                    // this tick — they will contribute from the next tick onward.
                }

                last_tcp_snapshots = current_tcp_snapshots;

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
