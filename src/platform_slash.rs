//! Direct slash commands for messaging platforms — port of the hermes
//! `gateway/slash_commands.py` direct-command subset.
//!
//! Messaging platforms (Telegram, Discord, Slack, …) dispatch chat messages
//! into the agent loop; a small command set answers directly WITHOUT an LLM
//! turn (hermes direct-command semantics), and `/skill-name` / `/<bundle>`
//! invocations expand into scaffolded agent turns exactly like the gateway
//! session-chat endpoints do.

use std::path::Path;

use crate::agent::Agent;
use crate::session::sqlite::SqliteSessionStore;

/// Outcome of resolving a platform slash message.
#[derive(Debug, Clone, PartialEq)]
pub enum PlatformSlashOutcome {
    /// Reply directly — no LLM turn.
    Direct(String),
    /// Replace the inbound text with an expanded agent-turn message.
    AgentTurn(String),
}

const PLATFORM_SLASH_HELP: &str = "Commands you can send as chat messages:
  /help            this list
  /skills          list skills (invoke one: /<skill-name> [instruction])
  /tools           list enabled tools
  /recap           recap this chat's session
  /title [text]    show or set the session title
  /usage           this session's token usage
  /insights [N] [--days N] [--source S]   usage analytics across sessions
  /reload-mcp      reload MCP servers (may ask confirmation)
  /approve, /deny  resolve a pending approval
  /<bundle>        invoke a skill bundle";

/// Resolve a platform chat message as a slash command, if it is one.
/// Returns `None` when the text is not a recognized command — unknown
/// `/…` text stays an ordinary agent message (users legitimately send
/// paths and other slash-prefixed text in chat).
pub async fn resolve(
    agent: &Agent,
    store: &SqliteSessionStore,
    home: &Path,
    session_id: &str,
    text: &str,
) -> Option<PlatformSlashOutcome> {
    let trimmed = text.trim();
    let stripped = trimmed.strip_prefix('/')?;
    if stripped.is_empty() {
        return None;
    }
    let mut parts = trimmed.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    let skills_dir = home.join("skills");
    match cmd {
        "/help" => Some(PlatformSlashOutcome::Direct(
            PLATFORM_SLASH_HELP.to_string(),
        )),
        "/skills" => {
            let skills = crate::skills::list_skills(&skills_dir);
            if skills.is_empty() {
                Some(PlatformSlashOutcome::Direct(
                    "no skills installed (<home>/skills).".to_string(),
                ))
            } else {
                let mut out = String::new();
                for skill in &skills {
                    out.push_str(&format!("  {} — {}\n", skill.name, skill.description));
                }
                Some(PlatformSlashOutcome::Direct(out.trim_end().to_string()))
            }
        }
        "/tools" => Some(PlatformSlashOutcome::Direct(
            agent.tool_names().join(", "),
        )),
        "/recap" => {
            let row = store.get_session_row(session_id).ok().flatten();
            let messages = store.load_messages(session_id).unwrap_or_default();
            let recap = crate::session::recap::build_recap(
                &messages,
                row.as_ref().and_then(|r| r.title.as_deref()),
                Some(session_id),
            );
            Some(PlatformSlashOutcome::Direct(recap))
        }
        "/title" => {
            if rest.is_empty() {
                let title = store
                    .get_session_row(session_id)
                    .ok()
                    .flatten()
                    .and_then(|row| row.title.clone())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "(untitled)".to_string());
                Some(PlatformSlashOutcome::Direct(format!("title: {title}")))
            } else {
                match store.set_session_title(session_id, rest) {
                    Ok(()) => Some(PlatformSlashOutcome::Direct(format!(
                        "title set: {rest}"
                    ))),
                    Err(e) => Some(PlatformSlashOutcome::Direct(format!(
                        "title set failed: {e}"
                    ))),
                }
            }
        }
        "/usage" => {
            let Some(row) = store.get_session_row(session_id).ok().flatten() else {
                return Some(PlatformSlashOutcome::Direct(
                    "session not found.".to_string(),
                ));
            };
            Some(PlatformSlashOutcome::Direct(format!(
                "messages: {}  tokens: {} in / {} out",
                row.message_count, row.input_tokens, row.output_tokens
            )))
        }
        "/insights" => {
            let mut days: u32 = 30;
            let mut source: Option<String> = None;
            let mut tokens = rest.split_whitespace();
            while let Some(token) = tokens.next() {
                if token == "--days" {
                    if let Some(value) = tokens.next() {
                        if let Ok(parsed) = value.parse::<u32>() {
                            days = parsed;
                        }
                    }
                } else if token == "--source" {
                    source = tokens.next().map(String::from);
                } else if let Ok(parsed) = token.parse::<u32>() {
                    days = parsed;
                }
            }
            let provider_hint = agent.context().config.model.provider.clone();
            let result = match crate::insights::InsightsEngine::open_default() {
                Ok(engine) => {
                    match engine.generate(days, source.as_deref(), Some(&provider_hint)) {
                        Ok(report) => crate::insights::format_gateway(&report),
                        Err(e) => format!("insights failed: {e}"),
                    }
                }
                Err(e) => format!("insights failed: {e}"),
            };
            Some(PlatformSlashOutcome::Direct(result))
        }
        _ => {
            // Bundles win over single skills (hermes bundle-over-skill slash
            // precedence); unknown names fall through to the agent.
            let cmd_name = cmd.trim_start_matches('/');
            if let Some(key) = crate::bundles::resolve_bundle_command_key(cmd_name) {
                if let Some((message, _loaded, _missing)) =
                    crate::bundles::build_bundle_invocation_message(&key, rest, &skills_dir)
                {
                    return Some(PlatformSlashOutcome::AgentTurn(message));
                }
            }
            if let Some(message) =
                crate::skills::build_skill_invocation_message(&skills_dir, cmd_name, rest)
            {
                return Some(PlatformSlashOutcome::AgentTurn(message));
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf, Arc<Agent>, Arc<SqliteSessionStore>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_path_buf();
        std::fs::create_dir_all(home.join("skills")).ok();
        let store = Arc::new(
            SqliteSessionStore::open(dir.path().join("state.db")).expect("store opens"),
        );
        let provider = Arc::new(
            crate::provider::openai::OpenAiProvider::builder()
                .endpoint("http://127.0.0.1:9/v1")
                .model("test-model")
                .name("test")
                .build()
                .expect("provider builds"),
        );
        let agent = Arc::new(
            Agent::new(provider, crate::tools::ToolRegistry::new()).with_store(store.clone()),
        );
        (dir, home, agent, store)
    }

    fn named_session(store: &SqliteSessionStore, id: &str) {
        store
            .create_named_session(id, "slack", None, None)
            .expect("named session");
    }

    #[tokio::test]
    async fn help_lists_commands_without_agent_state() {
        let (_dir, home, agent, store) = setup();
        let outcome = resolve(&agent, &store, &home, "s1", "/help").await;
        let Some(PlatformSlashOutcome::Direct(reply)) = outcome else {
            panic!("expected direct reply");
        };
        assert!(reply.contains("/skills"));
        assert!(reply.contains("/insights"));
        assert!(reply.contains("/approve"));
    }

    #[tokio::test]
    async fn tools_lists_registered_tool_names() {
        let (_dir, home, agent, store) = setup();
        let outcome = resolve(&agent, &store, &home, "s1", "/tools").await;
        let Some(PlatformSlashOutcome::Direct(reply)) = outcome else {
            panic!("expected direct reply");
        };
        // An empty registry still answers directly (empty list).
        assert!(!reply.contains("unknown command"));
    }

    #[tokio::test]
    async fn title_show_and_set_roundtrip() {
        let (_dir, home, agent, store) = setup();
        named_session(&store, "s1");

        let outcome = resolve(&agent, &store, &home, "s1", "/title").await;
        assert!(matches!(outcome, Some(PlatformSlashOutcome::Direct(ref t)) if t.contains("(untitled)")));

        let outcome = resolve(&agent, &store, &home, "s1", "/title My chat").await;
        assert!(matches!(outcome, Some(PlatformSlashOutcome::Direct(ref t)) if t.contains("title set: My chat")));

        let outcome = resolve(&agent, &store, &home, "s1", "/title").await;
        assert!(matches!(outcome, Some(PlatformSlashOutcome::Direct(ref t)) if t.contains("title: My chat")));
    }

    #[tokio::test]
    async fn usage_reports_session_counters() {
        let (_dir, home, agent, store) = setup();
        named_session(&store, "s1");
        let outcome = resolve(&agent, &store, &home, "s1", "/usage").await;
        let Some(PlatformSlashOutcome::Direct(reply)) = outcome else {
            panic!("expected direct reply");
        };
        assert!(reply.starts_with("messages: "), "got: {reply}");
        assert!(reply.contains("tokens:"));
    }

    #[tokio::test]
    async fn usage_missing_session_reports_not_found() {
        let (_dir, home, agent, store) = setup();
        let outcome = resolve(&agent, &store, &home, "ghost", "/usage").await;
        assert!(matches!(outcome, Some(PlatformSlashOutcome::Direct(ref t)) if t == "session not found."));
    }

    #[tokio::test]
    async fn unknown_slash_falls_through() {
        let (_dir, home, agent, store) = setup();
        assert!(resolve(&agent, &store, &home, "s1", "/usr/bin/foo").await.is_none());
        assert!(resolve(&agent, &store, &home, "s1", "plain text").await.is_none());
        assert!(resolve(&agent, &store, &home, "s1", "/").await.is_none());
    }

    #[tokio::test]
    async fn skills_listing_empty_home() {
        let (_dir, home, agent, store) = setup();
        let outcome = resolve(&agent, &store, &home, "s1", "/skills").await;
        assert!(matches!(outcome, Some(PlatformSlashOutcome::Direct(ref t)) if t.contains("no skills installed")));
    }

    #[tokio::test]
    async fn skill_invocation_expands_to_agent_turn() {
        let (_dir, home, agent, store) = setup();
        let skill_dir = home.join("skills").join("greet");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: greet\ndescription: Greet people\n---\n\nSay hello kindly.\n",
        )
        .expect("skill file");
        let outcome = resolve(&agent, &store, &home, "s1", "/greet the team").await;
        let Some(PlatformSlashOutcome::AgentTurn(message)) = outcome else {
            panic!("expected agent turn expansion");
        };
        assert!(
            message.contains("greet"),
            "expansion should reference the skill: {message}"
        );
    }
}
