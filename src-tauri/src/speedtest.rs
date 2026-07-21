use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use futures_util::StreamExt;

static TEST_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize, Clone, Debug)]
pub struct DnsPingResult {
    pub name: String,
    pub ip: String,
    pub latency_ms: u32,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct SpeedTestProgress {
    pub stage: String, // "ping", "download", "upload", "complete", "error"
    pub current_speed: u64, // Bytes per second
    pub progress_percent: u32, // 0 - 100
    pub pings: Vec<DnsPingResult>,
    pub download_speed: u64, // Final bytes per sec
    pub upload_speed: u64,   // Final bytes per sec
    pub average_ping: u32,
    pub message: String,
}

// Measure TCP latency to a specific target host:port
async fn measure_tcp_latency(host: &str, port: u16, samples: usize) -> u32 {
    let mut total_ms = 0u64;
    let mut successful_samples = 0u64;
    let addr = format!("{}:{}", host, port);

    for _ in 0..samples {
        let start = Instant::now();
        let timeout = Duration::from_millis(2000);
        if let Ok(Ok(_stream)) = tokio::time::timeout(timeout, TcpStream::connect(&addr)).await {
            let elapsed = start.elapsed().as_millis() as u64;
            total_ms += elapsed;
            successful_samples += 1;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    if successful_samples > 0 {
        (total_ms / successful_samples) as u32
    } else {
        999 // Offline or filtered
    }
}

// Perform multi-DNS ping benchmark
async fn run_multi_dns_ping() -> (Vec<DnsPingResult>, u32) {
    let targets = vec![
        ("Cloudflare", "1.1.1.1", 53),
        ("Google DNS", "8.8.8.8", 53),
        ("Quad9 DNS", "9.9.9.9", 53),
        ("OpenDNS", "208.67.222.222", 53),
    ];

    let mut results = Vec::new();
    let mut valid_pings = Vec::new();

    for (name, ip, port) in targets {
        let latency = measure_tcp_latency(ip, port, 3).await;
        results.push(DnsPingResult {
            name: name.to_string(),
            ip: ip.to_string(),
            latency_ms: latency,
        });
        if latency < 999 {
            valid_pings.push(latency);
        }
    }

    let avg_ping = if !valid_pings.is_empty() {
        (valid_pings.iter().sum::<u32>() as usize / valid_pings.len()) as u32
    } else {
        0
    };

    (results, avg_ping)
}

// Calculate Trimmed Mean (Ookla / Cloudflare Standard)
// Removes top 15% (transient burst flushes) and bottom 20% (ramp-up residuals)
fn calculate_trimmed_mean(mut samples: Vec<u64>) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    if samples.len() < 5 {
        return samples.iter().sum::<u64>() / samples.len() as u64;
    }

    samples.sort_unstable();
    let len = samples.len();
    let drop_bottom = (len as f32 * 0.20) as usize;
    let drop_top = (len as f32 * 0.15) as usize;

    let valid_slice = &samples[drop_bottom..(len - drop_top)];
    if valid_slice.is_empty() {
        return samples[samples.len() / 2];
    }

    let sum: u64 = valid_slice.iter().sum();
    sum / valid_slice.len() as u64
}

// Parallel Download Speed Test with Exponential Moving Average (EMA) smoothing
async fn execute_download_test<F>(mut progress_cb: F) -> u64
where
    F: FnMut(u64, u32),
{
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let total_bytes_downloaded = Arc::new(AtomicU64::new(0));
    let stop_signal = Arc::new(AtomicBool::new(false));
    let num_streams = 4;
    let mut handles = Vec::new();

    for _ in 0..num_streams {
        let client_clone = client.clone();
        let bytes_counter = Arc::clone(&total_bytes_downloaded);
        let stop_flag = Arc::clone(&stop_signal);

        let handle = tokio::spawn(async move {
            let url = "https://speed.cloudflare.com/__down?bytes=50000000";
            while !stop_flag.load(Ordering::Relaxed) {
                if let Ok(res) = client_clone.get(url).send().await {
                    let mut stream = res.bytes_stream();
                    while let Some(chunk_res) = stream.next().await {
                        if stop_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Ok(chunk) = chunk_res {
                            bytes_counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                        } else {
                            break;
                        }
                    }
                } else {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            }
        });
        handles.push(handle);
    }

    let warmup_duration = Duration::from_millis(1500);
    let test_duration = Duration::from_secs(8);
    let start_time = Instant::now();
    let mut last_sample_time = Instant::now();
    let mut last_total_bytes = 0u64;
    let mut steady_state_samples = Vec::new();
    let mut ema_speed: f64 = 0.0;
    let alpha = 0.35; // Smoothing factor

    while start_time.elapsed() < test_duration {
        tokio::time::sleep(Duration::from_millis(120)).await;

        let current_total = total_bytes_downloaded.load(Ordering::Relaxed);
        let elapsed = last_sample_time.elapsed().as_secs_f64();

        if elapsed >= 0.10 {
            let inst_bytes = current_total.saturating_sub(last_total_bytes);
            let raw_inst_speed = (inst_bytes as f64 / elapsed) as f64;

            if ema_speed == 0.0 {
                ema_speed = raw_inst_speed;
            } else {
                ema_speed = (alpha * raw_inst_speed) + ((1.0 - alpha) * ema_speed);
            }

            let smooth_speed_u64 = ema_speed as u64;

            // Only collect samples for final score after Warmup phase (1.5s)
            if start_time.elapsed() >= warmup_duration && smooth_speed_u64 > 0 {
                steady_state_samples.push(smooth_speed_u64);
            }

            let pct = ((start_time.elapsed().as_secs_f64() / test_duration.as_secs_f64()) * 100.0).min(100.0) as u32;
            progress_cb(smooth_speed_u64, pct);

            last_sample_time = Instant::now();
            last_total_bytes = current_total;
        }
    }

    stop_signal.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.await;
    }

    calculate_trimmed_mean(steady_state_samples)
}

// Parallel Upload Speed Test with EMA smoothing to prevent needle jitter
async fn execute_upload_test<F>(mut progress_cb: F) -> u64
where
    F: FnMut(u64, u32),
{
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let total_bytes_uploaded = Arc::new(AtomicU64::new(0));
    let stop_signal = Arc::new(AtomicBool::new(false));
    let num_streams = 3;
    let mut handles = Vec::new();

    for _ in 0..num_streams {
        let client_clone = client.clone();
        let bytes_counter = Arc::clone(&total_bytes_uploaded);
        let stop_flag = Arc::clone(&stop_signal);

        let handle = tokio::spawn(async move {
            let url = "https://speed.cloudflare.com/__up";
            let payload = vec![0u8; 1_048_576]; // 1 MB payload buffer for smooth TCP pipeline

            while !stop_flag.load(Ordering::Relaxed) {
                let bytes_counter_inner = Arc::clone(&bytes_counter);
                let payload_len = payload.len() as u64;

                let req = client_clone.post(url)
                    .header("Content-Type", "application/octet-stream")
                    .body(payload.clone());

                if req.send().await.is_ok() {
                    bytes_counter_inner.fetch_add(payload_len, Ordering::Relaxed);
                } else {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        });
        handles.push(handle);
    }

    let warmup_duration = Duration::from_millis(1500);
    let test_duration = Duration::from_secs(7);
    let start_time = Instant::now();
    let mut last_sample_time = Instant::now();
    let mut last_total_bytes = 0u64;
    let mut steady_state_samples = Vec::new();
    let mut ema_speed: f64 = 0.0;
    let alpha = 0.30; // Strong smoothing factor to absorb TCP ACK batching spikes

    while start_time.elapsed() < test_duration {
        tokio::time::sleep(Duration::from_millis(120)).await;

        let current_total = total_bytes_uploaded.load(Ordering::Relaxed);
        let elapsed = last_sample_time.elapsed().as_secs_f64();

        if elapsed >= 0.10 {
            let inst_bytes = current_total.saturating_sub(last_total_bytes);
            let raw_inst_speed = (inst_bytes as f64 / elapsed) as f64;

            if ema_speed == 0.0 {
                ema_speed = raw_inst_speed;
            } else {
                ema_speed = (alpha * raw_inst_speed) + ((1.0 - alpha) * ema_speed);
            }

            let smooth_speed_u64 = ema_speed as u64;

            if start_time.elapsed() >= warmup_duration && smooth_speed_u64 > 0 {
                steady_state_samples.push(smooth_speed_u64);
            }

            let pct = ((start_time.elapsed().as_secs_f64() / test_duration.as_secs_f64()) * 100.0).min(100.0) as u32;
            progress_cb(smooth_speed_u64, pct);

            last_sample_time = Instant::now();
            last_total_bytes = current_total;
        }
    }

    stop_signal.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.await;
    }

    calculate_trimmed_mean(steady_state_samples)
}

// Tauri command to execute the complete speed test procedure
#[tauri::command]
pub async fn run_speed_test(app: AppHandle) -> Result<SpeedTestProgress, String> {
    if TEST_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Speed test is already running".to_string());
    }

    let result = run_speed_test_internal(app).await;
    TEST_RUNNING.store(false, Ordering::SeqCst);
    result
}

async fn run_speed_test_internal(app: AppHandle) -> Result<SpeedTestProgress, String> {
    // 1. STAGE: Ping Test
    let mut progress = SpeedTestProgress {
        stage: "ping".to_string(),
        current_speed: 0,
        progress_percent: 5,
        pings: Vec::new(),
        download_speed: 0,
        upload_speed: 0,
        average_ping: 0,
        message: "Testing DNS latency...".to_string(),
    };
    let _ = app.emit("speedtest-progress", &progress);

    let (dns_results, avg_ping) = run_multi_dns_ping().await;
    progress.pings = dns_results;
    progress.average_ping = avg_ping;
    progress.progress_percent = 20;
    progress.message = format!("DNS Latency: {} ms. Starting Download Speed Test...", avg_ping);
    let _ = app.emit("speedtest-progress", &progress);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // 2. STAGE: Download Speed Test
    progress.stage = "download".to_string();
    let app_handle_dl = app.clone();
    let pings_copy = progress.pings.clone();
    let avg_ping_copy = progress.average_ping;

    let peak_download = execute_download_test(|inst_speed, pct| {
        let scaled_pct = 20 + ((pct as f32 / 100.0) * 40.0) as u32; // 20% to 60%
        let p = SpeedTestProgress {
            stage: "download".to_string(),
            current_speed: inst_speed,
            progress_percent: scaled_pct,
            pings: pings_copy.clone(),
            download_speed: inst_speed,
            upload_speed: 0,
            average_ping: avg_ping_copy,
            message: "Testing Download Speed...".to_string(),
        };
        let _ = app_handle_dl.emit("speedtest-progress", &p);
    }).await;

    progress.download_speed = peak_download;
    progress.progress_percent = 60;
    progress.message = "Download Test Complete. Starting Upload Speed Test...".to_string();
    let _ = app.emit("speedtest-progress", &progress);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // 3. STAGE: Upload Speed Test
    progress.stage = "upload".to_string();
    let app_handle_ul = app.clone();
    let pings_copy_ul = progress.pings.clone();

    let peak_upload = execute_upload_test(|inst_speed, pct| {
        let scaled_pct = 60 + ((pct as f32 / 100.0) * 35.0) as u32; // 60% to 95%
        let p = SpeedTestProgress {
            stage: "upload".to_string(),
            current_speed: inst_speed,
            progress_percent: scaled_pct,
            pings: pings_copy_ul.clone(),
            download_speed: peak_download,
            upload_speed: inst_speed,
            average_ping: avg_ping_copy,
            message: "Testing Upload Speed...".to_string(),
        };
        let _ = app_handle_ul.emit("speedtest-progress", &p);
    }).await;

    progress.upload_speed = peak_upload;

    // 4. STAGE: Complete
    progress.stage = "complete".to_string();
    progress.current_speed = 0;
    progress.progress_percent = 100;
    progress.message = "Speed Test Completed Successfully!".to_string();
    let _ = app.emit("speedtest-progress", &progress);

    Ok(progress)
}
