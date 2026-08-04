//! Cron — port of hermes' cron/ package (job store + schedule parsing +
//! scheduler loop).
//!
//! Jobs live in the state DB (`cron_jobs` table). Schedules support:
//!   - interval shorthands: "30m", "every 2h", "1d"
//!   - 5-field cron expressions: "0 9 * * *"
//!   - ISO timestamps for one-shot runs: "2026-06-01T09:00:00"

use crate::error::{AgentError, Result};
use chrono::{DateTime, Duration, Local, NaiveDateTime};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    #[serde(default)]
    pub skills: Vec<String>,
    pub enabled: bool,
    /// Remaining runs (None = forever).
    pub repeat: Option<i64>,
    pub next_run: Option<f64>,
    pub created_at: f64,
    pub last_run: Option<f64>,
    pub last_status: Option<String>,
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Schedule parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Schedule {
    /// Repeat every N seconds.
    Interval(i64),
    /// One-shot at a fixed unix time.
    OneShot(f64),
    /// 5-field cron expression (minute hour day month weekday).
    Cron(CronExpr),
}

/// Minimal 5-field cron expression (supports *, lists, ranges, steps).
#[derive(Debug, Clone, PartialEq)]
pub struct CronExpr {
    pub minutes: Vec<u32>,
    pub hours: Vec<u32>,
    pub days: Vec<u32>,
    pub months: Vec<u32>,
    pub weekdays: Vec<u32>,
}

fn parse_field(field: &str, min: u32, max: u32) -> std::result::Result<Vec<u32>, String> {
    let mut values = Vec::new();
    for part in field.split(',') {
        let (range_part, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().map_err(|_| format!("bad step: {}", s))?),
            None => (part, 1),
        };
        let (lo, hi) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            (
                a.parse::<u32>().map_err(|_| format!("bad range: {}", part))?,
                b.parse::<u32>().map_err(|_| format!("bad range: {}", part))?,
            )
        } else {
            let v = range_part
                .parse::<u32>()
                .map_err(|_| format!("bad value: {}", part))?;
            (v, v)
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("value out of range: {}", part));
        }
        let mut v = lo;
        while v <= hi {
            values.push(v);
            v += step.max(1);
        }
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

impl CronExpr {
    pub fn parse(expr: &str) -> std::result::Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!("cron expression needs 5 fields, got {}", fields.len()));
        }
        Ok(Self {
            minutes: parse_field(fields[0], 0, 59)?,
            hours: parse_field(fields[1], 0, 23)?,
            days: parse_field(fields[2], 1, 31)?,
            months: parse_field(fields[3], 1, 12)?,
            weekdays: parse_field(fields[4], 0, 6)?,
        })
    }

    /// Next run strictly after `from` (local time).
    pub fn next_after(&self, from: DateTime<Local>) -> Option<DateTime<Local>> {
        use chrono::Timelike;
        let mut candidate = from.checked_add_signed(Duration::minutes(1))?;
        // Zero out seconds.
        candidate = candidate
            .date_naive()
            .and_hms_opt(candidate.hour(), candidate.minute(), 0)?
            .and_local_timezone(Local)
            .single()?;
        // Walk forward up to 2 years.
        for _ in 0..(366 * 2 * 24 * 60) {
            if self.matches(candidate) {
                return Some(candidate);
            }
            candidate = candidate.checked_add_signed(Duration::minutes(1))?;
        }
        None
    }

    fn matches(&self, time: DateTime<Local>) -> bool {
        use chrono::Datelike;
        use chrono::Timelike;
        let weekday = time.weekday().num_days_from_sunday();
        self.minutes.contains(&time.minute())
            && self.hours.contains(&time.hour())
            && self.days.contains(&time.day())
            && self.months.contains(&time.month())
            && self.weekdays.contains(&weekday)
    }
}

/// Parse hermes-style schedule strings.
pub fn parse_schedule(raw: &str) -> Result<Schedule> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AgentError::config("empty schedule"));
    }

    // ISO timestamp one-shot (starts with YYYY-...).
    if raw.len() >= 10 && raw.as_bytes()[4] == b'-' && raw.chars().take(4).all(|c| c.is_ascii_digit()) {
        let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
            .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
            .or_else(|_| {
                NaiveDateTime::parse_from_str(&format!("{} 00:00:00", raw), "%Y-%m-%d %H:%M:%S")
            })
            .map_err(|e| AgentError::config(format!("bad ISO timestamp '{}': {}", raw, e)))?;
        let local = naive
            .and_local_timezone(Local)
            .single()
            .ok_or_else(|| AgentError::config(format!("ambiguous local time: {}", raw)))?;
        return Ok(Schedule::OneShot(local.timestamp() as f64));
    }

    // Interval shorthand: optional "every " prefix + number + unit, no spaces.
    let body = raw.strip_prefix("every ").unwrap_or(raw);
    if !body.contains(char::is_whitespace) {
        let split = body
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(body.len());
        let (number, unit_raw) = body.split_at(split);
        let unit = unit_raw.trim().to_lowercase();
        if !number.is_empty() {
            let n: i64 = number
                .parse()
                .map_err(|_| AgentError::config(format!("bad interval: {}", raw)))?;
            let seconds = match unit.as_str() {
                "s" | "sec" | "secs" | "second" | "seconds" => n,
                "m" | "min" | "mins" | "minute" | "minutes" => n * 60,
                "h" | "hr" | "hrs" | "hour" | "hours" => n * 3600,
                "d" | "day" | "days" => n * 86400,
                "w" | "week" | "weeks" => n * 604800,
                "" => {
                    return Err(AgentError::config(format!(
                        "schedule '{}' needs a unit (s/m/h/d/w) or a cron expression",
                        raw
                    )));
                }
                other => {
                    return Err(AgentError::config(format!(
                        "unknown interval unit '{}' (use s/m/h/d/w or a cron expression)",
                        other
                    )));
                }
            };
            if seconds < 60 {
                return Err(AgentError::config("minimum interval is 60 seconds"));
            }
            return Ok(Schedule::Interval(seconds));
        }
    }

    // 5-field cron expression.
    CronExpr::parse(raw)
        .map(Schedule::Cron)
        .map_err(|e| AgentError::config(format!("schedule '{}': {}", raw, e)))
}

/// Compute the next run time (unix seconds) for a schedule, strictly after now.
pub fn next_run(schedule: &Schedule) -> Option<f64> {
    match schedule {
        Schedule::Interval(seconds) => Some(now() + *seconds as f64),
        Schedule::OneShot(at) => Some(*at),
        Schedule::Cron(expr) => expr.next_after(Local::now()).map(|t| t.timestamp() as f64),
    }
}

// ---------------------------------------------------------------------------
// Job store (SQLite)
// ---------------------------------------------------------------------------

const CRON_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS cron_jobs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    schedule TEXT NOT NULL,
    prompt TEXT NOT NULL,
    skills TEXT NOT NULL DEFAULT '[]',
    enabled INTEGER NOT NULL DEFAULT 1,
    repeat INTEGER,
    next_run REAL,
    created_at REAL NOT NULL,
    last_run REAL,
    last_status TEXT
);
"#;

pub struct CronStore {
    conn: Mutex<Connection>,
}

impl CronStore {
    /// Open the cron store inside an existing state DB file.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| AgentError::session(format!("open cron db: {}", e)))?;
        conn.execute_batch(CRON_SCHEMA)
            .map_err(|e| AgentError::session(format!("cron schema: {}", e)))?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn open_default() -> Result<Self> {
        let home = crate::config::ensure_home()?;
        Self::open(&home.join("state.db"))
    }

    pub fn add(&self, job: &CronJob) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "INSERT INTO cron_jobs (id, name, schedule, prompt, skills, enabled, repeat, next_run, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                job.id,
                job.name,
                job.schedule,
                job.prompt,
                serde_json::to_string(&job.skills).unwrap_or_else(|_| "[]".into()),
                job.enabled as i32,
                job.repeat,
                job.next_run,
                job.created_at,
            ],
        )
        .map_err(|e| AgentError::session(format!("add job: {}", e)))?;
        Ok(())
    }

    pub fn update(&self, job: &CronJob) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "UPDATE cron_jobs SET name=?2, schedule=?3, prompt=?4, skills=?5, enabled=?6,
                repeat=?7, next_run=?8, last_run=?9, last_status=?10
             WHERE id=?1",
            params![
                job.id,
                job.name,
                job.schedule,
                job.prompt,
                serde_json::to_string(&job.skills).unwrap_or_else(|_| "[]".into()),
                job.enabled as i32,
                job.repeat,
                job.next_run,
                job.last_run,
                job.last_status,
            ],
        )
        .map_err(|e| AgentError::session(format!("update job: {}", e)))?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<CronJob>> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        let row = conn
            .query_row(
                "SELECT id, name, schedule, prompt, skills, enabled, repeat, next_run, created_at, last_run, last_status
                 FROM cron_jobs WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i32>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<f64>>(7)?,
                        row.get::<_, f64>(8)?,
                        row.get::<_, Option<f64>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AgentError::session(e.to_string()))?;
        Ok(row.map(|(id, name, schedule, prompt, skills, enabled, repeat, next_run, created_at, last_run, last_status)| {
            CronJob {
                id,
                name,
                schedule,
                prompt,
                skills: serde_json::from_str(&skills).unwrap_or_default(),
                enabled: enabled != 0,
                repeat,
                next_run,
                created_at,
                last_run,
                last_status,
            }
        }))
    }

    pub fn list(&self) -> Result<Vec<CronJob>> {
        let mut ids: Vec<String> = Vec::new();
        {
            let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
            let mut stmt = conn
                .prepare("SELECT id FROM cron_jobs ORDER BY created_at")
                .map_err(|e| AgentError::session(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| AgentError::session(e.to_string()))?;
            for row in rows {
                if let Ok(id) = row {
                    ids.push(id);
                }
            }
        }
        let mut jobs = Vec::new();
        for id in ids {
            if let Some(job) = self.get(&id)? {
                jobs.push(job);
            }
        }
        Ok(jobs)
    }

    pub fn remove(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        let n = conn
            .execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])
            .map_err(|e| AgentError::session(e.to_string()))?;
        Ok(n > 0)
    }

    /// Jobs due to run (enabled + next_run <= now).
    pub fn due_jobs(&self) -> Result<Vec<CronJob>> {
        let now = now();
        Ok(self
            .list()?
            .into_iter()
            .filter(|job| job.enabled && job.next_run.map(|t| t <= now).unwrap_or(false))
            .collect())
    }
}

/// The scheduler loop — call from a tokio task. Checks for due jobs every
/// `poll_secs` and executes them via the provided runner.
pub async fn run_scheduler<F, Fut>(store: std::sync::Arc<CronStore>, poll_secs: u64, mut runner: F)
where
    F: FnMut(CronJob) -> Fut + Send,
    Fut: std::future::Future<Output = Result<String>> + Send,
{
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(poll_secs.max(5)));
    loop {
        interval.tick().await;
        let due = match store.due_jobs() {
            Ok(due) => due,
            Err(_) => continue,
        };
        for mut job in due {
            let result = runner(job.clone()).await;
            job.last_run = Some(now());
            job.last_status = Some(match &result {
                Ok(_) => "ok".to_string(),
                Err(e) => format!("error: {}", e),
            });
            // Reschedule or disable.
            match parse_schedule(&job.schedule) {
                Ok(schedule) => {
                    if let Schedule::OneShot(_) = schedule {
                        job.enabled = false;
                        job.next_run = None;
                    } else if let Some(repeat) = job.repeat {
                        if repeat <= 1 {
                            job.enabled = false;
                            job.next_run = None;
                        } else {
                            job.repeat = Some(repeat - 1);
                            job.next_run = next_run(&schedule);
                        }
                    } else {
                        job.next_run = next_run(&schedule);
                    }
                }
                Err(e) => {
                    job.enabled = false;
                    job.last_status = Some(format!("reschedule failed: {}", e));
                }
            }
            store.update(&job).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_intervals() {
        assert_eq!(parse_schedule("30m").unwrap(), Schedule::Interval(1800));
        assert_eq!(parse_schedule("every 2h").unwrap(), Schedule::Interval(7200));
        assert_eq!(parse_schedule("1d").unwrap(), Schedule::Interval(86400));
        assert!(parse_schedule("5s").is_err()); // below minimum
    }

    #[test]
    fn test_parse_cron_expr() {
        use chrono::Timelike;
        let expr = match parse_schedule("0 9 * * *").unwrap() {
            Schedule::Cron(e) => e,
            other => panic!("expected cron, got {:?}", other),
        };
        assert_eq!(expr.minutes, vec![0]);
        assert_eq!(expr.hours, vec![9]);
        let next = expr.next_after(Local::now()).unwrap();
        assert_eq!(next.hour(), 9);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn test_parse_oneshot() {
        match parse_schedule("2030-06-01T09:00:00").unwrap() {
            Schedule::OneShot(ts) => assert!(ts > now()),
            other => panic!("expected one-shot, got {:?}", other),
        }
    }

    #[test]
    fn test_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = CronStore::open(&dir.path().join("state.db")).unwrap();
        let job = CronJob {
            id: "job-1".into(),
            name: "daily".into(),
            schedule: "0 9 * * *".into(),
            prompt: "Say good morning".into(),
            skills: vec![],
            enabled: true,
            repeat: None,
            next_run: Some(now() + 3600.0),
            created_at: now(),
            last_run: None,
            last_status: None,
        };
        store.add(&job).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(store.get("job-1").unwrap().is_some());
        assert!(store.remove("job-1").unwrap());
        assert_eq!(store.list().unwrap().len(), 0);
    }
}
