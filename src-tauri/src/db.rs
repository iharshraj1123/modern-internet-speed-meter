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
    
    // 1. Raw telemetry table (5-minute chunks)
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

    // 2. Hourly aggregated statistics
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

    // 4. App settings key-value store
    conn.execute(
        "CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
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
    
    // Read configurable retention days from app_settings (defaults: 7 days raw, 90 days hourly)
    let raw_retention_days: i64 = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'raw_retention_days'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);

    let hourly_retention_days: i64 = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'hourly_retention_days'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);

    // Round to start of current hour
    let current_hour = (now / 3600) * 3600;
    
    // 1. Roll up raw 5-minute telemetry into hourly stats
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

    // 2. Roll up hourly stats into daily stats (only for completed past days)
    conn.execute(
        "INSERT INTO daily_stats (date, process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds)
         SELECT strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') AS dt, process_name,
                SUM(bytes_downloaded), SUM(bytes_uploaded), SUM(screen_time_seconds)
         FROM hourly_stats
         WHERE timestamp < strftime('%s', 'now', 'start of day', 'localtime')
         GROUP BY dt, process_name
         ON CONFLICT(date, process_name) DO UPDATE SET
            bytes_downloaded = excluded.bytes_downloaded,
            bytes_uploaded = excluded.bytes_uploaded,
            screen_time_seconds = excluded.screen_time_seconds",
        [],
    )?;

    // 3. Purge old data based on configurable retention policy
    let raw_cutoff = now - (raw_retention_days * 24 * 3600);
    let hourly_cutoff = now - (hourly_retention_days * 24 * 3600);
    
    conn.execute(
        "DELETE FROM process_telemetry WHERE timestamp < ?1",
        params![raw_cutoff],
    )?;
    conn.execute(
        "DELETE FROM hourly_stats WHERE timestamp < ?1",
        params![hourly_cutoff],
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
        "this_hour" => {
            // Stats for the current clock hour from raw 5-minute telemetry
            "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM process_telemetry
             WHERE timestamp >= ((strftime('%s', 'now') / 3600) * 3600)
             GROUP BY process_name
             HAVING COALESCE(SUM(bytes_downloaded), 0) > 0 OR COALESCE(SUM(bytes_uploaded), 0) > 0 OR COALESCE(SUM(screen_time_seconds), 0) > 0
             ORDER BY (COALESCE(SUM(bytes_downloaded), 0) + COALESCE(SUM(bytes_uploaded), 0)) DESC"
        }
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
            // Last 7 days: historical daily_stats UNION with today's raw telemetry
            "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM (
                 SELECT process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds
                 FROM daily_stats
                 WHERE date >= strftime('%Y-%m-%d', 'now', '-7 days', 'localtime')
                   AND date < strftime('%Y-%m-%d', 'now', 'localtime')
                 UNION ALL
                 SELECT process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds
                 FROM process_telemetry
                 WHERE strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') = strftime('%Y-%m-%d', 'now', 'localtime')
             )
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
        p if p.starts_with("month_") => {
            let month_str = p.trim_start_matches("month_");
            let query = "SELECT process_name, COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0), COALESCE(SUM(screen_time_seconds), 0)
             FROM (
                 SELECT process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds
                 FROM daily_stats
                 WHERE strftime('%Y-%m', date) = ?1 AND date < strftime('%Y-%m-%d', 'now', 'localtime')
                 UNION ALL
                 SELECT process_name, bytes_downloaded, bytes_uploaded, screen_time_seconds
                 FROM process_telemetry
                 WHERE strftime('%Y-%m', timestamp, 'unixepoch', 'localtime') = ?1
                   AND strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') = strftime('%Y-%m-%d', 'now', 'localtime')
             )
             GROUP BY process_name
             HAVING COALESCE(SUM(bytes_downloaded), 0) > 0 OR COALESCE(SUM(bytes_uploaded), 0) > 0 OR COALESCE(SUM(screen_time_seconds), 0) > 0
             ORDER BY (COALESCE(SUM(bytes_downloaded), 0) + COALESCE(SUM(bytes_uploaded), 0)) DESC";

            let mut stmt = conn.prepare(query)?;
            let rows = stmt.query_map(params![month_str], |row| {
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
            return Ok(stats);
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

pub fn get_available_months(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT month FROM (
            SELECT strftime('%Y-%m', date) AS month FROM daily_stats
            UNION
            SELECT strftime('%Y-%m', timestamp, 'unixepoch', 'localtime') AS month FROM process_telemetry
         ) WHERE month IS NOT NULL AND month != '' ORDER BY month DESC"
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut months = Vec::new();
    for row in rows {
        if let Ok(m) = row {
            months.push(m);
        }
    }
    Ok(months)
}

// --- New helpers for Data & Storage features ---

#[derive(serde::Serialize)]
pub struct DbInfo {
    pub total_size_bytes: u64,
    pub raw_rows: u64,
    pub hourly_rows: u64,
    pub daily_rows: u64,
    pub raw_retention_days: i64,
    pub hourly_retention_days: i64,
}

pub fn get_db_info<P: AsRef<Path>>(conn: &Connection, db_path: P) -> Result<DbInfo> {
    let total_size_bytes = std::fs::metadata(db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let raw_rows: u64 = conn.query_row(
        "SELECT COUNT(*) FROM process_telemetry", [], |row| row.get(0)
    ).unwrap_or(0);

    let hourly_rows: u64 = conn.query_row(
        "SELECT COUNT(*) FROM hourly_stats", [], |row| row.get(0)
    ).unwrap_or(0);

    let daily_rows: u64 = conn.query_row(
        "SELECT COUNT(*) FROM daily_stats", [], |row| row.get(0)
    ).unwrap_or(0);

    let raw_retention_days: i64 = conn
        .query_row("SELECT value FROM app_settings WHERE key = 'raw_retention_days'", [], |row| row.get::<_, String>(0))
        .ok().and_then(|v| v.parse().ok()).unwrap_or(7);

    let hourly_retention_days: i64 = conn
        .query_row("SELECT value FROM app_settings WHERE key = 'hourly_retention_days'", [], |row| row.get::<_, String>(0))
        .ok().and_then(|v| v.parse().ok()).unwrap_or(90);

    Ok(DbInfo { total_size_bytes, raw_rows, hourly_rows, daily_rows, raw_retention_days, hourly_retention_days })
}

pub fn set_retention_policy(conn: &Connection, raw_days: i64, hourly_days: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES ('raw_retention_days', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![raw_days.to_string()],
    )?;
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES ('hourly_retention_days', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![hourly_days.to_string()],
    )?;
    Ok(())
}

pub fn get_today_total_bytes(conn: &Connection) -> Result<(u64, u64)> {
    let row: (u64, u64) = conn.query_row(
        "SELECT COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0)
         FROM process_telemetry
         WHERE strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') = strftime('%Y-%m-%d', 'now', 'localtime')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?))
    ).unwrap_or((0, 0));
    Ok(row)
}

pub fn get_month_total_bytes(conn: &Connection) -> Result<(u64, u64)> {
    let row: (u64, u64) = conn.query_row(
        "SELECT COALESCE(SUM(bytes_downloaded), 0), COALESCE(SUM(bytes_uploaded), 0) FROM (
             SELECT bytes_downloaded, bytes_uploaded FROM daily_stats
             WHERE date >= strftime('%Y-%m-%d', 'now', '-30 days', 'localtime')
               AND date < strftime('%Y-%m-%d', 'now', 'localtime')
             UNION ALL
             SELECT bytes_downloaded, bytes_uploaded FROM process_telemetry
             WHERE strftime('%Y-%m-%d', timestamp, 'unixepoch', 'localtime') = strftime('%Y-%m-%d', 'now', 'localtime')
         )",
        [],
        |row| Ok((row.get(0)?, row.get(1)?))
    ).unwrap_or((0, 0));
    Ok(row)
}

pub fn vacuum_db(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
    Ok(())
}
