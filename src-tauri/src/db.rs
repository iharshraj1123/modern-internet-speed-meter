use rusqlite::{params, Connection, Result};
use std::path::Path;
use chrono::Local;

pub fn open_conn<P: AsRef<Path>>(path: P) -> Result<Connection> {
    let conn = Connection::open(path)?;
    let _ = conn.pragma_update(None, "journal_mode", &"WAL");
    let _ = conn.pragma_update(None, "busy_timeout", &5000);
    Ok(conn)
}

pub fn init_db<P: AsRef<Path>>(path: P) -> Result<Connection> {
    let conn = open_conn(path)?;
    
    // 1. Raw telemetry table (5-minute chunks, kept for 7 days)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS process_telemetry (
            timestamp INTEGER NOT NULL,
            process_name TEXT NOT NULL,
            bytes_downloaded INTEGER DEFAULT 0,
            bytes_uploaded INTEGER DEFAULT 0,
            screen_time_seconds INTEGER DEFAULT 0,
            PRIMARY KEY(timestamp, process_name)
        )",
        [],
    )?;

    // 2. Hourly aggregated statistics (kept for 90 days)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS hourly_stats (
            timestamp INTEGER NOT NULL,
            process_name TEXT NOT NULL,
            bytes_downloaded INTEGER DEFAULT 0,
            bytes_uploaded INTEGER DEFAULT 0,
            screen_time_seconds INTEGER DEFAULT 0,
            PRIMARY KEY(timestamp, process_name)
        )",
        [],
    )?;

    // 3. Daily aggregated statistics (kept indefinitely)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS daily_stats (
            date TEXT NOT NULL,
            process_name TEXT NOT NULL,
            bytes_downloaded INTEGER DEFAULT 0,
            bytes_uploaded INTEGER DEFAULT 0,
            screen_time_seconds INTEGER DEFAULT 0,
            PRIMARY KEY(date, process_name)
        )",
        [],
    )?;

    // Create indexes for faster queries
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_telemetry_time ON process_telemetry(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_hourly_time ON hourly_stats(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_daily_date ON daily_stats(date)",
        [],
    )?;

    Ok(conn)
}

pub fn log_interval(
    conn: &Connection,
    timestamp: i64, // Unix epoch rounded to 5-minute interval
    process_name: &str,
    bytes_in: u64,
    bytes_out: u64,
    screen_time_sec: u32,
) -> Result<()> {
    conn.execute(
        "INSERT INTO process_telemetry (timestamp, process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(timestamp, process_name) DO UPDATE SET
            bytes_downloaded = bytes_downloaded + excluded.bytes_downloaded,
            bytes_uploaded = bytes_uploaded + excluded.bytes_uploaded,
            screen_time_seconds = screen_time_seconds + excluded.screen_time_seconds",
        params![timestamp, process_name, bytes_in, bytes_out, screen_time_sec],
    )?;
    Ok(())
}

pub fn aggregate_data(conn: &Connection) -> Result<()> {
    let now = Local::now().timestamp();
    
    // Round to start of current hour
    let current_hour = (now / 3600) * 3600;
    
    // 1. Roll up raw 5-minute telemetry into hourly stats (for hours before the current one)
    conn.execute(
        "INSERT INTO hourly_stats (timestamp, process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds)
         SELECT (timestamp / 3600) * 3600 AS hr, process_name, 
                SUM(bytes_downloaded), SUM(bytes_uploaded), SUM(screen_time_seconds)
         FROM process_telemetry
         WHERE timestamp < ?1
         GROUP BY hr, process_name
         ON CONFLICT(timestamp, process_name) DO UPDATE SET
            bytes_downloaded = excluded.bytes_downloaded,
            bytes_uploaded = excluded.bytes_uploaded,
            screen_time_seconds = excluded.screen_time_seconds",
        params![current_hour],
    )?;

    // 2. Roll up hourly stats into daily stats
    conn.execute(
        "INSERT INTO daily_stats (date, process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds)
         SELECT strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') AS dt, process_name,
                SUM(bytes_downloaded), SUM(bytes_uploaded), SUM(screen_time_seconds)
         FROM hourly_stats
         GROUP BY dt, process_name
         ON CONFLICT(date, process_name) DO UPDATE SET
            bytes_downloaded = excluded.bytes_downloaded,
            bytes_uploaded = excluded.bytes_uploaded,
            screen_time_seconds = excluded.screen_time_seconds",
        [],
    )?;

    // 3. Purge old raw data (older than 7 days) and hourly data (older than 90 days)
    let seven_days_ago = now - (7 * 24 * 3600);
    let ninety_days_ago = now - (90 * 24 * 3600);
    
    conn.execute(
        "DELETE FROM process_telemetry WHERE timestamp < ?1",
        params![seven_days_ago],
    )?;
    conn.execute(
        "DELETE FROM hourly_stats WHERE timestamp < ?1",
        params![ninety_days_ago],
    )?;

    Ok(())
}

#[derive(serde::Serialize)]
pub struct ProcessStat {
    pub process_name: String,
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub screen_time_seconds: u64,
}

pub fn get_stats_for_period(conn: &Connection, period: &str) -> Result<Vec<ProcessStat>> {
    let query = match period {
        "hourly" => {
            // Last 24 hours of raw 5-minute telemetry (to show stats immediately)
            "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM process_telemetry
             WHERE timestamp >= (strftime('%s', 'now') - 86400)
             GROUP BY process_name
             HAVING COALESCE(SUM(bytes_downloaded), 0) > 0 OR COALESCE(SUM(bytes_uploaded), 0) > 0 OR COALESCE(SUM(screen_time_seconds), 0) > 0
             ORDER BY (COALESCE(SUM(bytes_downloaded), 0) + COALESCE(SUM(bytes_uploaded), 0)) DESC"
        }
        "daily" => {
            // Today's stats from raw 5-minute telemetry
            "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM process_telemetry
             WHERE strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') = strftime('%Y-%m-%d', 'now', 'localtime')
             GROUP BY process_name
             HAVING COALESCE(SUM(bytes_downloaded), 0) > 0 OR COALESCE(SUM(bytes_uploaded), 0) > 0 OR COALESCE(SUM(screen_time_seconds), 0) > 0
             ORDER BY (COALESCE(SUM(bytes_downloaded), 0) + COALESCE(SUM(bytes_uploaded), 0)) DESC"
        }
        "weekly" => {
            // Last 7 days from raw telemetry (since raw data is purged after 7 days)
            "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM process_telemetry
             WHERE timestamp >= (strftime('%s', 'now') - 7 * 86400)
             GROUP BY process_name
             HAVING COALESCE(SUM(bytes_downloaded), 0) > 0 OR COALESCE(SUM(bytes_uploaded), 0) > 0 OR COALESCE(SUM(screen_time_seconds), 0) > 0
             ORDER BY (COALESCE(SUM(bytes_downloaded), 0) + COALESCE(SUM(bytes_uploaded), 0)) DESC"
        }
        "monthly" => {
            // Last 30 days: historical daily_stats UNION with today's raw telemetry
            "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM (
                 SELECT process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds
                 FROM daily_stats
                 WHERE date >= strftime('%Y-%m-%d', 'now', '-30 days', 'localtime')
                   AND date < strftime('%Y-%m-%d', 'now', 'localtime')
                 UNION ALL
                 SELECT process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds
                 FROM process_telemetry
                 WHERE strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') = strftime('%Y-%m-%d', 'now', 'localtime')
             )
             GROUP BY process_name
             ORDER BY (COALESCE(SUM(bytes_downloaded), 0) + COALESCE(SUM(bytes_uploaded), 0)) DESC"
        }
        "yearly" => {
            // Last 365 days: historical daily_stats UNION with today's raw telemetry
            "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM (
                 SELECT process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds
                 FROM daily_stats
                 WHERE date >= strftime('%Y-%m-%d', 'now', '-365 days', 'localtime')
                   AND date < strftime('%Y-%m-%d', 'now', 'localtime')
                 UNION ALL
                 SELECT process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds
                 FROM process_telemetry
                 WHERE strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') = strftime('%Y-%m-%d', 'now', 'localtime')
             )
             GROUP BY process_name
             ORDER BY (COALESCE(SUM(bytes_downloaded), 0) + COALESCE(SUM(bytes_uploaded), 0)) DESC"
        }
        _ => {
            // Default to today
            "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM process_telemetry
             WHERE strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') = strftime('%Y-%m-%d', 'now', 'localtime')
             GROUP BY process_name
             ORDER BY (COALESCE(SUM(bytes_downloaded), 0) + COALESCE(SUM(bytes_uploaded), 0)) DESC"
        }
    };

    let mut stmt = conn.prepare(query)?;
    let rows = stmt.query_map([], |row| {
        Ok(ProcessStat {
            process_name: row.get(0)?,
            bytes_downloaded: row.get(1)?,
            bytes_uploaded: row.get(2)?,
            screen_time_seconds: row.get(3)?,
        })
    })?;

    let mut stats = Vec::new();
    for row in rows {
        stats.push(row?);
    }
    Ok(stats)
}
