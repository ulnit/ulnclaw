//! Gateway message timestamps rendered exactly once (hermes
//! `gateway/message_timestamps.py` parity).
//!
//! Gateway messages need timestamps in the LLM context for temporal
//! awareness, but persisted message content must stay clean so replay
//! does not accumulate `[timestamp] [timestamp] ...` prefixes across
//! turns. The invariant is enforced by always stripping leading
//! timestamp prefixes before rendering a fresh one:
//!
//! * [`strip_leading_message_timestamps`] — remove one or more leading
//!   gateway timestamp prefixes, returning the clean text plus the
//!   embedded epoch of the prefix closest to the message text.
//! * [`render_user_content_with_timestamp`] — strip first, then prefix
//!   exactly one `[Tue 2026-04-28 13:40:53 CST]`-style timestamp
//!   (embedded time wins over the supplied value).
//! * [`coerce_message_timestamp`] — tolerant coercion of epoch numbers
//!   and ISO/human strings to Unix epoch seconds.
//!
//! Integration (hermes run.py semantics): inbound text is ALWAYS
//! stripped so the persisted transcript stays clean regardless of the
//! `[gateway] message_timestamps` toggle; the in-context RENDER is
//! gated on that toggle (default off).

use chrono::{DateTime, Local, TimeZone};
use std::sync::OnceLock;

use regex::Regex;

/// Current gateway format: `[Tue 2026-04-28 13:40:53 CEST]` (hermes
/// `_HUMAN_TIMESTAMP_RE`).
fn human_timestamp_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\[(?P<dow>[A-Z][a-z]{2}) (?P<date>\d{4}-\d{2}-\d{2}) (?P<time>\d{2}:\d{2}:\d{2})(?: (?P<tz>[A-Za-z0-9_+\-/:]+))?\]\s*",
        )
        .expect("human timestamp regex compiles")
    })
}

/// Older gateway format: `[2026-04-13T17:02:06+0200]` (hermes
/// `_ISO_TIMESTAMP_RE`).
fn iso_timestamp_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\[(?P<iso>\d{4}-\d{2}-\d{2}T[^\]]+)\]\s*")
            .expect("iso timestamp regex compiles")
    })
}

/// Coerce a timestamp-like value to Unix epoch seconds (hermes
/// `coerce_message_timestamp`). Accepts epoch numbers and ISO /
/// bracketed-prefix strings; None when uninterpretable.
pub fn coerce_message_timestamp(value: &str) -> Option<f64> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(epoch) = parse_timestamp_prefix(text) {
        return Some(epoch);
    }
    if let Ok(epoch) = text.parse::<f64>() {
        return Some(epoch);
    }
    parse_iso(text)
}

/// Format an epoch as `[Tue 2026-04-28 13:40:53 CST]` (hermes
/// `format_message_timestamp`). Empty string for an unrepresentable
/// time.
pub fn format_message_timestamp(epoch: f64) -> String {
    let Some(dt) = DateTime::from_timestamp(epoch as i64, 0) else {
        return String::new();
    };
    let local: DateTime<Local> = dt.with_timezone(&Local);
    format!("[{}]", local.format("%a %Y-%m-%d %H:%M:%S %Z"))
}

/// Strip one or more leading gateway timestamp prefixes from
/// `content` (hermes `strip_leading_message_timestamps`). Returns
/// `(clean_content, embedded_epoch)`; with multiple prefixes the one
/// closest to the message text wins (preserves the original
/// platform-send time for legacy contaminated rows).
pub fn strip_leading_message_timestamps(content: &str) -> (String, Option<f64>) {
    if content.is_empty() {
        return (content.to_string(), None);
    }
    let mut text = content;
    let mut embedded: Option<f64> = None;
    loop {
        let hit = human_timestamp_re()
            .find(text)
            .or_else(|| iso_timestamp_re().find(text));
        let Some(m) = hit else { break };
        if let Some(epoch) = parse_timestamp_prefix(text) {
            embedded = Some(epoch);
        }
        text = &text[m.end()..];
    }
    (text.to_string(), embedded)
}

/// Render a user message for LLM context with exactly one timestamp
/// prefix (hermes `render_user_content_with_timestamp`): existing
/// leading prefixes are removed first; an embedded time wins over
/// `ts_value`. Without any timestamp the cleaned content is returned
/// unchanged.
pub fn render_user_content_with_timestamp(content: &str, ts_value: Option<f64>) -> String {
    let (clean, embedded) = strip_leading_message_timestamps(content);
    let effective = embedded.or(ts_value);
    let Some(epoch) = effective else {
        return clean;
    };
    let prefix = format_message_timestamp(epoch);
    if prefix.is_empty() {
        return clean;
    }
    if clean.is_empty() {
        prefix
    } else {
        format!("{prefix} {clean}")
    }
}

fn parse_iso(text: &str) -> Option<f64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.timestamp() as f64);
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%#z") {
        return Some(dt.and_utc().timestamp() as f64);
    }
    // Naive ISO: interpret as local time (hermes `dt.astimezone()`).
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S") {
        if let Some(local) = Local.from_local_datetime(&naive).single() {
            return Some(local.timestamp() as f64);
        }
    }
    None
}

fn parse_timestamp_prefix(text: &str) -> Option<f64> {
    if let Some(m) = human_timestamp_re().find(text) {
        let caps = human_timestamp_re().captures(text)?;
        let date = caps.name("date")?.as_str();
        let time = caps.name("time")?.as_str();
        let naive =
            chrono::NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M:%S")
                .ok()?;
        // The bracketed tz abbreviation (e.g. CEST) is not reliably
        // parseable — hermes treats the wall-clock as local when no
        // tz object is supplied; do the same.
        let local = Local.from_local_datetime(&naive).single()?;
        let _ = m;
        return Some(local.timestamp() as f64);
    }
    if let Some(m) = iso_timestamp_re().find(text) {
        let caps = iso_timestamp_re().captures(text)?;
        let iso = caps.name("iso")?.as_str();
        let _ = m;
        return parse_iso(iso);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coerce_accepts_epoch_and_strings() {
        assert_eq!(coerce_message_timestamp("1745840453"), Some(1745840453.0));
        assert_eq!(coerce_message_timestamp("1745840453.5"), Some(1745840453.5));
        assert_eq!(coerce_message_timestamp(""), None);
        assert_eq!(coerce_message_timestamp("not a time"), None);
        let iso = coerce_message_timestamp("2026-04-13T17:02:06+02:00").unwrap();
        assert_eq!(iso, 1776092526.0);
    }

    #[test]
    fn format_roundtrips_through_strip() {
        let epoch = 1745840453.0;
        let formatted = format_message_timestamp(epoch);
        assert!(formatted.starts_with('['), "{formatted}");
        assert!(formatted.ends_with(']'), "{formatted}");
        let (clean, embedded) = strip_leading_message_timestamps(&format!("{formatted} hello"));
        assert_eq!(clean, "hello");
        // The embedded parse reconstructs the same wall-clock second.
        assert_eq!(embedded.unwrap() as i64, epoch as i64);
    }

    #[test]
    fn strip_removes_stacked_prefixes_closest_wins() {
        let t1 = format_message_timestamp(1745840000.0);
        let t2 = format_message_timestamp(1745840453.0);
        let input = format!("{t1} {t2} [sender] message body");
        let (clean, embedded) = strip_leading_message_timestamps(&input);
        assert_eq!(clean, "[sender] message body");
        assert_eq!(embedded.unwrap() as i64, 1745840453);
    }

    #[test]
    fn strip_iso_prefix() {
        let (clean, embedded) =
            strip_leading_message_timestamps("[2026-04-13T17:02:06+02:00] hi there");
        assert_eq!(clean, "hi there");
        assert_eq!(embedded.unwrap(), 1776092526.0);
    }

    #[test]
    fn strip_leaves_non_timestamp_brackets_alone() {
        let (clean, embedded) = strip_leading_message_timestamps("[sender] message");
        assert_eq!(clean, "[sender] message");
        assert!(embedded.is_none());
    }

    #[test]
    fn render_is_idempotent_no_accumulation() {
        // The core invariant: rendering an already-rendered message
        // must not stack prefixes.
        let once = render_user_content_with_timestamp("hello world", Some(1745840453.0));
        let twice = render_user_content_with_timestamp(&once, Some(1745999999.0));
        let count = twice.matches('[').count();
        assert_eq!(count, 1, "{twice}");
        // The embedded (original) time wins over the later ts_value.
        assert_eq!(once, twice);
    }

    #[test]
    fn render_keeps_one_prefix_or_none() {
        let t1 = format_message_timestamp(1745840000.0);
        let input = format!("{t1} plain text");
        // An embedded prefix is part of the content's meaning: render
        // keeps exactly one (re-rendered from the embedded time) even
        // without an explicit ts_value (hermes embedded-wins rule).
        let rendered = render_user_content_with_timestamp(&input, None);
        let (clean, _) = strip_leading_message_timestamps(&rendered);
        assert_eq!(clean, "plain text");
        assert_eq!(rendered.matches('[').count(), 1, "{rendered}");
        // No prefix at all: content passes through unchanged.
        assert_eq!(
            render_user_content_with_timestamp("no prefix", None),
            "no prefix"
        );
    }
}
