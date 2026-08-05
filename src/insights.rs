//! Session insights — usage analytics over the session store.
//!
//! Port of hermes `agent/insights.py` (v2026.8.3), adapted for ulnclaw's
//! SQLite schema: token consumption, models.dev-backed cost estimates,
//! tool usage, activity patterns (hour/weekday), source and model
//! breakdowns, top sessions. Inspired by Claude Code's `/insights`.

use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, TimeZone, Timelike};
use rusqlite::{params, Connection};

use crate::error::{AgentError, Result};

/// One session row as read by the engine.
#[derive(Debug, Clone)]
struct SessionData {
    id: String,
    source: String,
    model: String,
    started_at: f64,
    ended_at: Option<f64>,
    title: Option<String>,
    message_count: i64,
    tool_call_count: i64,
    input_tokens: i64,
    output_tokens: i64,
}

impl SessionData {
    fn total_tokens(&self) -> i64 {
        self.input_tokens + self.output_tokens
    }

    fn duration_seconds(&self) -> Option<f64> {
        self.ended_at
            .filter(|end| *end >= self.started_at)
            .map(|end| end - self.started_at)
    }
}

/// Overview counters (hermes `_compute_overview`).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Overview {
    pub total_sessions: usize,
    pub total_messages: i64,
    pub total_tool_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
    /// True when at least one session had models.dev pricing data.
    pub cost_known: bool,
    pub avg_session_seconds: f64,
    pub active_days: usize,
}

/// Per-model aggregate (hermes model breakdown).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelUsage {
    pub model: String,
    pub sessions: usize,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
    pub cost_known: bool,
}

/// Per-source aggregate (hermes platform breakdown).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceUsage {
    pub source: String,
    pub sessions: usize,
    pub total_tokens: i64,
    pub tool_calls: i64,
}

/// Per-tool call counts (hermes tool breakdown).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolUsage {
    pub tool: String,
    pub calls: i64,
}

/// Hour-of-day / weekday session starts (hermes activity patterns).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActivityPatterns {
    /// Session starts per hour of day (0-23).
    pub by_hour: Vec<u64>,
    /// Session starts per weekday (Mon=0 … Sun=6).
    pub by_weekday: Vec<u64>,
    pub peak_hour: Option<usize>,
    pub peak_weekday: Option<usize>,
}

/// Heavy-usage session (hermes top sessions).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TopSession {
    pub id: String,
    pub title: Option<String>,
    pub model: String,
    pub started_at: f64,
    pub messages: i64,
    pub tool_calls: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}

/// Full insights report (hermes `generate()` return shape).
#[derive(Debug, Clone, serde::Serialize)]
pub struct InsightsReport {
    pub days: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_filter: Option<String>,
    pub empty: bool,
    pub generated_at: f64,
    pub overview: Overview,
    pub models: Vec<ModelUsage>,
    pub sources: Vec<SourceUsage>,
    pub tools: Vec<ToolUsage>,
    pub activity: ActivityPatterns,
    pub top_sessions: Vec<TopSession>,
}

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<SessionData> {
    Ok(SessionData {
        id: row.get(0)?,
        source: row.get(1)?,
        model: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
        title: row.get(5)?,
        message_count: row.get(6)?,
        tool_call_count: row.get(7)?,
        input_tokens: row.get(8)?,
        output_tokens: row.get(9)?,
    })
}

fn row_to_tool(row: &rusqlite::Row) -> rusqlite::Result<ToolUsage> {
    Ok(ToolUsage {
        tool: row.get(0)?,
        calls: row.get(1)?,
    })
}

/// Analyzes session history and produces usage insights
/// (hermes `InsightsEngine`).
pub struct InsightsEngine {
    conn: Connection,
}

impl InsightsEngine {
    /// Open the store at `path` (a second WAL reader alongside the store).
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| AgentError::Tool(format!("insights: open {}: {}", path.display(), e)))?;
        Ok(Self { conn })
    }

    /// Open `$ULNCLAW_HOME/state.db`.
    pub fn open_default() -> Result<Self> {
        Self::open(&crate::config::ulnclaw_home().join("state.db"))
    }

    fn sessions_since(&self, cutoff: f64, source: Option<&str>) -> Result<Vec<SessionData>> {
        let mut rows = Vec::new();
        let (sql, source_filter): (String, bool) = match source {
            Some(_) => (
                "SELECT id, source, model, started_at, ended_at, title, message_count, \
                 tool_call_count, input_tokens, output_tokens FROM sessions \
                 WHERE started_at >= ?1 AND source = ?2 AND archived = 0 \
                 ORDER BY started_at DESC"
                    .to_string(),
                true,
            ),
            None => (
                "SELECT id, source, model, started_at, ended_at, title, message_count, \
                 tool_call_count, input_tokens, output_tokens FROM sessions \
                 WHERE started_at >= ?1 AND archived = 0 \
                 ORDER BY started_at DESC"
                    .to_string(),
                false,
            ),
        };
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| AgentError::Tool(format!("insights: prepare: {e}")))?;
        if source_filter {
            let mapped = stmt
                .query_map(params![cutoff, source.unwrap_or("")], row_to_session)
                .map_err(|e| AgentError::Tool(format!("insights: query: {e}")))?;
            for row in mapped {
                rows.push(row.map_err(|e| AgentError::Tool(format!("insights: row: {e}")))?);
            }
        } else {
            let mapped = stmt
                .query_map(params![cutoff], row_to_session)
                .map_err(|e| AgentError::Tool(format!("insights: query: {e}")))?;
            for row in mapped {
                rows.push(row.map_err(|e| AgentError::Tool(format!("insights: row: {e}")))?);
            }
        }
        Ok(rows)
    }

    fn tool_usage_since(&self, cutoff: f64, source: Option<&str>) -> Result<Vec<ToolUsage>> {
        // Tool-result messages carry the tool name; join to sessions for the
        // source filter.
        let (sql, filtered) = match source {
            Some(_) => (
                "SELECT m.tool_name, COUNT(*) FROM messages m \
                 JOIN sessions s ON s.id = m.session_id \
                 WHERE m.role = 'tool' AND m.tool_name IS NOT NULL AND m.tool_name != '' \
                 AND m.timestamp >= ?1 AND s.source = ?2 \
                 GROUP BY m.tool_name ORDER BY COUNT(*) DESC LIMIT 30"
                    .to_string(),
                true,
            ),
            None => (
                "SELECT tool_name, COUNT(*) FROM messages \
                 WHERE role = 'tool' AND tool_name IS NOT NULL AND tool_name != '' \
                 AND timestamp >= ?1 \
                 GROUP BY tool_name ORDER BY COUNT(*) DESC LIMIT 30"
                    .to_string(),
                false,
            ),
        };
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| AgentError::Tool(format!("insights: prepare: {e}")))?;
        let mut out = Vec::new();
        if filtered {
            let mapped = stmt
                .query_map(params![cutoff, source.unwrap_or("")], row_to_tool)
                .map_err(|e| AgentError::Tool(format!("insights: query: {e}")))?;
            for row in mapped {
                out.push(row.map_err(|e| AgentError::Tool(format!("insights: row: {e}")))?);
            }
        } else {
            let mapped = stmt
                .query_map(params![cutoff], row_to_tool)
                .map_err(|e| AgentError::Tool(format!("insights: query: {e}")))?;
            for row in mapped {
                out.push(row.map_err(|e| AgentError::Tool(format!("insights: row: {e}")))?);
            }
        }
        Ok(out)
    }

    /// Generate a complete insights report (hermes `generate`).
    ///
    /// `provider_hint` feeds models.dev cost lookup (session rows carry the
    /// model but not the provider).
    pub fn generate(
        &self,
        days: u32,
        source: Option<&str>,
        provider_hint: Option<&str>,
    ) -> Result<InsightsReport> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let cutoff = now - (days as f64) * 86400.0;

        let sessions = self.sessions_since(cutoff, source)?;
        if sessions.is_empty() {
            return Ok(InsightsReport {
                days,
                source_filter: source.map(String::from),
                empty: true,
                generated_at: now,
                overview: Overview::default(),
                models: Vec::new(),
                sources: Vec::new(),
                tools: Vec::new(),
                activity: ActivityPatterns {
                    by_hour: vec![0; 24],
                    by_weekday: vec![0; 7],
                    peak_hour: None,
                    peak_weekday: None,
                },
                top_sessions: Vec::new(),
            });
        }

        let tools = self.tool_usage_since(cutoff, source)?;
        let overview = compute_overview(&sessions, provider_hint);
        let models = compute_model_breakdown(&sessions, provider_hint);
        let sources = compute_source_breakdown(&sessions);
        let activity = compute_activity_patterns(&sessions);
        let top_sessions = compute_top_sessions(&sessions, provider_hint);

        Ok(InsightsReport {
            days,
            source_filter: source.map(String::from),
            empty: false,
            generated_at: now,
            overview,
            models,
            sources,
            tools,
            activity,
            top_sessions,
        })
    }
}

// =========================================================================
// Cost estimation (models.dev pricing)
// =========================================================================

/// USD cost for a model/token pair via models.dev pricing; `None` when the
/// model has no known pricing (hermes `_estimate_cost` semantics).
pub fn estimate_cost(provider_hint: Option<&str>, model: &str, input_tokens: i64, output_tokens: i64) -> Option<f64> {
    // models.dev is keyed by provider — without one the lookup can never
    // match, so skip the registry fetch entirely (keeps tests off the
    // shared models.dev cache).
    let provider = provider_hint.unwrap_or("");
    if provider.trim().is_empty() || model.trim().is_empty() {
        return None;
    }
    let info = crate::models_dev::get_model_info(provider, model)?;
    if !info.has_cost_data() {
        return None;
    }
    Some(
        (input_tokens as f64) * info.cost_input / 1_000_000.0
            + (output_tokens as f64) * info.cost_output / 1_000_000.0,
    )
}

// =========================================================================
// Aggregations
// =========================================================================

fn compute_overview(sessions: &[SessionData], provider_hint: Option<&str>) -> Overview {
    let mut overview = Overview {
        total_sessions: sessions.len(),
        ..Default::default()
    };
    let mut durations: Vec<f64> = Vec::new();
    let mut days: std::collections::HashSet<String> = std::collections::HashSet::new();

    for session in sessions {
        overview.total_messages += session.message_count;
        overview.total_tool_calls += session.tool_call_count;
        overview.input_tokens += session.input_tokens;
        overview.output_tokens += session.output_tokens;
        if let Some(duration) = session.duration_seconds() {
            durations.push(duration);
        }
        if let Some(date) = timestamp_date(session.started_at) {
            days.insert(date);
        }
        if let Some(cost) = estimate_cost(
            provider_hint,
            &session.model,
            session.input_tokens,
            session.output_tokens,
        ) {
            overview.estimated_cost_usd += cost;
            overview.cost_known = true;
        }
    }
    overview.total_tokens = overview.input_tokens + overview.output_tokens;
    overview.active_days = days.len();
    if !durations.is_empty() {
        overview.avg_session_seconds = durations.iter().sum::<f64>() / durations.len() as f64;
    }
    overview
}

fn compute_model_breakdown(sessions: &[SessionData], provider_hint: Option<&str>) -> Vec<ModelUsage> {
    let mut by_model: std::collections::BTreeMap<String, ModelUsage> = std::collections::BTreeMap::new();
    for session in sessions {
        let key = if session.model.is_empty() {
            "(unknown)".to_string()
        } else {
            session.model.clone()
        };
        let entry = by_model.entry(key.clone()).or_insert_with(|| ModelUsage {
            model: key,
            sessions: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            cost_known: false,
        });
        entry.sessions += 1;
        entry.input_tokens += session.input_tokens;
        entry.output_tokens += session.output_tokens;
        entry.total_tokens += session.total_tokens();
        if let Some(cost) = estimate_cost(
            provider_hint,
            &session.model,
            session.input_tokens,
            session.output_tokens,
        ) {
            entry.estimated_cost_usd += cost;
            entry.cost_known = true;
        }
    }
    let mut out: Vec<ModelUsage> = by_model.into_values().collect();
    out.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    out
}

fn compute_source_breakdown(sessions: &[SessionData]) -> Vec<SourceUsage> {
    let mut by_source: std::collections::BTreeMap<String, SourceUsage> = std::collections::BTreeMap::new();
    for session in sessions {
        let entry = by_source
            .entry(session.source.clone())
            .or_insert_with(|| SourceUsage {
                source: session.source.clone(),
                sessions: 0,
                total_tokens: 0,
                tool_calls: 0,
            });
        entry.sessions += 1;
        entry.total_tokens += session.total_tokens();
        entry.tool_calls += session.tool_call_count;
    }
    let mut out: Vec<SourceUsage> = by_source.into_values().collect();
    out.sort_by(|a, b| b.sessions.cmp(&a.sessions));
    out
}

fn timestamp_date(ts: f64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

fn compute_activity_patterns(sessions: &[SessionData]) -> ActivityPatterns {
    let mut by_hour = vec![0u64; 24];
    let mut by_weekday = vec![0u64; 7];
    for session in sessions {
        if let Some(dt) = Local.timestamp_opt(session.started_at as i64, 0).single() {
            by_hour[dt.hour() as usize] += 1;
            // chrono Weekday: Monday=0 … Sunday=6 via num_days_from_monday().
            by_weekday[dt.weekday().num_days_from_monday() as usize] += 1;
        }
    }
    let peak_hour = argmax(&by_hour);
    let peak_weekday = argmax(&by_weekday);
    ActivityPatterns {
        by_hour,
        by_weekday,
        peak_hour,
        peak_weekday,
    }
}

fn argmax(values: &[u64]) -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    for (i, value) in values.iter().enumerate() {
        if *value > 0 && best.map(|(_, b)| *value > b).unwrap_or(true) {
            best = Some((i, *value));
        }
    }
    best.map(|(i, _)| i)
}

fn compute_top_sessions(sessions: &[SessionData], provider_hint: Option<&str>) -> Vec<TopSession> {
    let mut ranked: Vec<&SessionData> = sessions.iter().collect();
    ranked.sort_by(|a, b| b.total_tokens().cmp(&a.total_tokens()));
    ranked
        .into_iter()
        .take(5)
        .map(|session| TopSession {
            id: session.id.clone(),
            title: session.title.clone(),
            model: session.model.clone(),
            started_at: session.started_at,
            messages: session.message_count,
            tool_calls: session.tool_call_count,
            total_tokens: session.total_tokens(),
            estimated_cost_usd: estimate_cost(
                provider_hint,
                &session.model,
                session.input_tokens,
                session.output_tokens,
            )
            .unwrap_or(0.0),
        })
        .collect()
}

// =========================================================================
// Terminal rendering (hermes format_terminal)
// =========================================================================

/// Simple horizontal bar chart strings (hermes `_bar_chart`).
pub fn bar_chart(values: &[u64], max_width: usize) -> Vec<String> {
    let peak = values.iter().copied().max().unwrap_or(1).max(1);
    values
        .iter()
        .map(|v| {
            if *v == 0 {
                String::new()
            } else {
                "█".repeat(((*v as f64 / peak as f64) * max_width as f64) as usize).max("█".to_string())
            }
        })
        .collect()
}

/// Compact duration: `3d 4h`, `2h 5m`, `45s` (hermes
/// `format_duration_compact`).
pub fn format_duration_compact(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Human token count (1234567 → "1.2M", 12345 → "12.3K").
pub fn format_tokens(tokens: i64) -> String {
    let value = tokens.max(0) as f64;
    if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Render the report for the terminal (hermes `format_terminal`).
pub fn format_terminal(report: &InsightsReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\n📊 ulnclaw Insights — last {} day{}\n",
        report.days,
        if report.days == 1 { "" } else { "s" }
    ));
    if let Some(source) = &report.source_filter {
        out.push_str(&format!("   source filter: {source}\n"));
    }
    out.push_str(&format!("{}\n", "─".repeat(50)));

    if report.empty {
        out.push_str("\nNo sessions recorded in this period.\n\n");
        return out;
    }

    let overview = &report.overview;
    out.push_str("\nOverview\n");
    out.push_str(&format!(
        "  Sessions: {} · Messages: {} · Tool calls: {}\n",
        overview.total_sessions, overview.total_messages, overview.total_tool_calls
    ));
    out.push_str(&format!(
        "  Tokens: {} in · {} out · {} total\n",
        format_tokens(overview.input_tokens),
        format_tokens(overview.output_tokens),
        format_tokens(overview.total_tokens)
    ));
    if overview.cost_known {
        out.push_str(&format!("  Est. cost: ${:.4}\n", overview.estimated_cost_usd));
    } else {
        out.push_str("  Est. cost: unknown (no models.dev pricing matched)\n");
    }
    out.push_str(&format!(
        "  Avg session: {} · Active days: {}\n",
        format_duration_compact(overview.avg_session_seconds),
        overview.active_days
    ));

    if !report.models.is_empty() {
        out.push_str("\nModels\n");
        let totals: Vec<u64> = report.models.iter().map(|m| m.total_tokens as u64).collect();
        let bars = bar_chart(&totals, 20);
        for (model, bar) in report.models.iter().zip(bars.iter()).take(8) {
            let cost = if model.cost_known {
                format!("  ${:.4}", model.estimated_cost_usd)
            } else {
                String::new()
            };
            out.push_str(&format!(
                "  {:<28} {:>4} sessions  {:>7} tokens  {}{}\n",
                truncate_str(&model.model, 28),
                model.sessions,
                format_tokens(model.total_tokens),
                bar,
                cost
            ));
        }
    }

    if !report.sources.is_empty() {
        out.push_str("\nSources\n");
        for source in &report.sources {
            out.push_str(&format!(
                "  {:<12} {:>4} sessions  {:>7} tokens  {:>5} tool calls\n",
                truncate_str(&source.source, 12),
                source.sessions,
                format_tokens(source.total_tokens),
                source.tool_calls
            ));
        }
    }

    if !report.tools.is_empty() {
        out.push_str("\nTools\n");
        let totals: Vec<u64> = report.tools.iter().map(|t| t.calls as u64).collect();
        let bars = bar_chart(&totals, 20);
        for (tool, bar) in report.tools.iter().zip(bars.iter()).take(10) {
            out.push_str(&format!(
                "  {:<28} {:>5}  {}\n",
                truncate_str(&tool.tool, 28),
                tool.calls,
                bar
            ));
        }
    }

    let activity = &report.activity;
    let any_activity = activity.by_hour.iter().any(|v| *v > 0);
    if any_activity {
        out.push_str("\nActivity\n");
        if let Some(peak) = activity.peak_hour {
            out.push_str(&format!("  Peak hour: {:02}:00\n", peak));
        }
        if let Some(peak) = activity.peak_weekday {
            out.push_str(&format!(
                "  Peak weekday: {}\n",
                WEEKDAYS.get(peak).copied().unwrap_or("?")
            ));
        }
        let hour_bars = bar_chart(&activity.by_hour, 16);
        for (hour, count) in activity.by_hour.iter().enumerate() {
            if *count > 0 {
                out.push_str(&format!(
                    "  {:02}:00  {:>3}  {}\n",
                    hour, count, hour_bars[hour]
                ));
            }
        }
        let weekday_bars = bar_chart(&activity.by_weekday, 10);
        for (i, day) in WEEKDAYS.iter().enumerate() {
            if activity.by_weekday[i] > 0 {
                out.push_str(&format!(
                    "  {} {:>3} {}\n",
                    day, activity.by_weekday[i], weekday_bars[i]
                ));
            }
        }
    }

    if !report.top_sessions.is_empty() {
        out.push_str("\nTop sessions (by tokens)\n");
        for session in &report.top_sessions {
            let label = session
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| session.id.chars().take(8).collect());
            let date = chrono::DateTime::from_timestamp(session.started_at as i64, 0)
                .map(|dt| dt.with_timezone(&Local).format("%m-%d %H:%M").to_string())
                .unwrap_or_default();
            out.push_str(&format!(
                "  {}  {:<20}  {:>4} msgs  {:>7} tokens  {}\n",
                date,
                truncate_str(&label, 20),
                session.messages,
                format_tokens(session.total_tokens),
                truncate_str(&session.model, 18)
            ));
        }
    }
    out.push('\n');
    out
}

fn truncate_str(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", head)
    }
}

/// Default state.db path helper for CLI wiring.
pub fn default_store_path() -> PathBuf {
    crate::config::ulnclaw_home().join("state.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SqliteSessionStore;

    fn seeded_store(dir: &Path) -> SqliteSessionStore {
        let store = SqliteSessionStore::open(dir.join("state.db")).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        for i in 0..3 {
            let id = store.create_session("cli", Some("test-model"), None).unwrap();
            store
                .update_usage(&id, 100 * (i + 1), 50 * (i + 1), i as u32)
                .unwrap();
            store.end_session(&id, "completed").unwrap();
        }
        // An archived session that must be excluded.
        let archived = store.create_session("cli", Some("test-model"), None).unwrap();
        store.set_session_archived(&archived, true).unwrap();
        let _ = now;
        store
    }

    #[test]
    fn generate_aggregates_sessions_and_excludes_archived() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path());
        drop(store);
        let engine = InsightsEngine::open(&dir.path().join("state.db")).unwrap();
        let report = engine.generate(30, None, None).unwrap();
        assert!(!report.empty);
        assert_eq!(report.overview.total_sessions, 3);
        assert_eq!(report.overview.input_tokens, 600);
        assert_eq!(report.overview.output_tokens, 300);
        assert_eq!(report.overview.total_tokens, 900);
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.models[0].model, "test-model");
        assert_eq!(report.sources[0].source, "cli");
    }

    #[test]
    fn empty_window_reports_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteSessionStore::open(dir.path().join("state.db")).unwrap();
        drop(store);
        let engine = InsightsEngine::open(&dir.path().join("state.db")).unwrap();
        let report = engine.generate(30, None, None).unwrap();
        assert!(report.empty);
        let text = format_terminal(&report);
        assert!(text.contains("No sessions recorded"));
    }

    #[test]
    fn source_filter_restricts() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path());
        let gateway_session = store.create_session("gateway", Some("test-model"), None).unwrap();
        store.update_usage(&gateway_session, 10, 10, 0).unwrap();
        drop(store);
        let engine = InsightsEngine::open(&dir.path().join("state.db")).unwrap();
        let report = engine.generate(30, Some("gateway"), None).unwrap();
        assert_eq!(report.overview.total_sessions, 1);
        assert_eq!(report.source_filter.as_deref(), Some("gateway"));
    }

    #[test]
    fn bar_chart_scales_to_peak() {
        let bars = bar_chart(&[0, 5, 10], 10);
        assert_eq!(bars[0], "");
        assert_eq!(bars[2].chars().count(), 10);
        assert!(bars[1].chars().count() >= 1 && bars[1].chars().count() <= 10);
    }

    #[test]
    fn duration_and_token_formats() {
        assert_eq!(format_duration_compact(45.0), "45s");
        assert_eq!(format_duration_compact(125.0), "2m 5s");
        assert_eq!(format_duration_compact(3725.0), "1h 2m");
        assert_eq!(format_duration_compact(90061.0), "1d 1h");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(12_345), "12.3K");
        assert_eq!(format_tokens(1_234_567), "1.2M");
    }

    #[test]
    fn truncate_keeps_width() {
        assert_eq!(truncate_str("short", 10), "short");
        let long = truncate_str("a very long session title here", 10);
        assert_eq!(long.chars().count(), 10);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn activity_patterns_bucket_starts() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path());
        drop(store);
        let engine = InsightsEngine::open(&dir.path().join("state.db")).unwrap();
        let report = engine.generate(30, None, None).unwrap();
        assert_eq!(report.activity.by_hour.len(), 24);
        assert_eq!(report.activity.by_weekday.len(), 7);
        let total: u64 = report.activity.by_hour.iter().sum();
        assert_eq!(total, 3);
    }
}
