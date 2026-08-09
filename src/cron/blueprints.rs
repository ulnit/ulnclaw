//! Automation blueprints — parameterized automations with typed slots.
//!
//! Port of hermes `cron/blueprint_catalog.py` (v2026.8.3): a blueprint
//! is a one-place definition of an automation that every surface can
//! render natively — the desktop renders a form (one field per slot),
//! chat surfaces a pre-filled `/blueprint` slash command, and the
//! agent gets a seed prompt. The single source of truth is the slot
//! schema below; [`fill_blueprint`] validates user-supplied values and
//! turns a blueprint into a create-job spec, so there is no second job
//! engine.
//!
//! Design choice: users never type raw cron. A blueprint carries a
//! fixed recurrence in `schedule_template` and parameterizes only the
//! human-friendly parts (time-of-day, weekday set). Blueprints needing
//! full flexibility would expose a `text` slot named `schedule` that
//! passes through verbatim.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

/// Named weekday recurrences -> cron day-of-week field (hermes
/// `WEEKDAY_PRESETS`).
pub const WEEKDAY_PRESETS: &[(&str, &str)] = &[
    ("everyday", "*"),
    ("weekdays", "1-5"),
    ("weekends", "0,6"),
];

const SLOT_TYPES: &[&str] = &["time", "enum", "text", "weekdays"];

/// A single fillable field on a blueprint (hermes `BlueprintSlot`).
#[derive(Debug, Clone, Serialize)]
pub struct BlueprintSlot {
    pub name: &'static str,
    #[serde(rename = "type")]
    pub slot_type: &'static str,
    pub label: &'static str,
    pub default: Option<&'static str>,
    pub options: Vec<&'static str>,
    pub optional: bool,
    /// When false, `options` are suggestions rather than a closed set —
    /// any value is accepted (e.g. the deliver slot, where the real set
    /// of valid platforms depends on configured gateways).
    pub strict: bool,
    pub help: &'static str,
}

impl BlueprintSlot {
    fn new(name: &'static str, slot_type: &'static str, label: &'static str) -> Self {
        assert!(SLOT_TYPES.contains(&slot_type), "unknown slot type");
        Self {
            name,
            slot_type,
            label,
            default: None,
            options: Vec::new(),
            optional: false,
            strict: true,
            help: "",
        }
    }
}

/// A parameterized automation blueprint (hermes `AutomationBlueprint`).
#[derive(Debug, Clone, Serialize)]
pub struct AutomationBlueprint {
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    /// Cron expression with `{slot}` placeholders, e.g.
    /// `{minute} {hour} * * {dow}`. A literal cron string with no
    /// placeholders = fixed schedule.
    pub schedule_template: &'static str,
    /// Seed instruction for the cron job prompt; may contain `{slot}`s.
    pub prompt_template: &'static str,
    pub slots: Vec<BlueprintSlot>,
    pub deliver_default: &'static str,
    pub skills: Vec<&'static str>,
    pub tags: Vec<&'static str>,
}

fn time_slot(default: &'static str) -> BlueprintSlot {
    BlueprintSlot {
        default: Some(default),
        help: "24h local time, e.g. 08:00",
        ..BlueprintSlot::new("time", "time", "What time?")
    }
}

fn deliver_slot() -> BlueprintSlot {
    BlueprintSlot {
        default: Some("origin"),
        options: vec!["origin", "local", "telegram", "discord", "email"],
        strict: false,
        help: "origin = the chat you set this up from (or your configured \
               home channel when created from the dashboard); local = save \
               only, no message; or any connected platform name",
        ..BlueprintSlot::new("deliver", "enum", "Where to deliver?")
    }
}

fn recurrence_slot(default: &'static str) -> BlueprintSlot {
    BlueprintSlot {
        default: Some(default),
        options: WEEKDAY_PRESETS.iter().map(|(k, _)| *k).collect(),
        ..BlueprintSlot::new("recurrence", "weekdays", "Repeat on")
    }
}

/// Curated in-repo catalog (hermes `CATALOG`).
pub fn catalog() -> Vec<AutomationBlueprint> {
    vec![
        AutomationBlueprint {
            key: "morning-brief",
            title: "Morning briefing",
            description: "A short daily briefing: today's calendar, weather, \
                          and anything urgent waiting on you.",
            category: "daily",
            schedule_template: "{minute} {hour} * * *",
            prompt_template: "Produce a concise morning briefing for the user: \
                              today's calendar events, the local weather, and \
                              any urgent items. Keep it short and scannable. If \
                              no data sources are connected, give a brief \
                              good-morning with the date and offer to connect \
                              calendar/email.",
            slots: vec![time_slot("08:00"), deliver_slot()],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["daily", "briefing"],
        },
        AutomationBlueprint {
            key: "important-mail",
            title: "Important-mail monitor",
            description: "Check your inbox periodically and ping you ONLY about \
                          mail that actually needs attention.",
            category: "email",
            schedule_template: "*/{interval_min} * * * *",
            prompt_template: "Check the user's inbox for new messages since the \
                              last run. Surface ONLY mail matching: {criteria}. \
                              Score candidates with the urgency classifier and \
                              deliver only what clears the bar; if nothing does, \
                              respond with [SILENT]. Requires a connected mail \
                              source; if none is configured, explain how to \
                              connect one and stop.",
            slots: vec![
                BlueprintSlot {
                    default: Some("30"),
                    options: vec!["15", "30", "60"],
                    help: "minutes between checks",
                    ..BlueprintSlot::new("interval_min", "enum", "How often?")
                },
                BlueprintSlot {
                    default: Some(
                        "needs a reply today, is from my manager or family, \
                         or mentions a deadline",
                    ),
                    ..BlueprintSlot::new(
                        "criteria",
                        "text",
                        "Only notify me if the mail…",
                    )
                },
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["email", "monitor"],
        },
        AutomationBlueprint {
            key: "weekly-review",
            title: "Weekly review",
            description: "A weekly recap: what got done, what's still open, and \
                          what's coming up.",
            category: "weekly",
            schedule_template: "{minute} {hour} * * {dow}",
            prompt_template: "Produce a weekly review for the user: what was \
                              accomplished this week, still-open items, and next \
                              week's calendar. Pull from connected sources. Keep \
                              it tight.",
            slots: vec![
                time_slot("18:00"),
                BlueprintSlot {
                    default: Some("sunday"),
                    options: vec!["sunday", "monday", "friday", "saturday"],
                    ..BlueprintSlot::new("day", "enum", "Which day?")
                },
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["weekly", "review"],
        },
        AutomationBlueprint {
            key: "workday-start",
            title: "Workday start reminder",
            description: "A weekday nudge with your agenda and top priorities.",
            category: "daily",
            schedule_template: "{minute} {hour} * * 1-5",
            prompt_template: "Give the user a brief weekday start-of-day nudge: \
                              today's calendar and the 1-3 highest-priority \
                              things to focus on, inferred from recent context \
                              and any task tools. Encouraging, short, one \
                              message.",
            slots: vec![time_slot("09:00"), deliver_slot()],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["daily", "focus"],
        },
        AutomationBlueprint {
            key: "custom-reminder",
            title: "Custom reminder",
            description: "A recurring reminder in your own words, on your \
                          schedule.",
            category: "general",
            schedule_template: "{minute} {hour} * * {dow}",
            prompt_template: "Remind the user: {what}",
            slots: vec![
                BlueprintSlot {
                    default: Some("take a break and stretch"),
                    ..BlueprintSlot::new("what", "text", "Remind me to…")
                },
                time_slot("14:00"),
                recurrence_slot("everyday"),
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["reminder"],
        },
        AutomationBlueprint {
            key: "evening-winddown",
            title: "Evening wind-down",
            description: "An end-of-day check-in: tomorrow's calendar at a \
                          glance and anything you should prep tonight.",
            category: "daily",
            schedule_template: "{minute} {hour} * * *",
            prompt_template: "Give the user a short evening wind-down: \
                              tomorrow's calendar, any early commitments to prep \
                              for, and one gentle nudge to wrap up loose ends \
                              from today. Keep it calm and brief — one message. \
                              If no calendar is connected, just offer a friendly \
                              sign-off and the weather for tomorrow.",
            slots: vec![time_slot("21:00"), deliver_slot()],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["daily", "evening"],
        },
        AutomationBlueprint {
            key: "news-digest",
            title: "Topic news digest",
            description: "A recurring digest on a topic you care about — \
                          deduped against what was already sent, so only \
                          genuinely new items land.",
            category: "general",
            schedule_template: "{minute} {hour} * * {dow}",
            prompt_template: "Search the web for new and noteworthy items about: \
                              {topic}. Dedupe against what you sent in previous \
                              runs — only include genuinely new developments. \
                              Deliver a tight digest of at most {count} bullets, \
                              each one line with a link. If nothing new since \
                              last run, respond with [SILENT].",
            slots: vec![
                BlueprintSlot {
                    default: Some("AI and technology"),
                    help: "a subject, product, person, or search phrase",
                    ..BlueprintSlot::new("topic", "text", "What topic?")
                },
                time_slot("18:00"),
                recurrence_slot("weekdays"),
                BlueprintSlot {
                    default: Some("5"),
                    options: vec!["3", "5", "8"],
                    ..BlueprintSlot::new("count", "enum", "How many bullets?")
                },
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["digest", "research"],
        },
        AutomationBlueprint {
            key: "bill-renewal-watch",
            title: "Bills & renewals reminder",
            description: "A heads-up before a recurring payment, subscription \
                          renewal, or due date — so nothing auto-charges by \
                          surprise.",
            category: "general",
            schedule_template: "{minute} {hour} * * {dow}",
            prompt_template: "Remind the user about an upcoming payment or \
                              renewal: {what}. Phrase it as an actionable \
                              heads-up (e.g. 'review or cancel before it \
                              renews'), not just a notification. One short \
                              message.",
            slots: vec![
                BlueprintSlot {
                    default: Some("my streaming subscription renews soon"),
                    ..BlueprintSlot::new("what", "text", "What's due?")
                },
                time_slot("10:00"),
                recurrence_slot("everyday"),
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["reminder", "finance"],
        },
        AutomationBlueprint {
            key: "habit-checkin",
            title: "Habit check-in",
            description: "A recurring nudge to keep a habit on track and \
                          reflect on whether you did it.",
            category: "general",
            schedule_template: "{minute} {hour} * * {dow}",
            prompt_template: "Nudge the user about their habit: {habit}. Ask \
                              whether they did it today, keep it warm and \
                              non-judgmental, and offer a one-line word of \
                              encouragement. One short message.",
            slots: vec![
                BlueprintSlot {
                    default: Some("20 minutes of reading"),
                    ..BlueprintSlot::new("habit", "text", "Which habit?")
                },
                time_slot("20:00"),
                recurrence_slot("everyday"),
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["habit", "wellbeing"],
        },
        AutomationBlueprint {
            key: "hydration-move",
            title: "Hydration & movement nudge",
            description: "A periodic nudge during the day to drink water, stand \
                          up, and stretch.",
            category: "general",
            // NOTE: cron minute-field steps (*/90) wrap per hour — */90 and
            // */120 both degrade to hourly. Use an hour-field step instead so
            // the chosen cadence is what actually fires.
            schedule_template: "0 {start_hour}-{end_hour}/{interval_hours} * * 1-5",
            prompt_template: "Send the user a brief, friendly nudge to drink \
                              some water, stand up, and stretch for a moment. \
                              Vary the wording each time so it doesn't feel \
                              robotic. One short line.",
            slots: vec![
                BlueprintSlot {
                    default: Some("1"),
                    options: vec!["1", "2", "3"],
                    help: "hours between nudges",
                    ..BlueprintSlot::new("interval_hours", "enum", "How often?")
                },
                BlueprintSlot {
                    default: Some("9"),
                    options: vec!["7", "8", "9", "10"],
                    help: "first hour of the active window (24h)",
                    ..BlueprintSlot::new("start_hour", "enum", "Start hour")
                },
                BlueprintSlot {
                    default: Some("17"),
                    options: vec!["16", "17", "18", "19"],
                    help: "last hour of the active window (24h)",
                    ..BlueprintSlot::new("end_hour", "enum", "End hour")
                },
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["wellbeing", "focus"],
        },
        AutomationBlueprint {
            key: "meal-plan",
            title: "Weekly meal plan",
            description: "A weekly meal plan plus a consolidated grocery list, \
                          tuned to your diet and how much time you have to \
                          cook.",
            category: "weekly",
            schedule_template: "{minute} {hour} * * {dow}",
            prompt_template: "Build the user a meal plan for the coming week: \
                              {meals} per day, suited to a {diet} diet and \
                              roughly {effort} cooking effort. Include a \
                              consolidated grocery list grouped by aisle. Keep \
                              blueprints simple and skimmable.",
            slots: vec![
                BlueprintSlot {
                    default: Some("no restrictions"),
                    options: vec![
                        "no restrictions",
                        "vegetarian",
                        "vegan",
                        "high-protein",
                        "low-carb",
                    ],
                    ..BlueprintSlot::new("diet", "enum", "Diet?")
                },
                BlueprintSlot {
                    default: Some("dinner only"),
                    options: vec!["dinner only", "lunch and dinner", "all three"],
                    ..BlueprintSlot::new("meals", "enum", "Meals per day?")
                },
                BlueprintSlot {
                    default: Some("quick"),
                    options: vec!["quick", "medium", "ambitious"],
                    ..BlueprintSlot::new("effort", "enum", "Cooking effort?")
                },
                time_slot("17:00"),
                BlueprintSlot {
                    default: Some("sunday"),
                    options: vec!["sunday", "monday", "friday", "saturday"],
                    ..BlueprintSlot::new("day", "enum", "Which day?")
                },
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["weekly", "food"],
        },
        AutomationBlueprint {
            key: "learn-daily",
            title: "Daily learning drip",
            description: "One bite-sized lesson a day on a topic you want to \
                          learn, building progressively over time.",
            category: "daily",
            schedule_template: "{minute} {hour} * * {dow}",
            prompt_template: "Teach the user one bite-sized lesson about: \
                              {topic}. Build on earlier lessons so it progresses \
                              rather than repeating. Keep it to a couple of \
                              short paragraphs with one concrete example, and \
                              end with a single question to check \
                              understanding.",
            slots: vec![
                BlueprintSlot {
                    default: Some("Spanish vocabulary"),
                    ..BlueprintSlot::new("topic", "text", "Learn about…")
                },
                time_slot("08:30"),
                recurrence_slot("weekdays"),
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["learning", "daily"],
        },
        AutomationBlueprint {
            key: "gratitude-journal",
            title: "Gratitude & reflection prompt",
            description: "A gentle evening prompt to reflect on the day and \
                          note what went well.",
            category: "general",
            schedule_template: "{minute} {hour} * * {dow}",
            prompt_template: "Send the user a short, warm reflection prompt for \
                              the end of the day — invite them to note one thing \
                              that went well, one thing they are grateful for, \
                              and one small win. If they reply, acknowledge it \
                              kindly. One message.",
            slots: vec![time_slot("21:30"), recurrence_slot("everyday"), deliver_slot()],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["wellbeing", "reflection"],
        },
        AutomationBlueprint {
            key: "on-this-day",
            title: "On-this-day discovery",
            description: "A daily dose of curiosity: a notable historical \
                          event, fact, or word for the day.",
            category: "daily",
            schedule_template: "{minute} {hour} * * *",
            prompt_template: "Give the user one interesting '{flavor}' item for \
                              today — keep it short, surprising, and genuinely \
                              interesting. One or two sentences, no filler.",
            slots: vec![
                BlueprintSlot {
                    default: Some("on this day in history"),
                    options: vec![
                        "on this day in history",
                        "word of the day",
                        "science fact",
                        "quote of the day",
                    ],
                    ..BlueprintSlot::new("flavor", "enum", "What kind?")
                },
                time_slot("07:30"),
                deliver_slot(),
            ],
            deliver_default: "origin",
            skills: vec![],
            tags: vec!["daily", "curiosity"],
        },
    ]
}

/// Look up a blueprint by key (hermes `get_blueprint`).
pub fn get_blueprint(key: &str) -> Option<AutomationBlueprint> {
    catalog().into_iter().find(|b| b.key == key)
}

// ---------------------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------------------

static PLACEHOLDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{(\w+)\}").unwrap());
static TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([01]?\d|2[0-3]):([0-5]\d)$").unwrap());

/// Build the flattened `/blueprint <key> slot=val …` command string
/// (hermes `blueprint_slash_command`). Uses each slot's default when
/// `values` is omitted; free-text slots are quoted.
pub fn blueprint_slash_command(
    blueprint: &AutomationBlueprint,
    values: Option<&HashMap<String, String>>,
) -> String {
    let empty = HashMap::new();
    let values = values.unwrap_or(&empty);
    let mut parts = vec![format!("/blueprint {}", blueprint.key)];
    for slot in &blueprint.slots {
        let mut val = values
            .get(slot.name)
            .cloned()
            .or_else(|| slot.default.map(str::to_string))
            .unwrap_or_default();
        if val.is_empty() {
            if slot.optional {
                continue;
            }
        }
        if slot.slot_type == "text" || val.contains(' ') {
            val = format!("\"{}\"", val.replace('"', "\\\""));
        }
        parts.push(format!("{}={}", slot.name, val));
    }
    parts.join(" ")
}

/// Build the `ulnclaw://blueprint/<key>?slot=val` deep-link URL (hermes
/// `blueprint_deeplink`, rebranded scheme).
pub fn blueprint_deeplink(
    blueprint: &AutomationBlueprint,
    values: Option<&HashMap<String, String>>,
) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    let empty = HashMap::new();
    let values = values.unwrap_or(&empty);
    let mut query: Vec<String> = Vec::new();
    for slot in &blueprint.slots {
        let val = values
            .get(slot.name)
            .cloned()
            .or_else(|| slot.default.map(str::to_string))
            .unwrap_or_default();
        if !val.is_empty() {
            query.push(format!(
                "{}={}",
                utf8_percent_encode(slot.name, NON_ALPHANUMERIC),
                utf8_percent_encode(&val, NON_ALPHANUMERIC)
            ));
        }
    }
    let key = utf8_percent_encode(blueprint.key, NON_ALPHANUMERIC);
    let qs = if query.is_empty() {
        String::new()
    } else {
        format!("?{}", query.join("&"))
    };
    format!("ulnclaw://blueprint/{key}{qs}")
}

/// A short human-readable description of when a blueprint runs with
/// default values (hermes `_humanize_schedule`).
fn humanize_schedule(blueprint: &AutomationBlueprint) -> String {
    let sched = blueprint.schedule_template;
    if let Some(rest) = sched.strip_prefix("*/") {
        let iv = blueprint.slots.iter().find(|s| s.name == "interval_min");
        let every = iv
            .and_then(|s| s.default)
            .map(str::to_string)
            .unwrap_or_else(|| {
                rest.split('/')
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string()
            });
        return format!("every {every} minutes");
    }
    if sched.contains("{interval_hours}") {
        let iv = blueprint
            .slots
            .iter()
            .find(|s| s.name == "interval_hours");
        let every = iv.and_then(|s| s.default).unwrap_or("1").to_string();
        let scope = if sched.contains("* * 1-5") { "weekdays, " } else { "" };
        return if every == "1" {
            format!("{scope}every hour")
        } else {
            format!("{scope}every {every} hours")
        };
    }
    let time_slot = blueprint.slots.iter().find(|s| s.slot_type == "time");
    let when = time_slot.and_then(|s| s.default);
    if sched.contains("* * 1-5") {
        return match when {
            Some(w) => format!("weekdays at {w}"),
            None => "every weekday".to_string(),
        };
    }
    if sched.contains("{dow}") {
        let day_slot = blueprint
            .slots
            .iter()
            .find(|s| s.name == "day" || s.name == "recurrence");
        let scope = day_slot.and_then(|s| s.default).unwrap_or("");
        if !scope.is_empty() {
            if let Some(w) = when {
                return format!("{scope} at {w}");
            }
        }
        return match when {
            Some(w) => format!("at {w}"),
            None => "on a schedule".to_string(),
        };
    }
    if let Some(w) = when {
        return format!("daily at {w}");
    }
    "on a schedule".to_string()
}

/// Unified serializable shape for a blueprint (hermes
/// `blueprint_catalog_entry`) — form schema plus the ready-to-paste
/// slash command, the deep-link URL, and a human-readable schedule.
pub fn blueprint_catalog_entry(blueprint: &AutomationBlueprint) -> serde_json::Value {
    serde_json::json!({
        "key": blueprint.key,
        "title": blueprint.title,
        "description": blueprint.description,
        "category": blueprint.category,
        "tags": blueprint.tags,
        "fields": blueprint.slots.iter().map(|s| serde_json::json!({
            "name": s.name,
            "type": s.slot_type,
            "label": s.label,
            "default": s.default,
            "options": s.options,
            "optional": s.optional,
            "strict": s.strict,
            "help": s.help,
        })).collect::<Vec<_>>(),
        "schedule": blueprint.schedule_template,
        "scheduleHuman": humanize_schedule(blueprint),
        "command": blueprint_slash_command(blueprint, None),
        "appUrl": blueprint_deeplink(blueprint, None),
    })
}

// ---------------------------------------------------------------------------
// Fill + validate + translate to a create-job spec
// ---------------------------------------------------------------------------

/// Result of [`fill_blueprint`] — a create-job spec (hermes `create_job`
/// kwargs shape).
#[derive(Debug, Clone, Serialize)]
pub struct FilledBlueprint {
    pub name: String,
    pub prompt: String,
    pub schedule: String,
    pub deliver: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
}

fn day_to_dow(day: &str) -> Option<&'static str> {
    Some(match day {
        "sunday" => "0",
        "monday" => "1",
        "tuesday" => "2",
        "wednesday" => "3",
        "thursday" => "4",
        "friday" => "5",
        "saturday" => "6",
        _ => return None,
    })
}

/// Fill the schedule_template placeholders from resolved slot values
/// (hermes `_resolve_schedule`).
fn resolve_schedule(
    blueprint: &AutomationBlueprint,
    values: &HashMap<String, String>,
) -> Result<String, String> {
    let sched = blueprint.schedule_template;

    // A free-text `schedule` slot passes through verbatim.
    if let Some(raw) = values.get("schedule") {
        if !raw.trim().is_empty() {
            return Ok(raw.trim().to_string());
        }
    }

    let mut repl: HashMap<String, String> = HashMap::new();

    // time -> minute/hour
    if sched.contains("{minute}") || sched.contains("{hour}") {
        let time_val = values.get("time").cloned().unwrap_or_default();
        if time_val.is_empty() {
            return Err("a time is required".to_string());
        }
        let m = TIME_RE
            .captures(time_val.trim())
            .ok_or_else(|| format!("invalid time {time_val:?} — use HH:MM (24h)"))?;
        repl.insert("hour".to_string(), m[1].parse::<u32>().unwrap().to_string());
        repl.insert("minute".to_string(), m[2].parse::<u32>().unwrap().to_string());
    }

    // weekday set -> dow
    if sched.contains("{dow}") {
        if values.contains_key("recurrence") {
            let preset = values
                .get("recurrence")
                .map(|v| v.to_lowercase())
                .unwrap_or_else(|| "everyday".to_string());
            let dow = WEEKDAY_PRESETS
                .iter()
                .find(|(k, _)| *k == preset)
                .map(|(_, v)| *v)
                .ok_or_else(|| {
                    format!(
                        "unknown recurrence {preset:?} — one of {}",
                        WEEKDAY_PRESETS
                            .iter()
                            .map(|(k, _)| *k)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            repl.insert("dow".to_string(), dow.to_string());
        } else if let Some(day) = values.get("day") {
            let day = day.to_lowercase();
            let dow = day_to_dow(&day)
                .ok_or_else(|| format!("unknown day {day:?}"))?;
            repl.insert("dow".to_string(), dow.to_string());
        } else {
            repl.insert("dow".to_string(), "*".to_string());
        }
    }

    // interval (minutes) for */N schedules
    if sched.contains("{interval_min}") {
        let iv = values
            .get("interval_min")
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        if iv.is_empty() || iv.parse::<u64>().map(|n| n == 0).unwrap_or(true) {
            return Err(format!(
                "invalid interval {iv:?} — minutes as a positive integer"
            ));
        }
        repl.insert("interval_min".to_string(), iv);
    }

    // Any remaining {slot} placeholders are filled verbatim from
    // validated enum/text slot values.
    let mut out = sched.to_string();
    for cap in PLACEHOLDER_RE.captures_iter(sched) {
        let name = &cap[1];
        if !repl.contains_key(name) {
            if let Some(val) = values.get(name) {
                repl.insert(name.to_string(), val.clone());
            }
        }
    }
    for (name, val) in &repl {
        out = out.replace(&format!("{{{name}}}"), val);
    }
    if let Some(cap) = PLACEHOLDER_RE.captures(&out) {
        return Err(format!(
            "schedule template missing value for {{{}}}",
            &cap[1]
        ));
    }
    Ok(out)
}

/// Render a template by substituting `{slot}` placeholders.
fn render_template(template: &str, values: &HashMap<String, String>) -> Result<String, String> {
    let mut out = template.to_string();
    for (name, val) in values {
        out = out.replace(&format!("{{{name}}}"), val);
    }
    if let Some(cap) = PLACEHOLDER_RE.captures(&out) {
        return Err(format!(
            "blueprint prompt missing value for {{{}}}",
            &cap[1]
        ));
    }
    Ok(out)
}

/// Validate `values` and return a create-job spec (hermes
/// `fill_blueprint`). Missing required slots, unknown slot names, and
/// out-of-set enum values all return errors naming the slot.
pub fn fill_blueprint(
    blueprint: &AutomationBlueprint,
    values: &HashMap<String, String>,
) -> Result<FilledBlueprint, String> {
    let known: std::collections::HashSet<&str> =
        blueprint.slots.iter().map(|s| s.name).collect();
    let mut unknown: Vec<&String> = values.keys().filter(|k| !known.contains(k.as_str())).collect();
    if !unknown.is_empty() {
        unknown.sort();
        let valid = blueprint
            .slots
            .iter()
            .map(|s| s.name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "unknown slot{}: {} — valid: {valid}",
            if unknown.len() > 1 { "s" } else { "" },
            unknown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let mut resolved: HashMap<String, String> = HashMap::new();
    for slot in &blueprint.slots {
        let raw = values
            .get(slot.name)
            .cloned()
            .or_else(|| slot.default.map(str::to_string))
            .unwrap_or_default();
        if raw.trim().is_empty() {
            if slot.optional {
                continue;
            }
            return Err(format!(
                "missing required value: {} ({})",
                slot.name, slot.label
            ));
        }
        if slot.slot_type == "enum"
            && slot.strict
            && !slot.options.is_empty()
            && !slot.options.iter().any(|o| *o == raw)
        {
            return Err(format!(
                "{}={raw:?} not allowed — one of {}",
                slot.name,
                slot.options.join(", ")
            ));
        }
        resolved.insert(slot.name.to_string(), raw);
    }

    let schedule = resolve_schedule(blueprint, &resolved)?;
    let prompt = render_template(blueprint.prompt_template, &resolved)?;

    Ok(FilledBlueprint {
        name: blueprint.title.to_string(),
        prompt,
        schedule,
        deliver: resolved
            .get("deliver")
            .cloned()
            .unwrap_or_else(|| blueprint.deliver_default.to_string()),
        skills: blueprint.skills.iter().map(|s| s.to_string()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn catalog_shape_matches_hermes() {
        let entries = catalog();
        assert_eq!(entries.len(), 14);
        let mut keys: Vec<&str> = entries.iter().map(|b| b.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 14, "blueprint keys must be unique");
        for bp in &entries {
            let entry = blueprint_catalog_entry(bp);
            assert!(entry["fields"].is_array());
            assert!(entry["scheduleHuman"].as_str().unwrap().len() > 0);
            assert!(entry["command"]
                .as_str()
                .unwrap()
                .starts_with(&format!("/blueprint {}", bp.key)));
            assert!(entry["appUrl"]
                .as_str()
                .unwrap()
                .starts_with("ulnclaw://blueprint/"));
        }
    }

    #[test]
    fn fill_morning_brief_defaults() {
        let bp = get_blueprint("morning-brief").unwrap();
        let filled = fill_blueprint(&bp, &vals(&[])).unwrap();
        assert_eq!(filled.schedule, "0 8 * * *");
        assert_eq!(filled.name, "Morning briefing");
        assert_eq!(filled.deliver, "origin");
    }

    #[test]
    fn fill_time_slot_parses_hhmm() {
        let bp = get_blueprint("morning-brief").unwrap();
        let filled = fill_blueprint(&bp, &vals(&[("time", "09:30")])).unwrap();
        assert_eq!(filled.schedule, "30 9 * * *");
    }

    #[test]
    fn fill_rejects_unknown_slot() {
        let bp = get_blueprint("morning-brief").unwrap();
        let err = fill_blueprint(&bp, &vals(&[("tiem", "07:15")])).unwrap_err();
        assert!(err.contains("unknown slot"), "{err}");
        assert!(err.contains("tiem"), "{err}");
    }

    #[test]
    fn fill_rejects_bad_time() {
        let bp = get_blueprint("morning-brief").unwrap();
        let err = fill_blueprint(&bp, &vals(&[("time", "25:99")])).unwrap_err();
        assert!(err.contains("invalid time"), "{err}");
    }

    #[test]
    fn fill_recurrence_preset_maps_to_dow() {
        let bp = get_blueprint("custom-reminder").unwrap();
        let filled =
            fill_blueprint(&bp, &vals(&[("what", "stretch"), ("recurrence", "weekdays")]))
                .unwrap();
        assert_eq!(filled.schedule, "0 14 * * 1-5");
        assert!(filled.prompt.contains("stretch"));
    }

    #[test]
    fn fill_day_enum_maps_to_dow() {
        let bp = get_blueprint("weekly-review").unwrap();
        let filled = fill_blueprint(&bp, &vals(&[("day", "friday")])).unwrap();
        assert_eq!(filled.schedule, "0 18 * * 5");
    }

    #[test]
    fn fill_strict_enum_rejects_out_of_set() {
        let bp = get_blueprint("news-digest").unwrap();
        let err = fill_blueprint(&bp, &vals(&[("count", "42")])).unwrap_err();
        assert!(err.contains("not allowed"), "{err}");
    }

    #[test]
    fn fill_non_strict_deliver_accepts_any_platform() {
        let bp = get_blueprint("morning-brief").unwrap();
        let filled = fill_blueprint(&bp, &vals(&[("deliver", "slack")])).unwrap();
        assert_eq!(filled.deliver, "slack");
    }

    #[test]
    fn fill_interval_schedule_hydration() {
        let bp = get_blueprint("hydration-move").unwrap();
        let filled = fill_blueprint(&bp, &vals(&[])).unwrap();
        assert_eq!(filled.schedule, "0 9-17/1 * * 1-5");
        let filled = fill_blueprint(
            &bp,
            &vals(&[("interval_hours", "2"), ("start_hour", "8"), ("end_hour", "18")]),
        )
        .unwrap();
        assert_eq!(filled.schedule, "0 8-18/2 * * 1-5");
    }

    #[test]
    fn fill_interval_minutes_monitor() {
        let bp = get_blueprint("important-mail").unwrap();
        let filled = fill_blueprint(&bp, &vals(&[("interval_min", "15")])).unwrap();
        assert_eq!(filled.schedule, "*/15 * * * *");
    }

    #[test]
    fn slash_command_quotes_free_text() {
        let bp = get_blueprint("custom-reminder").unwrap();
        let cmd = blueprint_slash_command(&bp, None);
        assert!(cmd.contains("what=\"take a break and stretch\""), "{cmd}");
    }

    #[test]
    fn humanize_schedules() {
        let by_key = |k: &str| {
            let bp = get_blueprint(k).unwrap();
            blueprint_catalog_entry(&bp)["scheduleHuman"].as_str().unwrap().to_string()
        };
        assert_eq!(by_key("morning-brief"), "daily at 08:00");
        assert_eq!(by_key("important-mail"), "every 30 minutes");
        assert_eq!(by_key("workday-start"), "weekdays at 09:00");
        assert_eq!(by_key("weekly-review"), "sunday at 18:00");
        assert_eq!(by_key("hydration-move"), "weekdays, every hour");
    }
}
