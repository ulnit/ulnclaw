//! Messaging platform gateways — port of hermes `gateway/platforms/`.
//!
//! Hermes runs chat platforms (Telegram/Discord/Slack/…) inside the
//! gateway process: each adapter normalizes incoming messages into a
//! `MessageEvent`, the core runs an agent turn per chat, and the reply
//! goes back through the platform. ulnclaw ports the architecture plus
//! three self-contained adapters:
//!
//! * **Telegram** — Bot API long-polling (`getUpdates` / `sendMessage`)
//! * **Discord**  — Gateway v10 websocket (IDENTIFY/heartbeat/
//!   MESSAGE_CREATE) + REST message send
//! * **Slack**    — Socket Mode websocket (events_api envelopes) +
//!   `chat.postMessage`
//!
//! Access control mirrors hermes pairing semantics: every platform is
//! allowlist-gated (`allowed_chat_ids` / `allowed_channel_ids`); an empty
//! allowlist refuses all traffic and logs the ids to add (fail closed).

use crate::agent::Agent;
use crate::error::{AgentError, Result};
use crate::session::sqlite::SqliteSessionStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// `[messaging]` config block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessagingConfig {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub slack: SlackConfig,
    /// Signal via signal-cli HTTP daemon (hermes `platforms.signal`).
    #[serde(default)]
    pub signal: crate::signal::SignalConfig,
    /// Offer pairing codes to unknown senders (hermes unauthorized-DM
    /// `pair` behavior). Approved codes join the allowlist as a union.
    #[serde(default = "default_pairing")]
    pub pairing: bool,
    /// WhatsApp Cloud webhook platform (mounted on the gateway router).
    #[serde(default)]
    pub whatsapp_cloud: crate::webhook_platforms::WhatsAppCloudConfig,
    /// Microsoft Graph change-notification ingress (gateway router).
    #[serde(default)]
    pub msgraph: crate::webhook_platforms::MsGraphConfig,
    /// Generic inbound-webhook platform: signed routes from external
    /// services (hermes `platforms.webhook`).
    #[serde(default)]
    pub webhook: crate::webhook_platforms::WebhookConfig,
    /// BlueBubbles iMessage bridge (hermes `platforms.bluebubbles`),
    /// mounted on the gateway router.
    #[serde(default)]
    pub bluebubbles: crate::webhook_platforms::BlueBubblesConfig,
    /// Weixin personal account via the iLink Bot API (hermes
    /// `platforms.weixin`).
    #[serde(default)]
    pub weixin: crate::weixin::WeixinConfig,
    /// Official QQ Bot API v2 adapter (hermes `platforms.qq`).
    #[serde(default)]
    pub qq: crate::qqbot::QQBotConfig,
    /// Yuanbao WS-gateway adapter (hermes `platforms.yuanbao`).
    #[serde(default)]
    pub yuanbao: crate::yuanbao::YuanbaoConfig,
    /// Email via IMAP/SMTP (hermes `platforms.email` plugin).
    #[serde(default)]
    pub email: crate::email_platform::EmailConfig,
    /// Mattermost via REST v4 + WebSocket (hermes `platforms.mattermost`
    /// plugin).
    #[serde(default)]
    pub mattermost: crate::mattermost::MattermostConfig,
    /// Matrix via the Client-Server API (hermes `platforms.matrix`
    /// plugin, sans E2EE).
    #[serde(default)]
    pub matrix: crate::matrix::MatrixConfig,
    /// DingTalk via Stream Mode (hermes `platforms.dingtalk` plugin).
    #[serde(default)]
    pub dingtalk: crate::dingtalk::DingTalkConfig,
    /// WeCom AI Bot via WebSocket gateway (hermes `platforms.wecom`
    /// plugin).
    #[serde(default)]
    pub wecom: crate::wecom::WeComConfig,
    /// Feishu/Lark via gateway webhook (hermes `platforms.feishu`
    /// plugin, webhook transport).
    #[serde(default)]
    pub feishu: crate::feishu::FeishuConfig,
    /// Home Assistant state-change events via the WS API (hermes
    /// `platforms.homeassistant` plugin).
    #[serde(default)]
    pub homeassistant: crate::homeassistant::HomeassistantConfig,
    /// Twilio SMS via REST + gateway webhook (hermes `platforms.sms`
    /// plugin).
    #[serde(default)]
    pub sms: crate::sms::SmsConfig,
    /// WhatsApp via an external Baileys HTTP bridge (hermes
    /// `platforms.whatsapp` plugin, bridge-client transport).
    #[serde(default)]
    pub whatsapp: crate::whatsapp::WhatsappConfig,
    /// IRC via a zero-dependency TLS client (hermes `platforms.irc`
    /// plugin).
    #[serde(default)]
    pub irc: crate::irc::IrcConfig,
    /// ntfy topics via HTTP streaming (hermes `platforms.ntfy` plugin).
    #[serde(default)]
    pub ntfy: crate::ntfy::NtfyConfig,
    /// SimpleX via the simplex-chat daemon WS API (hermes
    /// `platforms.simplex` plugin).
    #[serde(default)]
    pub simplex: crate::simplex::SimplexConfig,
    /// Microsoft Teams via the raw Bot Framework protocol (hermes
    /// `platforms.teams` plugin), mounted on the gateway router.
    #[serde(default)]
    pub teams: crate::teams::TeamsConfig,
    /// LINE Messaging API via gateway webhook (hermes `platforms.line`
    /// plugin).
    #[serde(default)]
    pub line: crate::line::LineConfig,
    /// Google Chat via HTTP-callback events + Chat REST API (hermes
    /// `platforms.google_chat` plugin), mounted on the gateway router.
    #[serde(default)]
    pub google_chat: crate::google_chat::GoogleChatConfig,
    /// iMessage via the `buzz` CLI (hermes `platforms.buzz` plugin,
    /// polling transport).
    #[serde(default)]
    pub buzz: crate::buzz::BuzzConfig,
    /// iMessage via the Photon sidecar HTTP API (hermes
    /// `platforms.photon` plugin, sidecar-client transport).
    #[serde(default)]
    pub photon: crate::photon::PhotonConfig,
    /// Raft activity wake events via gateway webhook (hermes
    /// `platforms.raft` plugin, wake-endpoint half).
    #[serde(default)]
    pub raft: crate::raft::RaftConfig,
    /// A2A v1.0 agent server surface on the gateway (hermes
    /// `platforms.a2a` plugin).
    #[serde(default)]
    pub a2a: crate::a2a::A2aConfig,
}

fn default_pairing() -> bool {
    true
}

/// `[messaging.telegram]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bot token from @BotFather (falls back to `TELEGRAM_BOT_TOKEN`).
    #[serde(default)]
    pub bot_token: String,
    /// Chat ids allowed to talk to the bot (hermes pairing). Empty =
    /// refuse all and log the ids that try.
    #[serde(default)]
    pub allowed_chat_ids: Vec<String>,
}

/// `[messaging.discord]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscordConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bot token (falls back to `DISCORD_BOT_TOKEN`).
    #[serde(default)]
    pub bot_token: String,
    /// Channel ids allowed to talk to the bot (empty = refuse all).
    #[serde(default)]
    pub allowed_channel_ids: Vec<String>,
    /// Seconds before clarify button prompts visually expire (hermes
    /// `approvals.discord_prompt_timeout`; 0 = 300 s default, clamped to
    /// 30–900 s like hermes).
    #[serde(default)]
    pub prompt_timeout_secs: u64,
}

/// `[messaging.slack]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlackConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bot token `xoxb-…` (falls back to `SLACK_BOT_TOKEN`).
    #[serde(default)]
    pub bot_token: String,
    /// App-level token `xapp-…` for Socket Mode (falls back to
    /// `SLACK_APP_TOKEN`).
    #[serde(default)]
    pub app_token: String,
    /// Channel ids allowed to talk to the bot (empty = refuse all).
    #[serde(default)]
    pub allowed_channel_ids: Vec<String>,
    /// Custom assistant-thread typing status (hermes `typing_status_text`).
    /// When unset the gateway shows "is thinking..." and switches to an
    /// elapsed-time "still working… (Xm YYs)" heartbeat after 30 s.
    #[serde(default)]
    pub typing_status_text: Option<String>,
}

/// A downloaded attachment cached in `<home>/media-cache/` (hermes media
/// pipeline input).
#[derive(Debug, Clone)]
pub struct MediaAttachment {
    pub path: std::path::PathBuf,
    pub mime: String,
    pub bytes: u64,
    pub original_name: String,
}

/// Normalized incoming message (hermes `MessageEvent`, core fields).
#[derive(Debug, Clone)]
pub struct MessageEvent {
    pub platform: String,
    pub chat_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub text: String,
    pub message_id: String,
    /// Cached media attachments (empty when the message had none or the
    /// download failed — failures degrade to a text note, never fatal).
    pub attachments: Vec<MediaAttachment>,
}

/// Per-chat conversation runner: one session per platform+chat
/// (hermes gateway session routing).
pub struct Dispatcher {
    agent: Arc<Agent>,
    store: Arc<SqliteSessionStore>,
    /// Per-chat in-flight guard: one turn at a time per chat.
    busy: Arc<Mutex<HashMap<String, bool>>>,
    /// Per-chat rolling history (mirrors the REPL continuity model).
    histories: Arc<Mutex<HashMap<String, Vec<crate::provider::Message>>>>,
}

/// Result of one dispatched platform message (hermes run-turn output):
/// the agent reply plus any 🎙️ transcript echoes to deliver first.
#[derive(Debug, Clone, Default)]
pub struct DispatchOutcome {
    pub reply: String,
    pub transcript_echoes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Messaging turn context + platform senders (clarify gateway integration)
// ---------------------------------------------------------------------------

/// The platform chat a messaging turn belongs to. Carried as a tokio
/// task-local so the messaging-aware clarify callback can render prompts
/// back to the right chat without threading platform state through the
/// tool layer.
#[derive(Debug, Clone)]
pub struct PlatformChatRef {
    pub platform: String,
    pub chat_id: String,
    pub session_key: String,
}

tokio::task_local! {
    static MESSAGING_CTX: PlatformChatRef;
}

/// The current turn's platform chat, when running inside a messaging
/// dispatch task.
pub fn current_messaging_ctx() -> Option<PlatformChatRef> {
    MESSAGING_CTX.try_with(|c| c.clone()).ok()
}

/// Outbound channel for a platform adapter, registered when the platform
/// loop starts. Used by the clarify gateway to render prompts mid-turn.
#[async_trait::async_trait]
pub trait PlatformSender: Send + Sync {
    async fn send_text(&self, chat_id: &str, text: &str);
    /// Render a clarify prompt natively (WhatsApp buttons/list). Returns
    /// false when the platform has no native interactive support — the
    /// caller falls back to numbered text.
    async fn send_clarify(
        &self,
        _chat_id: &str,
        _clarify_id: &str,
        _question: &str,
        _choices: &[String],
    ) -> bool {
        false
    }
    /// Render an exec-approval prompt natively (Teams adaptive-card
    /// buttons). Returns false when the platform has no native
    /// interactive support — the caller falls back to `/approve` text.
    #[allow(clippy::too_many_arguments)]
    async fn send_exec_approval(
        &self,
        _chat_id: &str,
        _command: &str,
        _session_key: &str,
        _description: &str,
        _allow_permanent: bool,
        _allow_session: bool,
        _smart_denied: bool,
    ) -> bool {
        false
    }
}

fn platform_senders() -> &'static std::sync::Mutex<HashMap<String, Arc<dyn PlatformSender>>> {
    static SENDERS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Arc<dyn PlatformSender>>>> =
        std::sync::OnceLock::new();
    SENDERS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub fn register_platform_sender(platform: &str, sender: Arc<dyn PlatformSender>) {
    platform_senders()
        .lock()
        .unwrap()
        .insert(platform.to_string(), sender);
}

pub fn platform_sender(platform: &str) -> Option<Arc<dyn PlatformSender>> {
    platform_senders().lock().unwrap().get(platform).cloned()
}

/// Truncate on a char boundary for approval previews (hermes
/// `command[:200] + "..."`).
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    format!("{cut}...")
}

/// Plain-text exec-approval prompt (hermes
/// `_format_exec_approval_fallback`): used on platforms without native
/// approval buttons and as the fallback when a card send fails.
pub fn format_approval_text(
    command: &str,
    description: &str,
    allow_session: bool,
    allow_permanent: bool,
    smart_denied: bool,
) -> String {
    let cmd_preview = truncate_chars(command, 200);
    let heading = if smart_denied {
        "⚠️ **Smart DENY — owner override for one operation:**"
    } else {
        "⚠️ **Dangerous command requires approval:**"
    };
    let mut choices: Vec<String> =
        vec!["Reply `/approve` to execute this one operation".to_string()];
    if !smart_denied && allow_session {
        choices.push("`/approve session` to approve this pattern for the session".to_string());
        if allow_permanent {
            choices.push("`/approve always` to approve permanently".to_string());
        }
    }
    choices.push("`/deny` to cancel".to_string());
    let last = choices.pop().expect("choices is never empty");
    format!(
        "{heading}\n```\n{cmd_preview}\n```\nReason: {description}\n\n{}, or {last}.",
        choices.join(", ")
    )
}

/// Parsed `/approve` / `/deny` chat command (hermes
/// `_handle_approve_command` / `_handle_deny_command`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalCommand {
    pub approve: bool,
    pub all: bool,
    /// `once` | `session` | `always` for approve; `deny` for deny.
    pub choice: &'static str,
}

/// Parse a `/approve` / `/deny` message. Accepts the slash form on
/// every platform (hermes slash-command semantics).
pub fn parse_approval_command(text: &str) -> Option<ApprovalCommand> {
    let mut parts = text.trim().split_whitespace();
    let head = parts.next()?.to_lowercase();
    let rest: Vec<String> = parts.map(|p| p.to_lowercase()).collect();
    match head.as_str() {
        "/approve" => {
            let all = rest.iter().any(|p| p == "all");
            let choice = if rest.iter().any(|p| p == "always") {
                crate::approval_gateway::CHOICE_ALWAYS
            } else if rest.iter().any(|p| p == "session") {
                crate::approval_gateway::CHOICE_SESSION
            } else {
                crate::approval_gateway::CHOICE_ONCE
            };
            Some(ApprovalCommand { approve: true, all, choice })
        }
        "/deny" => Some(ApprovalCommand {
            approve: false,
            all: rest.iter().any(|p| p == "all"),
            choice: crate::approval_gateway::CHOICE_DENY,
        }),
        _ => None,
    }
}

/// Apply a parsed approval command to the session's pending registry;
/// returns the user-facing confirmation (hermes approve/deny replies).
/// Returns `None` when nothing is pending and the message should be
/// treated as normal chat text.
pub fn apply_approval_command(session_key: &str, cmd: ApprovalCommand) -> String {
    if !crate::approval_gateway::has_blocking(session_key) {
        return "No pending approvals.".to_string();
    }
    let count = if cmd.all {
        crate::approval_gateway::resolve_all(session_key, cmd.choice)
    } else if crate::approval_gateway::resolve(session_key, cmd.choice) {
        1
    } else {
        0
    };
    if count == 0 {
        return "No pending approvals.".to_string();
    }
    let plural = if count == 1 { "command" } else { "commands" };
    let label = if cmd.approve {
        match cmd.choice {
            crate::approval_gateway::CHOICE_SESSION => "✅ Allowed (session)",
            crate::approval_gateway::CHOICE_ALWAYS => "✅ Always allowed",
            _ => "✅ Allowed (once)",
        }
    } else {
        "❌ Denied"
    };
    if count == 1 {
        label.to_string()
    } else {
        format!("{label} — {count} {plural}")
    }
}

/// Messaging-aware approve callback (hermes `tools/approval.py`
/// gateway-notify integration): inside a messaging turn the dangerous
/// command prompt renders on the platform (adaptive-card buttons where
/// supported, `/approve` text elsewhere) and blocks the agent until
/// the user taps or replies. Outside a messaging context the wrapped
/// callback (gateway run-based flow) decides.
pub fn messaging_aware_approve_fn(
    inner: crate::tools::context::ApproveFn,
) -> crate::tools::context::ApproveFn {
    Arc::new(move |reason, command| {
        let inner = inner.clone();
        Box::pin(async move {
            let Some(ctx) = current_messaging_ctx() else {
                return inner(reason, command).await;
            };
            // Redact credentials before the command reaches the chat
            // platform (hermes `_redact_approval_command`).
            let cmd = crate::redact::redact_sensitive_text(&command, Default::default());
            let handle = crate::approval_gateway::register(
                &ctx.session_key,
                &cmd,
                &reason,
                false,
                true,
                true,
            );
            let rendered = if let Some(sender) = platform_sender(&ctx.platform) {
                let native = sender
                    .send_exec_approval(&ctx.chat_id, &cmd, &ctx.session_key, &reason, true, true, false)
                    .await;
                if !native {
                    sender
                        .send_text(
                            &ctx.chat_id,
                            &format_approval_text(&cmd, &reason, true, true, false),
                        )
                        .await;
                }
                true
            } else {
                false
            };
            if !rendered {
                // No outbound channel: fail closed.
                crate::approval_gateway::resolve(&ctx.session_key, crate::approval_gateway::CHOICE_DENY);
                return false;
            }
            matches!(handle.rx.await, Ok(choice) if choice != crate::approval_gateway::CHOICE_DENY)
        })
    })
}

/// Numbered-text clarify rendering for platforms without native
/// interactive messages.
pub fn format_clarify_text(question: &str, choices: &[String], multi_select: bool) -> String {
    let mut out = format!("❓ {}", question.trim());
    if !choices.is_empty() {
        out.push_str("\n\n");
        for (i, choice) in choices.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, choice.trim()));
        }
        out.push_str(if multi_select {
            "\nReply with the numbers of all choices that apply."
        } else {
            "\nReply with the number of your choice (or type your own answer)."
        });
    }
    out
}

/// Messaging-aware clarify callback for gateway agents (hermes
/// `tools/clarify_gateway.py` integration): renders the prompt on the
/// current platform (native buttons on WhatsApp, numbered text
/// elsewhere) and blocks the tool until the user taps or replies. Runs
/// without a messaging context (plain `/api/chat`, cron, one-shot)
/// report the standard non-interactive error.
pub fn messaging_clarify_fn() -> crate::tools::context::ClarifyFn {
    Arc::new(|question, choices, multi| {
        Box::pin(async move {
            let Some(ctx) = current_messaging_ctx() else {
                return Err(AgentError::tool(
                    "No user is available to answer (non-interactive session). Proceed with \
                     your best judgment and state your assumptions in the final answer.",
                ));
            };
            let handle = crate::clarify_gateway::register(
                &ctx.session_key,
                &question,
                &choices,
                multi,
            );
            let Some(sender) = platform_sender(&ctx.platform) else {
                crate::clarify_gateway::resolve(&handle.clarify_id, "");
                return Err(AgentError::tool(format!(
                    "clarify: platform '{}' has no registered sender",
                    ctx.platform
                )));
            };
            let mut rendered_natively = false;
            if !choices.is_empty() {
                rendered_natively = sender
                    .send_clarify(&ctx.chat_id, &handle.clarify_id, &question, &choices)
                    .await;
            }
            if !rendered_natively {
                let text = if choices.is_empty() {
                    format!("❓ {}", question.trim())
                } else {
                    format_clarify_text(&question, &choices, multi)
                };
                sender.send_text(&ctx.chat_id, &text).await;
            }
            match handle.rx.await {
                Ok(answer) if !answer.is_empty() => Ok(answer),
                Ok(_) => Err(AgentError::tool("clarify: no answer received")),
                Err(_) => Err(AgentError::tool(
                    "clarify was cancelled (gateway restart or session end)",
                )),
            }
        })
    })
}

impl Dispatcher {
    pub fn new(agent: Arc<Agent>, store: Arc<SqliteSessionStore>) -> Arc<Self> {
        Arc::new(Self {
            agent,
            store,
            busy: Arc::new(Mutex::new(HashMap::new())),
            histories: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn session_key(event: &MessageEvent) -> String {
        format!("platform-{}-{}", event.platform, event.chat_id)
    }

    /// Run one agent turn for the event's chat; returns the reply text
    /// plus transcript echoes (hermes `_echo_pending_stt_transcripts_once`).
    pub async fn handle_event(self: &Arc<Self>, event: MessageEvent) -> Result<DispatchOutcome> {
        let key = Self::session_key(&event);
        // Exec-approval intercept (hermes `/approve` + `/deny` slash
        // commands): resolves a pending blocking approval while the
        // blocked turn holds the busy flag, so it must run before the
        // busy check below.
        if let Some(cmd) = parse_approval_command(&event.text) {
            return Ok(DispatchOutcome {
                reply: apply_approval_command(&key, cmd),
                transcript_echoes: Vec::new(),
            });
        }
        // Clarify text intercept (hermes `_maybe_intercept_clarify_text`):
        // when a prompt awaits a typed answer, the next message resolves it
        // instead of starting a fresh turn (the blocked clarify tool call
        // in the in-flight turn receives the answer and continues).
        if !event.text.trim().is_empty() {
            if let Some(pending) = crate::clarify_gateway::pending_for_session(&key) {
                if pending.awaiting_text
                    && crate::clarify_gateway::resolve(&pending.clarify_id, &event.text)
                {
                    return Ok(DispatchOutcome::default());
                }
            }
        }
        {
            let mut busy = self.busy.lock().await;
            if *busy.get(&key).unwrap_or(&false) {
                return Ok(DispatchOutcome {
                    reply: "(the previous message is still being processed — please wait)".into(),
                    transcript_echoes: Vec::new(),
                });
            }
            busy.insert(key.clone(), true);
        }
        let chat_ref = PlatformChatRef {
            platform: event.platform.clone(),
            chat_id: event.chat_id.clone(),
            session_key: key.clone(),
        };
        let result = MESSAGING_CTX.scope(chat_ref, self.run_turn(&key, &event)).await;
        self.busy.lock().await.insert(key, false);
        result
    }

    async fn run_turn(&self, key: &str, event: &MessageEvent) -> Result<DispatchOutcome> {
        use crate::provider::Role;
        // Ensure the session row exists under a deterministic id.
        if self.store.resolve_session_id(key).ok().flatten().is_none() {
            self.store
                .create_named_session(
                    key,
                    &format!("platform:{}", event.platform),
                    Some(&self.agent.context().config.model.model),
                    None,
                )
                .ok();
        }
        let mut histories = self.histories.lock().await;
        let history = histories.entry(key.to_string()).or_default();
        if history.is_empty() {
            if let Ok(messages) = self.store.load_messages(key) {
                *history = messages
                    .into_iter()
                    .filter(|m| m.role != Role::System)
                    .collect();
            }
        }
        // Voice-note STT enrichment (hermes
        // `_transcribe_pending_audio_event_once`): audio attachments are
        // transcribed ahead of the turn; transcripts are echoed back to
        // the chat and the enriched text replaces the raw caption.
        let config = &self.agent.context().config;
        let audio_paths: Vec<std::path::PathBuf> = event
            .attachments
            .iter()
            .filter(|a| crate::stt::attachment_is_stt_input(&a.mime))
            .map(|a| a.path.clone())
            .collect();
        let mut user_text = event.text.clone();
        let mut transcript_echoes: Vec<String> = Vec::new();
        let mut transcribed: Vec<std::path::PathBuf> = Vec::new();
        if !audio_paths.is_empty() {
            let (enriched, transcripts, done) = crate::stt::enrich_message_with_transcription(
                &config.stt,
                &user_text,
                &audio_paths,
            )
            .await;
            user_text = enriched;
            transcribed = done;
            if config.stt.echo_transcripts {
                transcript_echoes = transcripts
                    .iter()
                    .map(|t| crate::stt::echo_line(t))
                    .collect();
            }
        }
        let mut prompt = if event.sender_name.is_empty() {
            user_text
        } else {
            format!("{}: {}", event.sender_name, user_text)
        };
        // Cached attachments are referenced by path so the agent can apply
        // vision_analyze / video_analyze / read_file (hermes text-fallback
        // semantics for media). Transcribed voice notes already appear in
        // the enriched text — skip them to avoid duplicate path noise.
        let remaining: Vec<&MediaAttachment> = event
            .attachments
            .iter()
            .filter(|a| !transcribed.contains(&a.path))
            .collect();
        prompt.push_str(&attachment_note_refs(&remaining));
        let result = self
            .agent
            .run_with_session(&prompt, Some(history.clone()), Some(key))
            .await?;
        *history = result
            .conversation
            .into_iter()
            .filter(|m| m.role != Role::System)
            .collect();
        // Fire the gateway pre-dispatch observers' counterpart: the
        // session hooks already cover lifecycle; keep this minimal.
        Ok(DispatchOutcome {
            reply: result.content,
            transcript_echoes,
        })
    }
}

/// Access gate (hermes pairing): allowlisted ids pass; everything else
/// is refused and reported so the user knows what to allowlist.
fn allowlisted(allowlist: &[String], id: &str) -> bool {
    allowlist.iter().any(|allowed| allowed == id)
}

/// Offer a pairing code to an unauthorized sender (hermes pairing flow).
/// Returns the reply text, or None to stay silent (rate-limited repeat).
fn pairing_offer(
    store: &crate::pairing::PairingStore,
    platform: &str,
    sender_id: &str,
    sender_name: &str,
) -> Option<String> {
    // Repeat requests within the rate window are silently ignored (hermes).
    if store.is_rate_limited(platform, sender_id) {
        return None;
    }
    match store.generate_code(platform, sender_id, sender_name) {
        Some(code) => Some(format!(
            "Hi~ I don't recognize you yet!\n\n\
             Here's your pairing code: `{code}`\n\n\
             Ask the bot owner to run:\n\
             `ulnclaw pairing approve {platform} {code}`"
        )),
        None => {
            // Queue full or locked out: send the throttle notice once, then
            // silence follow-ups via the rate limit (hermes behavior).
            store.record_rate_limit(platform, sender_id);
            Some("Too many pairing requests right now~ Please try again later!".to_string())
        }
    }
}

/// Attachment download cap (Discord's free-tier upload limit — hermes
/// applies a similar ceiling).
const MAX_MEDIA_BYTES: u64 = 25 * 1024 * 1024;

/// Cache downloaded bytes and wrap failures into a text note (hermes
/// degrades attachment problems to text, never fatal).
fn cache_attachment(
    data: Vec<u8>,
    mime: &str,
    original_name: &str,
) -> Option<MediaAttachment> {
    if data.is_empty() || data.len() as u64 > MAX_MEDIA_BYTES {
        return None;
    }
    let home = crate::config::ulnclaw_home();
    match crate::media_cache::cache_media_bytes(&home, &data, mime, original_name) {
        Ok(path) => Some(MediaAttachment {
            path,
            mime: if mime.is_empty() {
                "application/octet-stream".to_string()
            } else {
                crate::media_cache::normalize_mime(mime)
            },
            bytes: data.len() as u64,
            original_name: original_name.to_string(),
        }),
        Err(e) => {
            eprintln!("[messaging] media cache write failed: {e}");
            None
        }
    }
}

/// Download one attachment URL into the media cache. `auth` is an
/// optional Authorization header (Slack private file URLs need the bot
/// bearer). Failures degrade to None with a log line (hermes policy).
async fn download_to_cache(
    client: &reqwest::Client,
    url: &str,
    auth: Option<&str>,
    mime: &str,
    original_name: &str,
    declared_size: u64,
) -> Option<MediaAttachment> {
    if declared_size > MAX_MEDIA_BYTES {
        eprintln!(
            "[messaging] skipping attachment {original_name:?}: {declared_size} bytes exceeds the {MAX_MEDIA_BYTES} byte cap"
        );
        return None;
    }
    let mut request = client.get(url);
    if let Some(auth) = auth {
        request = request.header("Authorization", auth);
    }
    match request.send().await.and_then(|r| r.error_for_status()) {
        Ok(response) => match response.bytes().await {
            Ok(bytes) => cache_attachment(bytes.to_vec(), mime, original_name),
            Err(e) => {
                eprintln!("[messaging] attachment download failed: {e}");
                None
            }
        },
        Err(e) => {
            eprintln!("[messaging] attachment download failed: {e}");
            None
        }
    }
}

/// Render attachments as a text note appended to the user message —
/// the agent can inspect images via `vision_analyze`, video via
/// `video_analyze`, and documents via `read_file` (hermes falls back to
/// exactly this text reference when native multimodal injection fails).
#[cfg(test)]
fn attachment_note(attachments: &[MediaAttachment]) -> String {
    let refs: Vec<&MediaAttachment> = attachments.iter().collect();
    attachment_note_refs(&refs)
}

fn attachment_note_refs(attachments: &[&MediaAttachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut note = String::from("\n\n[Attached media]\n");
    for attachment in attachments {
        let name = if attachment.original_name.is_empty() {
            String::new()
        } else {
            format!(" \"{}\"", attachment.original_name)
        };
        note.push_str(&format!(
            "- {} ({}, {} bytes){}\n",
            attachment.path.display(),
            attachment.mime,
            attachment.bytes,
            name
        ));
    }
    note.push_str(
        "Inspect images with vision_analyze, video with video_analyze,          documents with read_file.",
    );
    note
}

/// Extract `MEDIA:<path>` delivery tags from a reply (hermes
/// `extract_media`): returns the cleaned text plus the file paths, in
/// order of appearance. Tags may stand alone on a line; surrounding
/// whitespace-only lines are dropped with them.
pub fn extract_media_tags(text: &str) -> (String, Vec<std::path::PathBuf>) {
    let mut cleaned: Vec<String> = Vec::new();
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("MEDIA:") {
            let candidate = rest.trim().trim_matches('`');
            if !candidate.is_empty() {
                let expanded = if let Some(stripped) = candidate.strip_prefix("~/") {
                    std::env::var_os("HOME")
                        .map(|home| {
                            std::path::PathBuf::from(home)
                                .join(stripped)
                                .display()
                                .to_string()
                        })
                        .unwrap_or_else(|| candidate.to_string())
                } else {
                    candidate.to_string()
                };
                if std::path::Path::new(&expanded).is_file() {
                    paths.push(std::path::PathBuf::from(expanded));
                    continue;
                }
            }
        }
        cleaned.push(line.to_string());
    }
    // Collapse runs of blank lines left behind by removed tags.
    let mut out = String::new();
    let mut prev_blank = false;
    for line in &cleaned {
        let blank = line.trim().is_empty();
        if blank && prev_blank {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = blank;
    }
    (out.trim_end_matches('\n').to_string(), paths)
}

/// Pub wrapper for webhook platforms (whatsapp_cloud / msgraph).
pub fn pairing_offer_public(
    store: &crate::pairing::PairingStore,
    platform: &str,
    sender_id: &str,
    sender_name: &str,
) -> Option<String> {
    pairing_offer(store, platform, sender_id, sender_name)
}

/// Pub wrapper for webhook platforms (whatsapp_cloud / msgraph).
pub async fn pre_gateway_dispatch_gate_public(event: &mut MessageEvent) -> bool {
    pre_gateway_dispatch_gate(event).await
}

/// `pre_gateway_dispatch` plugin hook (hermes fires it BEFORE auth so
/// plugins can handle unauthorized senders without triggering pairing).
/// Returns false when a hook consumed the event (`{"action": "skip"}`);
/// a `{"action": "rewrite", "text": ...}` response replaces `event.text`.
async fn pre_gateway_dispatch_gate(event: &mut MessageEvent) -> bool {
    if !crate::plugins::has_hook("pre_gateway_dispatch") {
        return true;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let payload = crate::plugins::hook_payload(
        "pre_gateway_dispatch",
        "",
        &cwd,
        vec![
            ("platform", json!(event.platform)),
            ("chat_id", json!(event.chat_id)),
            ("sender_id", json!(event.sender_id)),
            ("sender_name", json!(event.sender_name)),
            ("text", json!(event.text)),
            ("message_id", json!(event.message_id)),
        ],
        json!({}),
    );
    let responses = crate::plugins::invoke_hook("pre_gateway_dispatch", payload).await;
    match crate::plugins::dispatch_decision(&responses) {
        Some((action, detail)) if action == "skip" => {
            eprintln!(
                "[messaging] pre_gateway_dispatch skipped {} message from chat {}: {}",
                event.platform,
                event.chat_id,
                detail.unwrap_or_default()
            );
            false
        }
        Some((action, text)) if action == "rewrite" => {
            if let Some(text) = text {
                event.text = text;
            }
            true
        }
        _ => true,
    }
}

/// Start every enabled platform; resolves when they have all stopped.
pub async fn run_messaging(
    config: &crate::config::UlncLawConfig,
    agent: Arc<Agent>,
    store: Arc<SqliteSessionStore>,
) {
    let dispatcher = Dispatcher::new(agent, store);
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let msg = &config.messaging;
    let pairing: Option<Arc<crate::pairing::PairingStore>> = if msg.pairing {
        Some(Arc::new(crate::pairing::PairingStore::open(
            &crate::config::ulnclaw_home(),
        )))
    } else {
        None
    };
    if msg.telegram.enabled {
        let cfg = msg.telegram.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            telegram::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.discord.enabled {
        let cfg = msg.discord.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            discord::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.slack.enabled {
        let cfg = msg.slack.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            slack::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.signal.enabled {
        let cfg = msg.signal.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::signal::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.weixin.enabled {
        let cfg = msg.weixin.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::weixin::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.qq.enabled {
        let cfg = msg.qq.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::qqbot::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.yuanbao.enabled {
        let cfg = msg.yuanbao.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::yuanbao::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.email.enabled {
        let cfg = msg.email.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::email_platform::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.mattermost.enabled {
        let cfg = msg.mattermost.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::mattermost::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.matrix.enabled {
        let cfg = msg.matrix.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::matrix::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.dingtalk.enabled {
        let cfg = msg.dingtalk.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::dingtalk::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.wecom.enabled {
        let cfg = msg.wecom.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::wecom::run(cfg, dispatcher, pairing).await;
        }));
    }
    // Standalone `notify/notify` sender (hermes `_standalone_send`):
    // delivery works without the live WS adapter whenever credentials
    // are configured; a starting live adapter overwrites the slot.
    crate::homeassistant::maybe_register_standalone_sender(&msg.homeassistant);
    if msg.homeassistant.enabled {
        let cfg = msg.homeassistant.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::homeassistant::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.whatsapp.enabled {
        let cfg = msg.whatsapp.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::whatsapp::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.irc.enabled {
        let cfg = msg.irc.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::irc::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.ntfy.enabled {
        let cfg = msg.ntfy.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::ntfy::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.simplex.enabled {
        let cfg = msg.simplex.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::simplex::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.teams.enabled {
        // Gateway-mounted webhook platform: register sender + runtime.
        crate::teams::register(&msg.teams);
    }
    if msg.line.enabled {
        crate::line::register(&msg.line);
    }
    if msg.google_chat.enabled {
        crate::google_chat::register(&msg.google_chat);
        if !msg
            .google_chat
            .resolve()
            .pubsub_subscription
            .trim()
            .is_empty()
        {
            let dispatcher = dispatcher.clone();
            let pairing = pairing.clone();
            tasks.push(tokio::spawn(async move {
                crate::google_chat::run_pubsub(dispatcher, pairing).await;
            }));
        }
    }
    if msg.buzz.enabled {
        let cfg = msg.buzz.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::buzz::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.photon.enabled {
        let cfg = msg.photon.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::photon::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.feishu.enabled
        && crate::feishu::is_websocket_mode(&msg.feishu.resolve().connection_mode)
    {
        // WebSocket long connection (hermes default); webhook mode
        // rides the gateway /webhooks/feishu route instead.
        let cfg = msg.feishu.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            crate::feishu_ws::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.a2a.enabled {
        crate::a2a::register(&msg.a2a);
    }
    if tasks.is_empty() {
        eprintln!("[messaging] no platforms enabled ([messaging.telegram|discord|slack|signal|weixin|qq|yuanbao|email|mattermost|matrix|dingtalk|wecom|homeassistant|whatsapp|irc|ntfy|simplex|buzz|photon|feishu] enabled = true (sms/teams/line/google_chat/raft/a2a ride gateway webhook routes))");
        return;
    }
    for task in tasks {
        task.await.ok();
    }
}

fn resolve_token(configured: &str, env_var: &str) -> Option<String> {
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        return Some(trimmed.to_string());
    }
    std::env::var(env_var).ok().filter(|v| !v.trim().is_empty())
}

/// Public token resolution for cross-module delivery targets
/// (generic-webhook `deliver = "telegram"`; hermes webhook.py reuses the
/// platform credentials).
pub fn resolve_telegram_token_public(cfg: &TelegramConfig) -> Option<String> {
    resolve_token(&cfg.bot_token, "TELEGRAM_BOT_TOKEN")
}

/// Public send wrapper (see `resolve_telegram_token_public`).
pub async fn telegram_send_public(client: &reqwest::Client, token: &str, chat_id: &str, text: &str) {
    telegram::send_message(client, token, chat_id, text).await
}

/// Public token resolution for webhook `deliver = "discord"`.
pub fn resolve_discord_token_public(cfg: &DiscordConfig) -> Option<String> {
    resolve_token(&cfg.bot_token, "DISCORD_BOT_TOKEN")
}

/// Public send wrapper (see `resolve_discord_token_public`).
pub async fn discord_send_public(token: &str, channel_id: &str, text: &str) {
    discord::send_channel_message(token, channel_id, text).await
}

/// Public token resolution for webhook `deliver = "slack"`.
pub fn resolve_slack_bot_token_public(cfg: &SlackConfig) -> Option<String> {
    resolve_token(&cfg.bot_token, "SLACK_BOT_TOKEN")
}

/// Public send wrapper (see `resolve_slack_bot_token_public`).
pub async fn slack_send_public(token: &str, channel: &str, text: &str) {
    slack::post_message(token, channel, text).await
}

// ---------------------------------------------------------------------------
// Telegram — Bot API long-polling
// ---------------------------------------------------------------------------

pub mod telegram {
    use super::*;

    const API: &str = "https://api.telegram.org";

    /// Telegram Bot API base URL — normally `https://api.telegram.org`;
    /// the `TELEGRAM_API_BASE` override exists for tests and corporate
    /// proxies (mirrors the slack `SLACK_API_BASE` pattern).
    pub(crate) fn telegram_api_base() -> String {
        std::env::var("TELEGRAM_API_BASE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| API.to_string())
    }

    struct Sender {
        client: reqwest::Client,
        token: String,
    }

    #[async_trait::async_trait]
    impl PlatformSender for Sender {
        async fn send_text(&self, chat_id: &str, text: &str) {
            send_message(&self.client, &self.token, chat_id, text).await;
        }

        /// Inline-keyboard clarify prompt (hermes Telegram `send_clarify`).
        /// Returns false on API failure so the numbered-text fallback still
        /// goes out.
        async fn send_clarify(
            &self,
            chat_id: &str,
            clarify_id: &str,
            question: &str,
            choices: &[String],
        ) -> bool {
            send_clarify_message(&self.client, &self.token, chat_id, clarify_id, question, choices)
                .await
        }
    }

    pub async fn run(cfg: TelegramConfig, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<crate::pairing::PairingStore>>) {
        let Some(token) = resolve_token(&cfg.bot_token, "TELEGRAM_BOT_TOKEN") else {
            eprintln!("[telegram] disabled: no bot_token configured (set messaging.telegram.bot_token or TELEGRAM_BOT_TOKEN)");
            return;
        };
        let client = reqwest::Client::new();
        register_platform_sender(
            "telegram",
            Arc::new(Sender {
                client: client.clone(),
                token: token.clone(),
            }),
        );
        match api(&client, &token, "getMe", json!({})).await {
            Ok(me) => {
                let username = me.pointer("/result/username").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("[telegram] connected as @{username}");
            }
            Err(e) => {
                eprintln!("[telegram] getMe failed: {e}");
                return;
            }
        }
        let mut offset: Option<i64> = None;
        loop {
            let mut params =
                json!({"timeout": 25, "allowed_updates": ["message", "callback_query"]});
            if let Some(offset) = offset {
                params["offset"] = json!(offset + 1);
            }
            let updates = match api(&client, &token, "getUpdates", params).await {
                Ok(value) => value
                    .pointer("/result")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                Err(e) => {
                    eprintln!("[telegram] getUpdates failed: {e} — retrying in 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };
            for update in updates {
                let update_id = update.get("update_id").and_then(|v| v.as_i64()).unwrap_or(0);
                offset = Some(offset.unwrap_or(0).max(update_id));
                // Clarify button taps arrive as callback_query updates and
                // resolve the pending clarify (hermes Telegram callback
                // handler); they never enter the message pipeline.
                if let Some(query) = update.get("callback_query") {
                    handle_callback_query(&client, &token, &cfg, pairing.as_deref(), query)
                        .await;
                    continue;
                }
                let Some(message) = update.get("message") else { continue };
                let text = message.get("text").and_then(|v| v.as_str()).unwrap_or("");
                // Media (photo/document/video/audio/voice) is downloaded
                // and cached; media-only messages still flow (hermes).
                let attachments = download_media(&client, &token, message).await;
                if text.is_empty() && attachments.is_empty() {
                    continue;
                }
                let chat = message.get("chat").cloned().unwrap_or(json!({}));
                let chat_id = chat
                    .get("id")
                    .map(|v| match v {
                        Value::Number(n) => n.to_string(),
                        other => other.as_str().unwrap_or("").to_string(),
                    })
                    .unwrap_or_default();
                if chat_id.is_empty() {
                    continue;
                }
                let sender = message.get("from").cloned().unwrap_or(json!({}));
                let mut event = MessageEvent {
                    platform: "telegram".into(),
                    chat_id: chat_id.clone(),
                    sender_id: sender.get("id").map(|v| v.to_string()).unwrap_or_default(),
                    sender_name: sender
                        .get("first_name")
                        .or_else(|| sender.get("username"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    text: text.to_string(),
                    message_id: message
                        .get("message_id")
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    attachments,
                };
                // Plugin gate before auth (hermes ordering).
                if !pre_gateway_dispatch_gate(&mut event).await {
                    continue;
                }
                // Auth union: configured allowlist OR an approved pairing
                // code (hermes authz union).
                let authorized = allowlisted(&cfg.allowed_chat_ids, &chat_id)
                    || pairing
                        .as_ref()
                        .map(|store| store.is_approved("telegram", &event.sender_id))
                        .unwrap_or(false);
                if !authorized {
                    eprintln!(
                        "[telegram] refusing message from chat {chat_id} — add it to \
                         messaging.telegram.allowed_chat_ids or approve a pairing code"
                    );
                    if let Some(store) = pairing.as_ref() {
                        if let Some(reply) = pairing_offer(store, "telegram", &event.sender_id, &event.sender_name) {
                            send_message(&client, &token, &chat_id, &reply).await;
                        }
                    }
                    continue;
                }
                let dispatcher = dispatcher.clone();
                let client = client.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    let outcome = match dispatcher.handle_event(event.clone()).await {
                        Ok(outcome) => outcome,
                        Err(e) => crate::messaging::DispatchOutcome {
                            reply: format!("error: {e}"),
                            transcript_echoes: Vec::new(),
                        },
                    };
                    for echo in &outcome.transcript_echoes {
                        send_message(&client, &token, &event.chat_id, echo).await;
                    }
                    let reply = outcome.reply;
                    // MEDIA:<path> tags become native attachments (hermes
                    // extract_media); the rest is sent as text.
                    let (reply_text, media_paths) = extract_media_tags(&reply);
                    if !reply_text.trim().is_empty() {
                        send_message(&client, &token, &event.chat_id, &reply_text).await;
                    }
                    for path in &media_paths {
                        if crate::media_cache::mime_for_ext(path).starts_with("image/") {
                            send_photo(&client, &token, &event.chat_id, path).await;
                        } else {
                            send_document(&client, &token, &event.chat_id, path).await;
                        }
                    }
                });
            }
        }
    }

    async fn api(client: &reqwest::Client, token: &str, method: &str, params: Value) -> Result<Value> {
        let url = format!("{}/bot{token}/{method}", telegram_api_base());
        let response = client
            .post(&url)
            .json(&params)
            .timeout(std::time::Duration::from_secs(35))
            .send()
            .await
            .map_err(|e| AgentError::Tool(format!("telegram {method}: {e}")))?;
        let value: Value = response
            .json()
            .await
            .map_err(|e| AgentError::Tool(format!("telegram {method} parse: {e}")))?;
        if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(value)
        } else {
            Err(AgentError::Tool(format!(
                "telegram {method}: {}",
                value.get("description").and_then(|v| v.as_str()).unwrap_or("unknown error")
            )))
        }
    }

    /// HTML-escape for HTML parse-mode payloads (hermes `_html.escape`).
    fn html_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#x27;")
    }

    /// Render a clarify prompt with one inline button per choice (hermes
    /// Telegram `send_clarify` layout): full option text in the message
    /// body so mobile users can read long choices, short numeric button
    /// labels to avoid Telegram truncation, plus a final "✏️ Other (type
    /// answer)" row that flips the prompt into text-capture mode.
    /// `cl:<id>:<idx|other>` callback payloads stay inside Telegram's
    /// 64-byte callback_data cap. Returns false on API failure so the
    /// caller falls back to numbered text.
    async fn send_clarify_message(
        client: &reqwest::Client,
        token: &str,
        chat_id: &str,
        clarify_id: &str,
        question: &str,
        choices: &[String],
    ) -> bool {
        let mut text = format!("❓ {}", html_escape(question));
        let mut rows: Vec<Value> = Vec::new();
        if !choices.is_empty() {
            let option_lines = choices
                .iter()
                .enumerate()
                .map(|(idx, choice)| format!("{}. {}", idx + 1, html_escape(choice)))
                .collect::<Vec<_>>()
                .join("\n");
            text = format!("{text}\n\n{option_lines}");
            for idx in 0..choices.len() {
                rows.push(json!([{
                    "text": (idx + 1).to_string(),
                    "callback_data": format!("cl:{clarify_id}:{idx}"),
                }]));
            }
            rows.push(json!([{
                "text": "✏️ Other (type answer)",
                "callback_data": format!("cl:{clarify_id}:other"),
            }]));
        }
        let mut params = json!({"chat_id": chat_id, "text": text, "parse_mode": "HTML"});
        if !rows.is_empty() {
            params["reply_markup"] = json!({"inline_keyboard": rows});
        }
        match api(client, token, "sendMessage", params).await {
            Ok(_) => true,
            Err(e) => {
                eprintln!("[telegram] send_clarify failed: {e}");
                false
            }
        }
    }

    /// Route a clarify button tap (hermes Telegram `cl:` callback branch).
    /// Every path answers the callback so the Telegram client stops the
    /// loading spinner. Unauthorized taps get ⛔; taps on resolved prompts
    /// say so; taps on expired prompts (tool side gave up) get the ⚠️
    /// notice instead of a misleading ✓.
    async fn handle_callback_query(
        client: &reqwest::Client,
        token: &str,
        cfg: &TelegramConfig,
        pairing: Option<&crate::pairing::PairingStore>,
        query: &Value,
    ) {
        let Some(data) = query.get("data").and_then(|v| v.as_str()) else { return };
        if !data.starts_with("cl:") {
            return;
        }
        let parts: Vec<&str> = data.splitn(3, ':').collect();
        if parts.len() != 3 {
            return;
        }
        let clarify_id = parts[1];
        let choice_token = parts[2];
        let Some(query_id) = query.get("id").and_then(|v| v.as_str()) else { return };
        let from = query.get("from").cloned().unwrap_or(json!({}));
        let caller_id = from.get("id").map(|v| v.to_string()).unwrap_or_default();
        let user_display = from
            .get("first_name")
            .and_then(|v| v.as_str())
            .unwrap_or("User")
            .to_string();
        let message = query.get("message").cloned().unwrap_or(json!({}));
        let original_text = message
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let chat_id = message
            .get("chat")
            .and_then(|c| c.get("id"))
            .map(|v| match v {
                Value::Number(n) => n.to_string(),
                other => other.as_str().unwrap_or("").to_string(),
            })
            .unwrap_or_default();
        let message_id = message.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0);

        // Auth union: configured allowlist OR an approved pairing code
        // (hermes `_is_callback_user_authorized`).
        let authorized = allowlisted(&cfg.allowed_chat_ids, &chat_id)
            || pairing
                .map(|store| store.is_approved("telegram", &caller_id))
                .unwrap_or(false);
        if !authorized {
            answer_callback(
                client,
                token,
                query_id,
                "⛔ You are not authorized to answer this prompt.",
            )
            .await;
            return;
        }

        if !crate::clarify_gateway::contains(clarify_id) {
            answer_callback(
                client,
                token,
                query_id,
                "This prompt has already been resolved.",
            )
            .await;
            return;
        }

        if choice_token == "other" {
            // Flip into text-capture mode; the next plain message in the
            // session resolves the clarify (hermes mark_awaiting_text).
            if !crate::clarify_gateway::mark_awaiting_text(clarify_id) {
                notify_clarify_expired(client, token, query_id, &chat_id, message_id, &original_text)
                    .await;
                return;
            }
            answer_callback(client, token, query_id, "✏️ Type your answer in the chat.").await;
            let edited = format!(
                "❓ {}\n\n<i>Awaiting typed response from {}…</i>",
                html_escape(&original_text),
                html_escape(&user_display)
            );
            edit_clarify_message(client, token, &chat_id, message_id, &edited).await;
            return;
        }

        let Ok(idx) = choice_token.parse::<usize>() else {
            answer_callback(client, token, query_id, "Invalid choice.").await;
            return;
        };
        // Choice text from the registered entry; fall back to the index if
        // the entry was cleaned up (hermes race fallback).
        let resolved_text = crate::clarify_gateway::peek_choice(clarify_id, choice_token)
            .unwrap_or_else(|| format!("choice {}", idx + 1));
        if crate::clarify_gateway::resolve(clarify_id, &resolved_text) {
            let preview: String = resolved_text.chars().take(60).collect();
            answer_callback(client, token, query_id, &format!("✓ {preview}")).await;
            let edited = format!(
                "❓ {}\n\n<b>{}:</b> {}",
                html_escape(&original_text),
                html_escape(&user_display),
                html_escape(&resolved_text)
            );
            edit_clarify_message(client, token, &chat_id, message_id, &edited).await;
        } else {
            // Entry evicted (clarify timeout / gateway restart) between ask
            // and tap — surface it instead of a misleading ✓.
            notify_clarify_expired(client, token, query_id, &chat_id, message_id, &original_text)
                .await;
        }
    }

    /// answerCallbackQuery wrapper — always fire-and-forget; a failed
    /// answer only means the spinner lingers (hermes ignores errors too).
    async fn answer_callback(client: &reqwest::Client, token: &str, query_id: &str, text: &str) {
        let params = json!({"callback_query_id": query_id, "text": text});
        if let Err(e) = api(client, token, "answerCallbackQuery", params).await {
            eprintln!("[telegram] answerCallbackQuery failed: {e}");
        }
    }

    /// Edit the clarify prompt after a tap (hermes edit_message_text with
    /// reply_markup=None — the explicit null drops the inline keyboard).
    async fn edit_clarify_message(
        client: &reqwest::Client,
        token: &str,
        chat_id: &str,
        message_id: i64,
        text: &str,
    ) {
        let params = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": null,
        });
        if let Err(e) = api(client, token, "editMessageText", params).await {
            eprintln!("[telegram] editMessageText failed: {e}");
        }
    }

    /// Tell the user a clarify tap arrived too late to be delivered
    /// (hermes `_notify_clarify_expired`).
    async fn notify_clarify_expired(
        client: &reqwest::Client,
        token: &str,
        query_id: &str,
        chat_id: &str,
        message_id: i64,
        original_text: &str,
    ) {
        answer_callback(client, token, query_id, "⚠️ This prompt expired — please /retry.").await;
        let edited = format!(
            "❓ {}\n\n<i>⚠️ This question expired or the session reset — please /retry.</i>",
            html_escape(original_text)
        );
        edit_clarify_message(client, token, chat_id, message_id, &edited).await;
    }

    /// Send a reply, splitting at 4096-char Telegram limit (hermes chunks
    /// long replies the same way).
    /// Download the message's first media payload (photo > document >
    /// video > audio > voice, hermes precedence) via getFile and cache it.
    async fn download_media(
        client: &reqwest::Client,
        token: &str,
        message: &Value,
    ) -> Vec<MediaAttachment> {
        let (file_id, mime, name): (String, &str, String) =
            if let Some(photo) = message.get("photo").and_then(|v| v.as_array()) {
                // Bot API sends photos as a size ladder; last = largest.
                match photo.last().and_then(|p| p.get("file_id").and_then(|v| v.as_str())) {
                    Some(file_id) => (file_id.to_string(), "image/jpeg", String::new()),
                    None => return Vec::new(),
                }
            } else if let Some(media) = message
                .get("document")
                .or_else(|| message.get("video"))
                .or_else(|| message.get("audio"))
                .or_else(|| message.get("voice"))
            {
                let Some(file_id) = media.get("file_id").and_then(|v| v.as_str()) else {
                    return Vec::new();
                };
                let mime = if message.get("voice").is_some() {
                    "audio/ogg"
                } else {
                    media.get("mime_type").and_then(|v| v.as_str()).unwrap_or("")
                };
                let name = media
                    .get("file_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (file_id.to_string(), mime, name)
            } else {
                return Vec::new();
            };
        let info = match api(client, token, "getFile", json!({"file_id": file_id})).await {
            Ok(info) => info,
            Err(e) => {
                eprintln!("[telegram] getFile failed: {e}");
                return Vec::new();
            }
        };
        let Some(file_path) = info.pointer("/result/file_path").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let url = format!("{}/file/bot{token}/{file_path}", telegram_api_base());
        match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
            Ok(response) => match response.bytes().await {
                Ok(bytes) => cache_attachment(bytes.to_vec(), mime, &name)
                    .into_iter()
                    .collect(),
                Err(e) => {
                    eprintln!("[telegram] media download failed: {e}");
                    Vec::new()
                }
            },
            Err(e) => {
                eprintln!("[telegram] media download failed: {e}");
                Vec::new()
            }
        }
    }

    /// Send a photo (MEDIA: delivery, hermes extract_media path).
    pub async fn send_photo(client: &reqwest::Client, token: &str, chat_id: &str, path: &std::path::Path) {
        let Ok(data) = tokio::fs::read(path).await else {
            eprintln!("[telegram] cannot read media {}", path.display());
            return;
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "media".to_string());
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", reqwest::multipart::Part::bytes(data).file_name(file_name));
        let url = format!("{API}/bot{token}/sendPhoto");
        if let Err(e) = client.post(&url).multipart(form).send().await {
            eprintln!("[telegram] sendPhoto failed: {e}");
        }
    }

    /// Send a non-image file (MEDIA: delivery).
    pub async fn send_document(client: &reqwest::Client, token: &str, chat_id: &str, path: &std::path::Path) {
        let Ok(data) = tokio::fs::read(path).await else {
            eprintln!("[telegram] cannot read media {}", path.display());
            return;
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "media".to_string());
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", reqwest::multipart::Part::bytes(data).file_name(file_name));
        let url = format!("{API}/bot{token}/sendDocument");
        if let Err(e) = client.post(&url).multipart(form).send().await {
            eprintln!("[telegram] sendDocument failed: {e}");
        }
    }

    pub async fn send_message(client: &reqwest::Client, token: &str, chat_id: &str, text: &str) {
        for chunk in chunk_text(text, 4000) {
            let params = json!({"chat_id": chat_id, "text": chunk});
            if let Err(e) = api(client, token, "sendMessage", params).await {
                eprintln!("[telegram] sendMessage failed: {e}");
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// axum mock of the Bot API — logs (method, body) per call.
        async fn spawn_telegram_api(
            log: Arc<std::sync::Mutex<Vec<(String, Value)>>>,
            response_ok: bool,
        ) -> String {
            use axum::extract::State;
            use axum::routing::post;
            let app = axum::Router::new()
                .route(
                    "/botTEST/:method",
                    post(
                        move |State(log): State<Arc<std::sync::Mutex<Vec<(String, Value)>>>>,
                         axum::extract::Path(method): axum::extract::Path<String>,
                         axum::Json(body): axum::Json<Value>| async move {
                            log.lock().unwrap().push((method, body));
                            axum::Json(json!({ "ok": response_ok, "result": {} }))
                        },
                    ),
                )
                .with_state(log);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            format!("http://{addr}")
        }

        fn clarify_query(clarify_id: &str, token: &str) -> Value {
            json!({
                "id": "cb-1",
                "from": {"id": 7, "first_name": "Ann"},
                "data": format!("cl:{clarify_id}:{token}"),
                "message": {
                    "message_id": 99,
                    "text": "❓ Pick\n\n1. Alpha\n2. Beta",
                    "chat": {"id": 42},
                },
            })
        }

        fn authorized_cfg() -> TelegramConfig {
            TelegramConfig {
                allowed_chat_ids: vec!["42".into()],
                ..Default::default()
            }
        }

        #[tokio::test]
        async fn telegram_send_clarify_renders_inline_keyboard() {
            let _env_guard = crate::models_dev::test_env_lock();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
            let base = spawn_telegram_api(log.clone(), true).await;
            std::env::set_var("TELEGRAM_API_BASE", &base);
            let ok = send_clarify_message(
                &reqwest::Client::new(),
                "TEST",
                "42",
                "abc123def456",
                "Pick <one>",
                &["Alpha & A".into(), "Beta".into()],
            )
            .await;
            std::env::remove_var("TELEGRAM_API_BASE");
            assert!(ok);
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            let (method, body) = &reqs[0];
            assert_eq!(method, "sendMessage");
            let text = body["text"].as_str().unwrap();
            // HTML parse mode with escaped question + numbered options in
            // the body (hermes mobile readability layout).
            assert_eq!(body["parse_mode"], "HTML");
            assert!(text.starts_with("❓ Pick &lt;one&gt;"));
            assert!(text.contains("1. Alpha &amp; A"));
            assert!(text.contains("2. Beta"));
            // One row per choice (short numeric labels) + Other row, with
            // cl:<id>:<idx|other> callback payloads.
            let rows = body["reply_markup"]["inline_keyboard"].as_array().unwrap();
            assert_eq!(rows.len(), 3);
            assert_eq!(rows[0][0]["text"], "1");
            assert_eq!(rows[0][0]["callback_data"], "cl:abc123def456:0");
            assert_eq!(rows[1][0]["text"], "2");
            assert_eq!(rows[1][0]["callback_data"], "cl:abc123def456:1");
            assert_eq!(rows[2][0]["text"], "✏️ Other (type answer)");
            assert_eq!(rows[2][0]["callback_data"], "cl:abc123def456:other");
        }

        #[tokio::test]
        async fn telegram_send_clarify_api_failure_returns_false() {
            let _env_guard = crate::models_dev::test_env_lock();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
            let base = spawn_telegram_api(log.clone(), false).await;
            std::env::set_var("TELEGRAM_API_BASE", &base);
            let ok = send_clarify_message(
                &reqwest::Client::new(),
                "TEST",
                "42",
                "abc123def456",
                "Pick",
                &["Alpha".into()],
            )
            .await;
            std::env::remove_var("TELEGRAM_API_BASE");
            // False → messaging_clarify_fn sends the numbered-text fallback.
            assert!(!ok);
        }

        #[tokio::test]
        async fn telegram_callback_numeric_choice_resolves_clarify() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
            let base = spawn_telegram_api(log.clone(), true).await;
            std::env::set_var("TELEGRAM_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-telegram-42",
                "Pick",
                &["Alpha".into(), "Beta".into()],
                false,
            );
            let clarify_id = handle.clarify_id.clone();
            let query = clarify_query(&clarify_id, "1");
            handle_callback_query(&reqwest::Client::new(), "TEST", &authorized_cfg(), None, &query)
                .await;
            std::env::remove_var("TELEGRAM_API_BASE");
            // The tool waiter received the tapped choice text.
            assert_eq!(handle.rx.await.unwrap(), "Beta");
            let reqs = log.lock().unwrap();
            let methods: Vec<&str> = reqs.iter().map(|(m, _)| m.as_str()).collect();
            assert_eq!(methods, vec!["answerCallbackQuery", "editMessageText"]);
            assert_eq!(reqs[0].1["text"], "✓ Beta");
            let edit = &reqs[1].1;
            assert!(edit["text"].as_str().unwrap().contains("<b>Ann:</b> Beta"));
            // Explicit null drops the inline keyboard.
            assert_eq!(edit["reply_markup"], Value::Null);
        }

        #[tokio::test]
        async fn telegram_callback_other_flips_to_text_capture() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
            let base = spawn_telegram_api(log.clone(), true).await;
            std::env::set_var("TELEGRAM_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-telegram-42",
                "Pick",
                &["Alpha".into(), "Beta".into()],
                false,
            );
            let clarify_id = handle.clarify_id.clone();
            let query = clarify_query(&clarify_id, "other");
            handle_callback_query(&reqwest::Client::new(), "TEST", &authorized_cfg(), None, &query)
                .await;
            std::env::remove_var("TELEGRAM_API_BASE");
            // Entry survives in text-capture mode; the next message in the
            // session resolves it (text intercept).
            let pending = crate::clarify_gateway::pending_for_session("platform-telegram-42")
                .expect("entry must survive the Other tap");
            assert!(pending.awaiting_text);
            assert!(crate::clarify_gateway::resolve(&clarify_id, "typed answer"));
            assert_eq!(handle.rx.await.unwrap(), "typed answer");
            let reqs = log.lock().unwrap();
            let methods: Vec<&str> = reqs.iter().map(|(m, _)| m.as_str()).collect();
            assert_eq!(methods, vec!["answerCallbackQuery", "editMessageText"]);
            assert_eq!(reqs[0].1["text"], "✏️ Type your answer in the chat.");
            assert!(reqs[1].1["text"]
                .as_str()
                .unwrap()
                .contains("Awaiting typed response from Ann…"));
        }

        #[tokio::test]
        async fn telegram_callback_stale_entry_answers_resolved() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
            let base = spawn_telegram_api(log.clone(), true).await;
            std::env::set_var("TELEGRAM_API_BASE", &base);
            // No entry registered — a tap on a resolved/unknown prompt.
            let query = clarify_query("zzzzzzzzzzzz", "0");
            handle_callback_query(&reqwest::Client::new(), "TEST", &authorized_cfg(), None, &query)
                .await;
            std::env::remove_var("TELEGRAM_API_BASE");
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            assert_eq!(reqs[0].0, "answerCallbackQuery");
            assert_eq!(reqs[0].1["text"], "This prompt has already been resolved.");
        }

        #[tokio::test]
        async fn telegram_callback_unauthorized_rejected() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
            let base = spawn_telegram_api(log.clone(), true).await;
            std::env::set_var("TELEGRAM_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-telegram-42",
                "Pick",
                &["Alpha".into()],
                false,
            );
            let clarify_id = handle.clarify_id.clone();
            let query = clarify_query(&clarify_id, "0");
            // Chat 42 is not allowlisted and no pairing store exists.
            let cfg = TelegramConfig {
                allowed_chat_ids: vec!["999".into()],
                ..Default::default()
            };
            handle_callback_query(&reqwest::Client::new(), "TEST", &cfg, None, &query).await;
            std::env::remove_var("TELEGRAM_API_BASE");
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            assert_eq!(
                reqs[0].1["text"],
                "⛔ You are not authorized to answer this prompt."
            );
            // The waiter is untouched — the legitimate user can still answer.
            assert!(crate::clarify_gateway::contains(&clarify_id));
            assert!(crate::clarify_gateway::resolve(&clarify_id, "Alpha"));
        }

        #[tokio::test]
        async fn telegram_callback_expired_when_waiter_gone() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
            let base = spawn_telegram_api(log.clone(), true).await;
            std::env::set_var("TELEGRAM_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-telegram-42",
                "Pick",
                &["Alpha".into(), "Beta".into()],
                false,
            );
            let clarify_id = handle.clarify_id.clone();
            // The clarify tool gave up (receiver dropped) before the tap.
            drop(handle.rx);
            let query = clarify_query(&clarify_id, "1");
            handle_callback_query(&reqwest::Client::new(), "TEST", &authorized_cfg(), None, &query)
                .await;
            std::env::remove_var("TELEGRAM_API_BASE");
            let reqs = log.lock().unwrap();
            let methods: Vec<&str> = reqs.iter().map(|(m, _)| m.as_str()).collect();
            assert_eq!(methods, vec!["answerCallbackQuery", "editMessageText"]);
            assert_eq!(reqs[0].1["text"], "⚠️ This prompt expired — please /retry.");
            assert!(reqs[1].1["text"]
                .as_str()
                .unwrap()
                .contains("This question expired or the session reset"));
        }
    }
}

// ---------------------------------------------------------------------------
// Discord — Gateway v10 websocket + REST
// ---------------------------------------------------------------------------

pub mod discord {
    use super::*;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    const GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
    const REST: &str = "https://discord.com/api/v10";
    const INTENT_GUILD_MESSAGES: u64 = 1 << 9;
    const INTENT_DIRECT_MESSAGES: u64 = 1 << 12;
    const INTENT_MESSAGE_CONTENT: u64 = 1 << 15;

    /// Discord REST base URL — normally `https://discord.com/api/v10`;
    /// the `DISCORD_API_BASE` override exists for tests and corporate
    /// proxies (mirrors the slack/telegram pattern).
    pub(crate) fn discord_api_base() -> String {
        std::env::var("DISCORD_API_BASE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| REST.to_string())
    }

    struct Sender {
        token: String,
        prompt_timeout_secs: u64,
    }

    #[async_trait::async_trait]
    impl PlatformSender for Sender {
        async fn send_text(&self, chat_id: &str, text: &str) {
            send_channel_message(&self.token, chat_id, text).await;
        }

        /// Embed + button clarify prompt (hermes Discord `send_clarify`).
        /// Returns false on API failure so the numbered-text fallback still
        /// goes out.
        async fn send_clarify(
            &self,
            chat_id: &str,
            clarify_id: &str,
            question: &str,
            choices: &[String],
        ) -> bool {
            send_clarify_message(
                &self.token,
                chat_id,
                clarify_id,
                question,
                choices,
                self.prompt_timeout_secs,
            )
            .await
        }
    }

    pub async fn run(cfg: DiscordConfig, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<crate::pairing::PairingStore>>) {
        let Some(token) = resolve_token(&cfg.bot_token, "DISCORD_BOT_TOKEN") else {
            eprintln!("[discord] disabled: no bot_token configured (set messaging.discord.bot_token or DISCORD_BOT_TOKEN)");
            return;
        };
        register_platform_sender(
            "discord",
            Arc::new(Sender {
                token: token.clone(),
                prompt_timeout_secs: cfg.prompt_timeout_secs,
            }),
        );
        loop {
            if let Err(e) = run_session(&cfg, &token, dispatcher.clone(), pairing.clone()).await {
                eprintln!("[discord] gateway session ended: {e} — reconnecting in 5s");
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn run_session(cfg: &DiscordConfig, token: &str, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<crate::pairing::PairingStore>>) -> Result<()> {
        use futures::{SinkExt, StreamExt};
        let (ws, _) = tokio_tungstenite::connect_async(GATEWAY)
            .await
            .map_err(|e| AgentError::Tool(format!("discord connect: {e}")))?;
        eprintln!("[discord] gateway connected");
        let (mut sink, mut stream) = ws.split();
        let mut heartbeat_interval = std::time::Duration::from_secs(41);
        let mut last_sequence: Option<u64> = None;
        let mut identified = false;
        let mut heartbeat_due = tokio::time::Instant::now() + heartbeat_interval;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(heartbeat_due) => {
                    let payload = json!({"op": 1, "d": last_sequence});
                    sink.send(WsMessage::Text(payload.to_string())).await.ok();
                    heartbeat_due = tokio::time::Instant::now() + heartbeat_interval;
                }
                message = stream.next() => {
                    let Some(Ok(message)) = message else {
                        return Err(AgentError::Tool("discord gateway closed".into()));
                    };
                    let WsMessage::Text(text) = message else { continue };
                    let Ok(payload) = serde_json::from_str::<Value>(&text) else { continue };
                    let op = payload.get("op").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
                    if let Some(seq) = payload.get("s").and_then(|v| v.as_u64()) {
                        last_sequence = Some(seq);
                    }
                    match op {
                        10 => {
                            heartbeat_interval = std::time::Duration::from_millis(
                                payload.pointer("/d/heartbeat_interval").and_then(|v| v.as_u64()).unwrap_or(41250),
                            );
                            heartbeat_due = tokio::time::Instant::now() + heartbeat_interval;
                            if !identified {
                                let identify = json!({
                                    "op": 2,
                                    "d": {
                                        "token": token,
                                        "intents": INTENT_GUILD_MESSAGES | INTENT_DIRECT_MESSAGES | INTENT_MESSAGE_CONTENT,
                                        "properties": {"os": "linux", "browser": "ulnclaw", "device": "ulnclaw"},
                                    }
                                });
                                sink.send(WsMessage::Text(identify.to_string())).await.ok();
                                identified = true;
                            }
                        }
                        11 => {} // heartbeat ACK
                        1 => {
                            let payload = json!({"op": 1, "d": last_sequence});
                            sink.send(WsMessage::Text(payload.to_string())).await.ok();
                        }
                        0 => {
                            let event_name = payload.get("t").and_then(|v| v.as_str()).unwrap_or("");
                            if event_name == "READY" {
                                let username = payload.pointer("/d/user/username").and_then(|v| v.as_str()).unwrap_or("?");
                                eprintln!("[discord] logged in as {username}");
                            }
                            if event_name == "MESSAGE_CREATE" {
                                handle_message_create(cfg, token, &dispatcher, payload.get("d").cloned().unwrap_or(json!({})), pairing.as_deref()).await;
                            }
                            // Clarify button taps arrive as MESSAGE_COMPONENT
                            // interactions (hermes ClarifyChoiceView routing).
                            if event_name == "INTERACTION_CREATE" {
                                handle_interaction_create(cfg, token, payload.get("d").cloned().unwrap_or(json!({})), pairing.as_deref()).await;
                            }
                        }
                        7 | 9 => {
                            return Err(AgentError::Tool("discord requested reconnect / invalid session".into()));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn handle_message_create(cfg: &DiscordConfig, token: &str, dispatcher: &Arc<Dispatcher>, data: Value, pairing: Option<&crate::pairing::PairingStore>) {
        // Ignore bot/webhook messages (never talk to ourselves).
        if data.get("author").and_then(|a| a.get("bot")).and_then(|v| v.as_bool()).unwrap_or(false) {
            return;
        }
        let text = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        // Attachment downloads run before any gate so media-only messages
        // still flow (hermes). Discord exposes url/filename/content_type.
        let attachments = {
            let client = reqwest::Client::new();
            let mut downloaded = Vec::new();
            if let Some(items) = data.get("attachments").and_then(|v| v.as_array()) {
                for item in items {
                    let Some(url) = item.get("url").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let name = item.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                    let mime = item.get("content_type").and_then(|v| v.as_str()).unwrap_or("");
                    let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                    if let Some(attachment) =
                        download_to_cache(&client, url, None, mime, name, size).await
                    {
                        downloaded.push(attachment);
                    }
                }
            }
            downloaded
        };
        if text.is_empty() && attachments.is_empty() {
            return;
        }
        let channel_id = data.get("channel_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if channel_id.is_empty() {
            return;
        }
        let author = data.get("author").cloned().unwrap_or(json!({}));
        let mut event = MessageEvent {
            platform: "discord".into(),
            chat_id: channel_id.clone(),
            sender_id: author.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            sender_name: author
                .get("global_name")
                .or_else(|| author.get("username"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            text: text.to_string(),
            message_id: data.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            attachments,
        };
        // Plugin gate before auth (hermes ordering).
        if !pre_gateway_dispatch_gate(&mut event).await {
            return;
        }
        // Auth union: configured allowlist OR an approved pairing code.
        let authorized = allowlisted(&cfg.allowed_channel_ids, &channel_id)
            || pairing
                .map(|store| store.is_approved("discord", &event.sender_id))
                .unwrap_or(false);
        if !authorized {
            eprintln!(
                "[discord] refusing message from channel {channel_id} — add it to \
                 messaging.discord.allowed_channel_ids or approve a pairing code"
            );
            if let Some(store) = pairing {
                if let Some(reply) = pairing_offer(store, "discord", &event.sender_id, &event.sender_name) {
                    send_channel_message(token, &channel_id, &reply).await;
                }
            }
            return;
        }
        let dispatcher = dispatcher.clone();
        let token = token.to_string();
        tokio::spawn(async move {
            let outcome = match dispatcher.handle_event(event.clone()).await {
                Ok(outcome) => outcome,
                Err(e) => crate::messaging::DispatchOutcome {
                    reply: format!("error: {e}"),
                    transcript_echoes: Vec::new(),
                },
            };
            for echo in &outcome.transcript_echoes {
                send_channel_message(&token, &event.chat_id, echo).await;
            }
            let reply = outcome.reply;
            let (reply_text, media_paths) = extract_media_tags(&reply);
            if !reply_text.trim().is_empty() {
                send_channel_message(&token, &event.chat_id, &reply_text).await;
            }
            for path in &media_paths {
                send_attachment(&token, &event.chat_id, path).await;
            }
        });
    }

    /// Upload a file as a native Discord attachment (MEDIA: delivery).
    async fn send_attachment(token: &str, channel_id: &str, path: &std::path::Path) {
        let Ok(data) = tokio::fs::read(path).await else {
            eprintln!("[discord] cannot read media {}", path.display());
            return;
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "media".to_string());
        let form = reqwest::multipart::Form::new()
            .part("files[0]", reqwest::multipart::Part::bytes(data).file_name(file_name));
        let url = format!("{}/channels/{channel_id}/messages", discord_api_base());
        let response = reqwest::Client::new()
            .post(&url)
            .header("Authorization", format!("Bot {token}"))
            .multipart(form)
            .send()
            .await;
        if let Err(e) = response.and_then(|r| r.error_for_status()) {
            eprintln!("[discord] attachment upload failed: {e}");
        }
    }

    /// POST /channels/{id}/messages with 2000-char Discord chunking.
    pub async fn send_channel_message(token: &str, channel_id: &str, text: &str) {
        let client = reqwest::Client::new();
        for chunk in chunk_text(text, 1900) {
            let result = client
                .post(format!("{}/channels/{channel_id}/messages", discord_api_base()))
                .header("Authorization", format!("Bot {token}"))
                .json(&json!({"content": chunk}))
                .send()
                .await;
            if let Err(e) = result {
                eprintln!("[discord] send failed: {e}");
            }
        }
    }

    // -- Clarify buttons — hermes ClarifyChoiceView parity -----------------

    const BUTTON_LABEL_LIMIT: usize = 80; // Discord button-label UTF-16 cap
    const MAX_CHOICES: usize = 24; // 25 buttons per message − the Other slot

    /// hermes `approvals.discord_prompt_timeout` semantics: default 300 s,
    /// clamped to 30–900 s so prompts never outlive Discord's 15-minute
    /// interaction-token expiry.
    fn prompt_timeout(cfg_timeout_secs: u64) -> std::time::Duration {
        let secs = if cfg_timeout_secs == 0 {
            300
        } else {
            cfg_timeout_secs.clamp(30, 900)
        };
        std::time::Duration::from_secs(secs)
    }

    fn utf16_len(text: &str) -> usize {
        text.chars().map(|c| c.len_utf16()).sum()
    }

    fn prefix_within_utf16_limit(text: &str, limit: usize) -> String {
        let mut out = String::new();
        let mut used = 0;
        for ch in text.chars() {
            let width = ch.len_utf16();
            if used + width > limit {
                break;
            }
            out.push(ch);
            used += width;
        }
        out
    }

    fn truncate_chars(text: &str, max_chars: usize) -> String {
        text.chars().take(max_chars).collect()
    }

    /// Discord button labels cap at 80 UTF-16 units and mobile width is
    /// much narrower, so hermes cuts long labels at a word boundary when
    /// possible: last space in the trailing half, then the latest soft
    /// punctuation, then a hard cut — always ending with an ellipsis.
    fn clarify_button_label(idx: usize, choice: &str) -> String {
        let prefix = format!("{}. ", idx + 1);
        let budget = BUTTON_LABEL_LIMIT.saturating_sub(utf16_len(&prefix));
        if utf16_len(choice) <= budget {
            return format!("{prefix}{choice}");
        }
        let truncated: Vec<char> = prefix_within_utf16_limit(choice, budget.saturating_sub(1))
            .trim_end()
            .chars()
            .collect();
        let len = truncated.len();
        let half = len / 2;
        let mut cut = len; // hard cut (last resort)
        if let Some(space) = truncated.iter().rposition(|c| *c == ' ') {
            if space >= half {
                cut = space;
            }
        }
        if cut == len {
            if let Some(pos) = truncated.iter().rposition(|c| matches!(c, '-' | ',' | '.' | ')')) {
                if pos >= half {
                    cut = pos + 1; // inclusive: label ends on the soft char
                }
            }
        }
        let mut body: String = truncated[..cut].iter().collect();
        body = body.trim_end().to_string();
        body.push('…');
        format!("{prefix}{body}")
    }

    /// One action row per 5 buttons (Discord limits), one button per
    /// choice plus the ✏️ Other row — `clarify:<id>:<idx|other>` custom
    /// ids (hermes ClarifyChoiceView).
    fn clarify_components(clarify_id: &str, choices: &[String], disabled: bool) -> Vec<Value> {
        let mut buttons: Vec<Value> = choices
            .iter()
            .enumerate()
            .map(|(idx, choice)| {
                json!({
                    "type": 2,
                    "style": 1,
                    "label": clarify_button_label(idx, choice),
                    "custom_id": format!("clarify:{clarify_id}:{idx}"),
                    "disabled": disabled,
                })
            })
            .collect();
        buttons.push(json!({
            "type": 2,
            "style": 2,
            "label": "✏️ Other (type answer)",
            "custom_id": format!("clarify:{clarify_id}:other"),
            "disabled": disabled,
        }));
        buttons
            .chunks(5)
            .map(|row| json!({"type": 1, "components": row}))
            .collect()
    }

    fn clarify_embed(question: &str, with_choices: bool) -> Value {
        let mut description = question.trim().to_string();
        if description.chars().count() > 4088 {
            description = format!("{}...", truncate_chars(&description, 4085));
        }
        let field = if with_choices {
            json!({"name": "Choices", "value": "Pick one below, or click ✏️ Other to type a custom answer.", "inline": false})
        } else {
            json!({"name": "Reply", "value": "Reply in this channel with your answer.", "inline": false})
        };
        json!({
            "title": "❓ ulnclaw needs your input",
            "description": description,
            "color": 0xE67E22, // discord.Color.orange()
            "fields": [field],
        })
    }

    /// Plain-content mirror of the embed — embeds are invisible on some
    /// clients (hermes `_self_contained_prompt_content`).
    fn self_contained_prompt_content(header: &str, body: &str, tail: &str) -> String {
        let prefix = format!("{header}\n\n");
        let truncated_suffix = "\n... [truncated]";
        let budget = 2000usize.saturating_sub(prefix.chars().count() + tail.chars().count());
        let mut body = body.to_string();
        if body.chars().count() > budget {
            let cut = budget.saturating_sub(truncated_suffix.chars().count());
            body = format!("{}{truncated_suffix}", truncate_chars(&body, cut));
        }
        format!("{prefix}{body}{tail}")
    }

    /// Send a clarify prompt as embed + one button per choice (hermes
    /// Discord `send_clarify`). Returns false on failure (or with no
    /// choices — open-ended prompts ride the numbered-text path) so the
    /// caller falls back to numbered text.
    pub async fn send_clarify_message(
        token: &str,
        channel_id: &str,
        clarify_id: &str,
        question: &str,
        choices: &[String],
        prompt_timeout_secs: u64,
    ) -> bool {
        if choices.is_empty() {
            return false;
        }
        let choices = &choices[..choices.len().min(MAX_CHOICES)];
        let content = self_contained_prompt_content(
            "❓ **ulnclaw needs your input**",
            question.trim(),
            "\n\nPick one below, or click ✏️ Other to type a custom answer.",
        );
        let body = json!({
            "content": content,
            "embeds": [clarify_embed(question, true)],
            "components": clarify_components(clarify_id, choices, false),
        });
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/channels/{channel_id}/messages", discord_api_base()))
            .header("Authorization", format!("Bot {token}"))
            .json(&body)
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => {
                let sent: Value = resp.json().await.unwrap_or(json!({}));
                if let Some(message_id) = sent.get("id").and_then(|v| v.as_str()) {
                    spawn_prompt_expiry(
                        token,
                        channel_id,
                        message_id,
                        clarify_id,
                        question,
                        choices.to_vec(),
                        prompt_timeout(prompt_timeout_secs),
                    );
                }
                true
            }
            Ok(resp) => {
                eprintln!("[discord] send_clarify failed: HTTP {}", resp.status());
                false
            }
            Err(e) => {
                eprintln!("[discord] send_clarify failed: {e}");
                false
            }
        }
    }

    /// Visual expiration after the prompt window (hermes ClarifyChoiceView
    /// `on_timeout`): grey the embed, disable the buttons. ulnclaw skips
    /// prompts already resolved or still capturing a typed answer — hermes
    /// overwrites the "Awaiting typed response" footer regardless, which
    /// would hide an active text capture (documented divergence).
    fn spawn_prompt_expiry(
        token: &str,
        channel_id: &str,
        message_id: &str,
        clarify_id: &str,
        question: &str,
        choices: Vec<String>,
        timeout: std::time::Duration,
    ) {
        let token = token.to_string();
        let channel_id = channel_id.to_string();
        let message_id = message_id.to_string();
        let clarify_id = clarify_id.to_string();
        let question = question.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if !crate::clarify_gateway::contains(&clarify_id)
                || crate::clarify_gateway::is_awaiting_text(&clarify_id)
            {
                return;
            }
            let mut embed = clarify_embed(&question, true);
            embed["color"] = json!(0x99AAB5); // discord.Color.greyple()
            embed["footer"] = json!({"text": "⏱ Prompt expired — no action taken"});
            let body = json!({
                "embeds": [embed],
                "components": clarify_components(&clarify_id, &choices, true),
            });
            let client = reqwest::Client::new();
            let url = format!(
                "{}/channels/{channel_id}/messages/{message_id}",
                discord_api_base()
            );
            let result = client
                .patch(&url)
                .header("Authorization", format!("Bot {token}"))
                .json(&body)
                .send()
                .await;
            if let Err(e) = result.and_then(|r| r.error_for_status()) {
                eprintln!("[discord] prompt expiry edit failed: {e}");
            }
        });
    }

    /// Route INTERACTION_CREATE button taps (hermes ClarifyChoiceView
    /// callbacks). Only `clarify:<id>:<idx|other>` custom_ids are handled;
    /// every handled path answers via the interaction callback endpoint.
    async fn handle_interaction_create(
        cfg: &DiscordConfig,
        token: &str,
        data: Value,
        pairing: Option<&crate::pairing::PairingStore>,
    ) {
        let interaction_type = data.get("type").and_then(|v| v.as_u64()).unwrap_or(0);
        if interaction_type != 3 {
            return; // MESSAGE_COMPONENT only
        }
        let custom_id = data
            .pointer("/data/custom_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !custom_id.starts_with("clarify:") {
            return;
        }
        let parts: Vec<&str> = custom_id.splitn(3, ':').collect();
        if parts.len() != 3 {
            return;
        }
        let clarify_id = parts[1];
        let choice_token = parts[2];
        let interaction_id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let interaction_token = data.get("token").and_then(|v| v.as_str()).unwrap_or("");
        if interaction_id.is_empty() || interaction_token.is_empty() {
            return;
        }
        let user = data
            .get("user")
            .or_else(|| data.pointer("/member/user"))
            .cloned()
            .unwrap_or(json!({}));
        let user_id = user.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let display_name = user
            .get("global_name")
            .or_else(|| user.get("username"))
            .and_then(|v| v.as_str())
            .unwrap_or("user")
            .to_string();
        let channel_id = data
            .get("channel_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let client = reqwest::Client::new();

        // Same union as MESSAGE_CREATE: channel allowlist OR an approved
        // pairing (ulnclaw's analogue of hermes `_component_check_auth`).
        let authorized = allowlisted(&cfg.allowed_channel_ids, &channel_id)
            || pairing
                .map(|store| store.is_approved("discord", &user_id))
                .unwrap_or(false);

        // Resolved prompts first (hermes `self.resolved` check).
        if !crate::clarify_gateway::contains(clarify_id) {
            ephemeral_notice(
                &client,
                token,
                interaction_id,
                interaction_token,
                "This prompt has already been answered~",
            )
            .await;
            return;
        }
        if !authorized {
            ephemeral_notice(
                &client,
                token,
                interaction_id,
                interaction_token,
                "You're not authorized to answer this prompt~",
            )
            .await;
            return;
        }

        let message = data.get("message").cloned().unwrap_or(json!({}));
        if choice_token == "other" {
            if crate::clarify_gateway::mark_awaiting_text(clarify_id) {
                let embed = restyle_embed(
                    &message,
                    0x3498DB, // discord.Color.blue()
                    &format!("Awaiting typed response from {display_name}…"),
                );
                update_message_response(
                    &client,
                    token,
                    interaction_id,
                    interaction_token,
                    &message,
                    embed,
                )
                .await;
            } else {
                ephemeral_notice(
                    &client,
                    token,
                    interaction_id,
                    interaction_token,
                    "⚠️ This prompt expired — please retry.",
                )
                .await;
            }
            return;
        }

        let Ok(idx) = choice_token.parse::<usize>() else {
            ephemeral_notice(&client, token, interaction_id, interaction_token, "Invalid choice.")
                .await;
            return;
        };
        // Canonical choice text from the registry (hermes `_entries`
        // lookup); fall back to the index if the entry was cleaned up.
        let resolved_text = crate::clarify_gateway::peek_choice(clarify_id, choice_token)
            .unwrap_or_else(|| format!("choice {}", idx + 1));
        if crate::clarify_gateway::resolve(clarify_id, &resolved_text) {
            let embed = restyle_embed(
                &message,
                0x2ECC71, // discord.Color.green()
                &format!("Answered by {display_name}: {resolved_text}"),
            );
            update_message_response(
                &client,
                token,
                interaction_id,
                interaction_token,
                &message,
                embed,
            )
            .await;
        } else {
            // Entry evicted between ask and click — say so instead of
            // leaving a live-looking prompt (hermes logs only).
            ephemeral_notice(
                &client,
                token,
                interaction_id,
                interaction_token,
                "⚠️ This prompt expired — please retry.",
            )
            .await;
        }
    }

    /// Restyle the prompt's embed with the resolution footer (hermes edits
    /// `interaction.message.embeds[0]` color + footer).
    fn restyle_embed(message: &Value, color: u32, footer_text: &str) -> Value {
        let mut embed = message
            .get("embeds")
            .and_then(|v| v.as_array())
            .and_then(|rows| rows.first())
            .cloned()
            .unwrap_or(json!({}));
        embed["color"] = json!(color);
        embed["footer"] = json!({"text": footer_text});
        embed
    }

    /// Clone the message's component rows with every button disabled
    /// (hermes `child.disabled = True` before the edit).
    fn disable_components(components: Option<&Value>) -> Option<Value> {
        let rows = components?.as_array()?;
        let disabled_rows: Vec<Value> = rows
            .iter()
            .map(|row| {
                let mut row = row.clone();
                if let Some(items) = row.get_mut("components").and_then(|v| v.as_array_mut()) {
                    for item in items {
                        if let Some(object) = item.as_object_mut() {
                            object.insert("disabled".to_string(), Value::Bool(true));
                        }
                    }
                }
                row
            })
            .collect();
        Some(Value::Array(disabled_rows))
    }

    /// POST /interactions/{id}/{token}/callback.
    async fn interaction_callback(
        client: &reqwest::Client,
        token: &str,
        interaction_id: &str,
        interaction_token: &str,
        body: Value,
    ) {
        let url = format!(
            "{}/interactions/{interaction_id}/{interaction_token}/callback",
            discord_api_base()
        );
        let result = client
            .post(&url)
            .header("Authorization", format!("Bot {token}"))
            .json(&body)
            .send()
            .await;
        if let Err(e) = result.and_then(|r| r.error_for_status()) {
            eprintln!("[discord] interaction callback failed: {e}");
        }
    }

    /// Ephemeral notice — callback type 4 + EPHEMERAL flag (hermes
    /// `interaction.response.send_message(..., ephemeral=True)`).
    async fn ephemeral_notice(
        client: &reqwest::Client,
        token: &str,
        interaction_id: &str,
        interaction_token: &str,
        text: &str,
    ) {
        interaction_callback(
            client,
            token,
            interaction_id,
            interaction_token,
            json!({"type": 4, "data": {"content": text, "flags": 64}}),
        )
        .await;
    }

    /// UPDATE_MESSAGE response (callback type 7) — restyles the embed and
    /// disables the buttons in place.
    async fn update_message_response(
        client: &reqwest::Client,
        token: &str,
        interaction_id: &str,
        interaction_token: &str,
        message: &Value,
        embed: Value,
    ) {
        let mut payload = json!({"type": 7, "message": {"embeds": [embed]}});
        if let Some(components) = disable_components(message.get("components")) {
            payload["message"]["components"] = components;
        }
        interaction_callback(client, token, interaction_id, interaction_token, payload).await;
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// axum mock of Discord REST — logs (method, path, body) per call.
        async fn spawn_discord_api(
            log: Arc<std::sync::Mutex<Vec<(String, String, Value)>>>,
        ) -> String {
            use axum::extract::State;
            use axum::routing::{patch, post};
            type Log = Arc<std::sync::Mutex<Vec<(String, String, Value)>>>;
            let app = axum::Router::new()
                .route(
                    "/channels/:channel/messages",
                    post(
                        move |State(log): State<Log>,
                         axum::extract::Path(channel): axum::extract::Path<String>,
                         axum::Json(body): axum::Json<Value>| async move {
                            log.lock().unwrap().push((
                                "POST".into(),
                                format!("/channels/{channel}/messages"),
                                body,
                            ));
                            axum::Json(json!({"id": "777"}))
                        },
                    ),
                )
                .route(
                    "/channels/:channel/messages/:message",
                    patch(
                        move |State(log): State<Log>,
                         axum::extract::Path((channel, message)): axum::extract::Path<(
                            String,
                            String,
                        )>,
                         axum::Json(body): axum::Json<Value>| async move {
                            log.lock().unwrap().push((
                                "PATCH".into(),
                                format!("/channels/{channel}/messages/{message}"),
                                body,
                            ));
                            axum::Json(json!({}))
                        },
                    ),
                )
                .route(
                    "/interactions/:id/:token/callback",
                    post(
                        move |State(log): State<Log>,
                         axum::extract::Path((id, _token)): axum::extract::Path<(String, String)>,
                         axum::Json(body): axum::Json<Value>| async move {
                            log.lock().unwrap().push((
                                "POST".into(),
                                format!("/interactions/{id}/callback"),
                                body,
                            ));
                            axum::Json(json!({}))
                        },
                    ),
                )
                .with_state(log);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            format!("http://{addr}")
        }

        fn interaction_payload(clarify_id: &str, token: &str) -> Value {
            json!({
                "type": 3,
                "id": "int-1",
                "token": "int-token",
                "channel_id": "42",
                "data": {"custom_id": format!("clarify:{clarify_id}:{token}")},
                "user": {"id": "7", "username": "ann", "global_name": "Ann"},
                "message": {
                    "embeds": [{
                        "title": "❓ ulnclaw needs your input",
                        "description": "Pick",
                        "color": 0xE67E22,
                    }],
                    "components": [{
                        "type": 1,
                        "components": [
                            {"type": 2, "style": 1, "label": "1. Alpha", "custom_id": format!("clarify:{clarify_id}:0")},
                            {"type": 2, "style": 1, "label": "2. Beta", "custom_id": format!("clarify:{clarify_id}:1")},
                        ],
                    }],
                },
            })
        }

        fn authorized_cfg() -> DiscordConfig {
            DiscordConfig {
                allowed_channel_ids: vec!["42".into()],
                ..Default::default()
            }
        }

        #[test]
        fn discord_button_label_smart_truncation() {
            // Short labels pass through with the numeric prefix.
            assert_eq!(clarify_button_label(0, "Yes"), "1. Yes");
            // Long labels cut at a word boundary in the trailing half and
            // end with an ellipsis, within the 80 UTF-16 unit cap.
            let long = "The quick brown fox jumps over the lazy dog and keeps on running through the forest forever";
            let label = clarify_button_label(1, long);
            assert!(label.starts_with("2. "));
            assert!(label.ends_with('…'));
            assert!(utf16_len(&label) <= BUTTON_LABEL_LIMIT);
            let body = label.trim_start_matches("2. ");
            let kept = body.trim_end_matches('…');
            // Kept text is a clean prefix of the choice cut at a word
            // boundary (next char is a space or the end).
            assert!(long.starts_with(kept));
            let rest = &long[kept.len()..];
            assert!(rest.starts_with(' ') || rest.is_empty());
        }

        #[test]
        fn discord_prompt_timeout_clamps_like_hermes() {
            assert_eq!(prompt_timeout(0), std::time::Duration::from_secs(300));
            assert_eq!(prompt_timeout(5), std::time::Duration::from_secs(30));
            assert_eq!(prompt_timeout(4000), std::time::Duration::from_secs(900));
            assert_eq!(prompt_timeout(300), std::time::Duration::from_secs(300));
        }

        #[tokio::test]
        async fn discord_send_clarify_posts_embed_and_buttons() {
            let _env_guard = crate::models_dev::test_env_lock();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
            let base = spawn_discord_api(log.clone()).await;
            std::env::set_var("DISCORD_API_BASE", &base);
            let ok = send_clarify_message(
                "TOKEN",
                "42",
                "cid000000001",
                "Pick one",
                &["Alpha".into(), "Beta".into()],
                300,
            )
            .await;
            std::env::remove_var("DISCORD_API_BASE");
            assert!(ok);
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            let (method, path, body) = &reqs[0];
            assert_eq!(method, "POST");
            assert_eq!(path, "/channels/42/messages");
            // Self-contained content mirrors the embed (some clients hide
            // embeds); the embed carries the question; buttons the choices.
            let content = body["content"].as_str().unwrap();
            assert!(content.starts_with("❓ **ulnclaw needs your input**\n\nPick one"));
            assert!(content.contains("Pick one below, or click ✏️ Other to type a custom answer."));
            assert_eq!(body["embeds"][0]["title"], "❓ ulnclaw needs your input");
            assert_eq!(body["embeds"][0]["description"], "Pick one");
            assert_eq!(body["embeds"][0]["color"], 0xE67E22);
            let rows = body["components"].as_array().unwrap();
            assert_eq!(rows.len(), 1);
            let buttons = rows[0]["components"].as_array().unwrap();
            assert_eq!(buttons.len(), 3);
            assert_eq!(buttons[0]["label"], "1. Alpha");
            assert_eq!(buttons[0]["style"], 1);
            assert_eq!(buttons[0]["custom_id"], "clarify:cid000000001:0");
            assert_eq!(buttons[1]["custom_id"], "clarify:cid000000001:1");
            assert_eq!(buttons[2]["label"], "✏️ Other (type answer)");
            assert_eq!(buttons[2]["style"], 2);
            assert_eq!(buttons[2]["custom_id"], "clarify:cid000000001:other");
        }

        #[tokio::test]
        async fn discord_send_clarify_caps_choices_at_24() {
            let _env_guard = crate::models_dev::test_env_lock();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
            let base = spawn_discord_api(log.clone()).await;
            std::env::set_var("DISCORD_API_BASE", &base);
            let choices: Vec<String> = (0..30).map(|i| format!("Choice {i}")).collect();
            let ok = send_clarify_message("TOKEN", "42", "cid000000002", "Pick", &choices, 300)
                .await;
            std::env::remove_var("DISCORD_API_BASE");
            assert!(ok);
            let reqs = log.lock().unwrap();
            let rows = reqs[0].2["components"].as_array().unwrap();
            // 24 choices + Other = 25 buttons = 5 rows of 5 (Discord max).
            assert_eq!(rows.len(), 5);
            let flat: Vec<&Value> = rows
                .iter()
                .flat_map(|row| row["components"].as_array().unwrap())
                .collect();
            assert_eq!(flat.len(), 25);
            assert_eq!(flat[23]["custom_id"], "clarify:cid000000002:23");
            assert_eq!(flat[24]["custom_id"], "clarify:cid000000002:other");
        }

        #[tokio::test]
        async fn discord_interaction_numeric_choice_resolves() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
            let base = spawn_discord_api(log.clone()).await;
            std::env::set_var("DISCORD_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-discord-42",
                "Pick",
                &["Alpha".into(), "Beta".into()],
                false,
            );
            let clarify_id = handle.clarify_id.clone();
            let payload = interaction_payload(&clarify_id, "1");
            handle_interaction_create(&authorized_cfg(), "TOKEN", payload, None).await;
            std::env::remove_var("DISCORD_API_BASE");
            assert_eq!(handle.rx.await.unwrap(), "Beta");
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            let (method, path, body) = &reqs[0];
            assert_eq!(method, "POST");
            assert_eq!(path, "/interactions/int-1/callback");
            // UPDATE_MESSAGE with green embed + resolution footer and
            // disabled buttons.
            assert_eq!(body["type"], 7);
            let embed = &body["message"]["embeds"][0];
            assert_eq!(embed["color"], 0x2ECC71);
            assert_eq!(embed["footer"]["text"], "Answered by Ann: Beta");
            let rows = body["message"]["components"].as_array().unwrap();
            assert!(rows.iter().all(|row| row["components"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b["disabled"].as_bool() == Some(true))));
        }

        #[tokio::test]
        async fn discord_interaction_other_flips_text_capture() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
            let base = spawn_discord_api(log.clone()).await;
            std::env::set_var("DISCORD_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-discord-42",
                "Pick",
                &["Alpha".into(), "Beta".into()],
                false,
            );
            let clarify_id = handle.clarify_id.clone();
            let payload = interaction_payload(&clarify_id, "other");
            handle_interaction_create(&authorized_cfg(), "TOKEN", payload, None).await;
            std::env::remove_var("DISCORD_API_BASE");
            // Entry survives in text-capture mode; a typed message resolves
            // it through the session intercept.
            let pending = crate::clarify_gateway::pending_for_session("platform-discord-42")
                .expect("entry must survive the Other click");
            assert!(pending.awaiting_text);
            assert!(crate::clarify_gateway::resolve(&clarify_id, "typed answer"));
            assert_eq!(handle.rx.await.unwrap(), "typed answer");
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            let body = &reqs[0].2;
            assert_eq!(body["type"], 7);
            let embed = &body["message"]["embeds"][0];
            assert_eq!(embed["color"], 0x3498DB);
            assert_eq!(embed["footer"]["text"], "Awaiting typed response from Ann…");
        }

        #[tokio::test]
        async fn discord_interaction_stale_prompt_notice() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
            let base = spawn_discord_api(log.clone()).await;
            std::env::set_var("DISCORD_API_BASE", &base);
            // No entry registered — a click on an answered/unknown prompt.
            let payload = interaction_payload("zzzzzzzzzzzz", "0");
            handle_interaction_create(&authorized_cfg(), "TOKEN", payload, None).await;
            std::env::remove_var("DISCORD_API_BASE");
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            let body = &reqs[0].2;
            assert_eq!(body["type"], 4);
            assert_eq!(body["data"]["flags"], 64);
            assert_eq!(body["data"]["content"], "This prompt has already been answered~");
        }

        #[tokio::test]
        async fn discord_interaction_unauthorized_notice() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
            let base = spawn_discord_api(log.clone()).await;
            std::env::set_var("DISCORD_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-discord-42",
                "Pick",
                &["Alpha".into()],
                false,
            );
            let clarify_id = handle.clarify_id.clone();
            let payload = interaction_payload(&clarify_id, "0");
            // Channel 42 is not allowlisted and no pairing store exists.
            let cfg = DiscordConfig {
                allowed_channel_ids: vec!["999".into()],
                ..Default::default()
            };
            handle_interaction_create(&cfg, "TOKEN", payload, None).await;
            std::env::remove_var("DISCORD_API_BASE");
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            assert_eq!(
                reqs[0].2["data"]["content"],
                "You're not authorized to answer this prompt~"
            );
            // The waiter is untouched — the legitimate user can still answer.
            assert!(crate::clarify_gateway::contains(&clarify_id));
            assert!(crate::clarify_gateway::resolve(&clarify_id, "Alpha"));
        }

        #[tokio::test]
        async fn discord_prompt_expiry_disables_unanswered_prompt() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
            let base = spawn_discord_api(log.clone()).await;
            std::env::set_var("DISCORD_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-discord-42",
                "Pick",
                &["Alpha".into()],
                false,
            );
            spawn_prompt_expiry(
                "TOKEN",
                "42",
                "777",
                &handle.clarify_id,
                "Pick",
                vec!["Alpha".into()],
                std::time::Duration::from_millis(200),
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            std::env::remove_var("DISCORD_API_BASE");
            let reqs = log.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            let (method, path, body) = &reqs[0];
            assert_eq!(method, "PATCH");
            assert_eq!(path, "/channels/42/messages/777");
            assert_eq!(body["embeds"][0]["color"], 0x99AAB5);
            assert_eq!(body["embeds"][0]["footer"]["text"], "⏱ Prompt expired — no action taken");
            let rows = body["components"].as_array().unwrap();
            assert!(rows.iter().all(|row| row["components"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b["disabled"].as_bool() == Some(true))));
        }

        #[tokio::test]
        async fn discord_prompt_expiry_skips_text_capture() {
            let _env_guard = crate::models_dev::test_env_lock();
            let _clarify_guard = crate::clarify_gateway::test_lock().lock().unwrap();
            crate::clarify_gateway::reset_for_tests();
            let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
            let base = spawn_discord_api(log.clone()).await;
            std::env::set_var("DISCORD_API_BASE", &base);
            let handle = crate::clarify_gateway::register(
                "platform-discord-42",
                "Pick",
                &["Alpha".into()],
                false,
            );
            assert!(crate::clarify_gateway::mark_awaiting_text(&handle.clarify_id));
            spawn_prompt_expiry(
                "TOKEN",
                "42",
                "777",
                &handle.clarify_id,
                "Pick",
                vec!["Alpha".into()],
                std::time::Duration::from_millis(200),
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            std::env::remove_var("DISCORD_API_BASE");
            // No PATCH — the "Awaiting typed response" footer stays and the
            // typed answer can still arrive (documented hermes divergence).
            assert!(log.lock().unwrap().is_empty());
            assert!(crate::clarify_gateway::resolve(&handle.clarify_id, "late answer"));
        }
    }
}

// ---------------------------------------------------------------------------
// Slack — Socket Mode websocket + chat.postMessage
// ---------------------------------------------------------------------------

pub mod slack {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    struct Sender {
        bot_token: String,
    }

    #[async_trait::async_trait]
    impl PlatformSender for Sender {
        async fn send_text(&self, chat_id: &str, text: &str) {
            post_message(&self.bot_token, chat_id, text).await;
        }
    }

    pub async fn run(cfg: SlackConfig, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<crate::pairing::PairingStore>>) {
        let Some(bot_token) = resolve_token(&cfg.bot_token, "SLACK_BOT_TOKEN") else {
            eprintln!("[slack] disabled: no bot_token configured (set messaging.slack.bot_token or SLACK_BOT_TOKEN)");
            return;
        };
        let Some(app_token) = resolve_token(&cfg.app_token, "SLACK_APP_TOKEN") else {
            eprintln!("[slack] disabled: no app_token configured (set messaging.slack.app_token or SLACK_APP_TOKEN)");
            return;
        };
        register_platform_sender(
            "slack",
            Arc::new(Sender {
                bot_token: bot_token.clone(),
            }),
        );
        loop {
            match run_socket_session(&cfg, &bot_token, &app_token, dispatcher.clone(), pairing.clone()).await {
                Ok(()) => {}
                Err(e) => eprintln!("[slack] socket session ended: {e} — reconnecting in 5s"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn open_socket_url(client: &reqwest::Client, app_token: &str) -> Result<String> {
        let response = client
            .post(format!("{}/apps.connections.open", slack_api_base()))
            .header("Authorization", format!("Bearer {app_token}"))
            .send()
            .await
            .map_err(|e| AgentError::Tool(format!("slack open: {e}")))?;
        let value: Value = response.json().await.map_err(|e| AgentError::Tool(format!("slack open parse: {e}")))?;
        if !value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Err(AgentError::Tool(format!(
                "slack apps.connections.open: {}",
                value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
            )));
        }
        value
            .get("url")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| AgentError::Tool("slack open: no url".into()))
    }

    async fn run_socket_session(
        cfg: &SlackConfig,
        bot_token: &str,
        app_token: &str,
        dispatcher: Arc<Dispatcher>,
        pairing: Option<Arc<crate::pairing::PairingStore>>,
    ) -> Result<()> {
        let client = reqwest::Client::new();
        let url = open_socket_url(&client, app_token).await?;
        let (ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| AgentError::Tool(format!("slack connect: {e}")))?;
        eprintln!("[slack] socket-mode connected");
        let (mut sink, mut stream) = ws.split();
        let own_bot_id: Option<String> = None;
        while let Some(Ok(message)) = stream.next().await {
            let WsMessage::Text(text) = message else { continue };
            let Ok(envelope) = serde_json::from_str::<Value>(&text) else { continue };
            let envelope_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let envelope_id = envelope.get("envelope_id").and_then(|v| v.as_str()).unwrap_or("");
            if envelope_type == "hello" {
                continue;
            }
            // Ack every envelope promptly (Slack retries unacked ones).
            if !envelope_id.is_empty() {
                let ack = json!({"envelope_id": envelope_id});
                sink.send(WsMessage::Text(ack.to_string())).await.ok();
            }
            if envelope_type != "events_api" {
                continue;
            }
            let event = envelope.pointer("/payload/event").cloned().unwrap_or(json!({}));
            let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if event_type == "app_mention" || (event_type == "message" && event.get("subtype").is_none()) {
                if let Some(bot_id) = event.get("bot_id").and_then(|v| v.as_str()) {
                    if own_bot_id.as_deref() == Some(bot_id) || event.get("bot_profile").is_some() {
                        continue;
                    }
                }
                let channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let has_files = event
                    .get("files")
                    .and_then(|v| v.as_array())
                    .map(|files| !files.is_empty())
                    .unwrap_or(false);
                if channel.is_empty() || (text.is_empty() && !has_files) {
                    continue;
                }
                // Strip a leading <@BOTID> mention so the agent sees clean text.
                let text = strip_mention(&text);
                // Slack file URLs are private — the bot bearer authenticates
                // the download (hermes slack adapter behavior).
                let attachments = {
                    let client = reqwest::Client::new();
                    let auth = format!("Bearer {bot_token}");
                    let mut downloaded = Vec::new();
                    if let Some(files) = event.get("files").and_then(|v| v.as_array()) {
                        for file in files {
                            let Some(url) = file.get("url_private").and_then(|v| v.as_str()) else {
                                continue;
                            };
                            let name = file.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let mime = file.get("mimetype").and_then(|v| v.as_str()).unwrap_or("");
                            let size = file.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                            if let Some(attachment) =
                                download_to_cache(&client, url, Some(&auth), mime, name, size).await
                            {
                                downloaded.push(attachment);
                            }
                        }
                    }
                    downloaded
                };
                let mut message_event = MessageEvent {
                    platform: "slack".into(),
                    chat_id: channel.clone(),
                    sender_id: event.get("user").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    sender_name: event.get("user").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    text,
                    message_id: event.get("ts").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    attachments,
                };
                // Plugin gate before auth (hermes ordering).
                if !pre_gateway_dispatch_gate(&mut message_event).await {
                    continue;
                }
                // Auth union: configured allowlist OR an approved pairing code.
                let authorized = allowlisted(&cfg.allowed_channel_ids, &channel)
                    || pairing
                        .as_ref()
                        .map(|store| store.is_approved("slack", &message_event.sender_id))
                        .unwrap_or(false);
                if !authorized {
                    eprintln!(
                        "[slack] refusing message from channel {channel} — add it to \
                         messaging.slack.allowed_channel_ids or approve a pairing code"
                    );
                    if let Some(store) = pairing.as_ref() {
                        if let Some(reply) = pairing_offer(store, "slack", &message_event.sender_id, &message_event.sender_name) {
                            post_message(bot_token, &channel, &reply).await;
                        }
                    }
                    continue;
                }
                // hermes typing parity: the assistant-thread status rides the
                // event's thread, falling back to the message's own ts for
                // top-level messages (hermes synthetic-thread session keying).
                let raw_thread_ts = event
                    .get("thread_ts")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let typing_thread = if raw_thread_ts.is_empty() {
                    message_event.message_id.clone()
                } else {
                    raw_thread_ts
                };
                let typing_status_cfg = cfg.typing_status_text.clone();
                let dispatcher = dispatcher.clone();
                let bot_token = bot_token.to_string();
                tokio::spawn(async move {
                    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
                    let heartbeat = if typing_thread.is_empty() {
                        None
                    } else {
                        Some(tokio::spawn(typing_heartbeat(
                            bot_token.clone(),
                            message_event.chat_id.clone(),
                            typing_thread,
                            typing_status_cfg,
                            std::time::Duration::from_secs(2),
                            stop_rx,
                        )))
                    };
                    let outcome = match dispatcher.handle_event(message_event.clone()).await {
                        Ok(outcome) => outcome,
                        Err(e) => crate::messaging::DispatchOutcome {
                            reply: format!("error: {e}"),
                            transcript_echoes: Vec::new(),
                        },
                    };
                    for echo in &outcome.transcript_echoes {
                        post_message(&bot_token, &message_event.chat_id, echo).await;
                    }
                    let reply = outcome.reply;
                    // MEDIA:<path> tags become native Slack file uploads
                    // (hermes files.upload flow); the rest posts as text.
                    let (reply_text, media_paths) = extract_media_tags(&reply);
                    if !reply_text.trim().is_empty() {
                        post_message(&bot_token, &message_event.chat_id, &reply_text).await;
                    }
                    for path in media_paths {
                        upload_file(&bot_token, &message_event.chat_id, &path).await;
                    }
                    // hermes stop_typing: clear the status once the reply is out.
                    if let Some(handle) = heartbeat {
                        let _ = stop_tx.send(true);
                        let _ = handle.await;
                    }
                });
            }
        }
        Ok(())
    }

    pub(crate) fn strip_mention(text: &str) -> String {
        let trimmed = text.trim_start();
        if trimmed.starts_with("<@") {
            if let Some(end) = trimmed.find('>') {
                return trimmed[end + 1..].trim().to_string();
            }
        }
        text.to_string()
    }

    /// chat.postMessage with Slack's ~40k limit chunked at 3500 chars.
    pub async fn post_message(bot_token: &str, channel: &str, text: &str) {
        let client = reqwest::Client::new();
        for chunk in chunk_text(text, 3500) {
            let result = client
                .post(format!("{}/chat.postMessage", slack_api_base()))
                .header("Authorization", format!("Bearer {bot_token}"))
                .json(&json!({"channel": channel, "text": chunk}))
                .send()
                .await;
            match result {
                Ok(response) => {
                    if let Ok(value) = response.json::<Value>().await {
                        if !value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                            eprintln!(
                                "[slack] postMessage failed: {}",
                                value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                            );
                        }
                    }
                }
                Err(e) => eprintln!("[slack] postMessage failed: {e}"),
            }
        }
    }

    /// Upload a local file as a native Slack file (MEDIA: delivery).
    /// Modern three-step flow: `files.getUploadURLExternal` → PUT the
    /// binary to the signed URL → `files.completeUploadExternal` into the
    /// channel (the legacy one-shot `files.upload` is retired).
    pub async fn upload_file(bot_token: &str, channel: &str, path: &std::path::Path) {
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("[slack] cannot read {}: {e}", path.display());
                return;
            }
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let client = reqwest::Client::new();
        let auth = format!("Bearer {bot_token}");

        // Step 1: request an upload URL + file id.
        let url_response = client
            .get(format!("{}/files.getUploadURLExternal", slack_api_base()))
            .header("Authorization", &auth)
            .query(&[("filename", &file_name), ("length", &data.len().to_string())])
            .send()
            .await;
        let url_value = match url_response {
            Ok(response) => match response.json::<Value>().await {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("[slack] getUploadURLExternal parse failed: {e}");
                    return;
                }
            },
            Err(e) => {
                eprintln!("[slack] getUploadURLExternal failed: {e}");
                return;
            }
        };
        if !url_value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            eprintln!(
                "[slack] getUploadURLExternal failed: {}",
                url_value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
            );
            return;
        }
        let upload_url = match url_value.get("upload_url").and_then(|v| v.as_str()) {
            Some(url) => url.to_string(),
            None => {
                eprintln!("[slack] getUploadURLExternal returned no upload_url");
                return;
            }
        };
        let file_id = match url_value.get("file_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                eprintln!("[slack] getUploadURLExternal returned no file_id");
                return;
            }
        };

        // Step 2: PUT the raw bytes to the signed URL.
        let put_response = client
            .put(&upload_url)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await;
        match put_response {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                eprintln!("[slack] file upload PUT failed ({})", response.status());
                return;
            }
            Err(e) => {
                eprintln!("[slack] file upload PUT failed: {e}");
                return;
            }
        }

        // Step 3: complete the upload, sharing into the channel.
        let files_json = serde_json::to_string(&json!([{ "id": file_id, "title": file_name }]))
            .unwrap_or_default();
        let channels_json = serde_json::to_string(&json!([channel])).unwrap_or_default();
        let complete = client
            .post(format!("{}/files.completeUploadExternal", slack_api_base()))
            .header("Authorization", &auth)
            .form(&[
                ("files", files_json.as_str()),
                ("channel_id", channel),
            ])
            .send()
            .await;
        match complete {
            Ok(response) => match response.json::<Value>().await {
                Ok(value) => {
                    if !value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        eprintln!(
                            "[slack] completeUploadExternal failed: {}",
                            value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                        );
                    }
                    let _ = channels_json;
                }
                Err(e) => eprintln!("[slack] completeUploadExternal parse failed: {e}"),
            },
            Err(e) => eprintln!("[slack] completeUploadExternal failed: {e}"),
        }
    }

    /// Slack Web API base URL — normally `https://slack.com/api`; the
    /// `SLACK_API_BASE` override exists for tests and corporate proxies
    /// (mirrors the qqbot `QQ_PORTAL_HOST` pattern).
    pub(crate) fn slack_api_base() -> String {
        env_or_none("SLACK_API_BASE").unwrap_or_else(|| "https://slack.com/api".to_string())
    }

    fn env_or_none(name: &str) -> Option<String> {
        std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }

    /// hermes `assistant.threads.setStatus` — show (or clear, when `status`
    /// is empty) the assistant thread status line next to the bot name.
    /// Requires the `assistant:write` scope; failures are logged but never
    /// fatal (hermes silently degrades to no indicator).
    pub async fn set_thread_status(bot_token: &str, channel: &str, thread_ts: &str, status: &str) -> bool {
        let client = reqwest::Client::new();
        let result = client
            .post(format!("{}/assistant.threads.setStatus", slack_api_base()))
            .header("Authorization", format!("Bearer {bot_token}"))
            .json(&json!({"channel_id": channel, "thread_ts": thread_ts, "status": status}))
            .send()
            .await;
        match result {
            Ok(response) => match response.json::<Value>().await {
                Ok(value) => {
                    let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    if !ok {
                        eprintln!(
                            "[slack] assistant.threads.setStatus failed: {}",
                            value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                        );
                    }
                    ok
                }
                Err(e) => {
                    eprintln!("[slack] assistant.threads.setStatus parse failed: {e}");
                    false
                }
            },
            Err(e) => {
                eprintln!("[slack] assistant.threads.setStatus failed: {e}");
                false
            }
        }
    }

    /// hermes `send_typing` status selection: configured text always wins;
    /// after 30 s the default becomes an elapsed-time heartbeat (hermes
    /// #45702) so a long turn reads as working, not stuck.
    pub fn typing_status_text(elapsed: std::time::Duration, configured: Option<&str>) -> String {
        if let Some(text) = configured.filter(|t| !t.is_empty()) {
            return text.to_string();
        }
        let secs = elapsed.as_secs();
        if secs >= 30 {
            let mins = secs / 60;
            let rem = secs % 60;
            let human = if mins > 0 {
                format!("{mins}m{rem:02}s")
            } else {
                format!("{rem}s")
            };
            format!("still working… ({human})")
        } else {
            "is thinking...".to_string()
        }
    }

    /// hermes `_keep_typing` (Slack flavor): refresh the assistant-thread
    /// status every `interval` (hermes default 2 s) until `stop` flips,
    /// each call bounded so a slow round-trip cannot stall the cadence
    /// (hermes `max(0.25, min(1.5, interval - 0.25))`); on stop, clear the
    /// indicator (hermes `stop_typing` posts an empty status).
    pub async fn typing_heartbeat(
        bot_token: String,
        channel: String,
        thread_ts: String,
        configured: Option<String>,
        interval: std::time::Duration,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) {
        let started = std::time::Instant::now();
        let call_timeout =
            std::time::Duration::from_secs_f64((interval.as_secs_f64() - 0.25).clamp(0.25, 1.5));
        loop {
            if *stop.borrow() {
                break;
            }
            let status = typing_status_text(started.elapsed(), configured.as_deref());
            let _ = tokio::time::timeout(
                call_timeout,
                set_thread_status(&bot_token, &channel, &thread_ts, &status),
            )
            .await;
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        break;
                    }
                }
            }
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs_f64(1.5),
            set_thread_status(&bot_token, &channel, &thread_ts, ""),
        )
        .await;
    }
}

/// Split text into chunks no longer than `max` chars, preferring newline
/// boundaries (hermes message splitting).
pub(crate) fn chunk_text(text: &str, max: usize) -> Vec<String> {
    if text.chars().count() <= max {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if current.chars().count() + line.chars().count() > max && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        // A single line longer than max must be hard-split.
        let mut rest = line;
        while rest.chars().count() > max {
            let split_at = rest
                .char_indices()
                .nth(max)
                .map(|(idx, _)| idx)
                .unwrap_or(rest.len());
            let (head, tail) = rest.split_at(split_at);
            out.push(head.to_string());
            rest = tail;
        }
        current.push_str(rest);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn approval_command_parsing() {
        let cmd = parse_approval_command("/approve").unwrap();
        assert!(cmd.approve && !cmd.all && cmd.choice == crate::approval_gateway::CHOICE_ONCE);
        let cmd = parse_approval_command("/approve all session").unwrap();
        assert!(cmd.all && cmd.choice == crate::approval_gateway::CHOICE_SESSION);
        let cmd = parse_approval_command("/approve always").unwrap();
        assert_eq!(cmd.choice, crate::approval_gateway::CHOICE_ALWAYS);
        let cmd = parse_approval_command("/deny all because reasons").unwrap();
        assert!(!cmd.approve && cmd.all && cmd.choice == crate::approval_gateway::CHOICE_DENY);
        assert!(parse_approval_command("approve").is_none()); // slash form only
        assert!(parse_approval_command("/approve2").is_none());
        assert!(parse_approval_command("hello world").is_none());
    }

    #[test]
    fn approval_text_format() {
        let text = format_approval_text("rm -rf /", "dangerous command", true, true, false);
        assert!(text.contains("Dangerous command requires approval"));
        assert!(text.contains("```\nrm -rf /\n```"));
        assert!(text.contains("Reason: dangerous command"));
        assert!(text.contains("/approve session"));
        assert!(text.contains("/approve always"));
        assert!(text.contains("/deny"));
        let smart = format_approval_text("c", "d", true, true, true);
        assert!(smart.contains("Smart DENY"));
        assert!(!smart.contains("/approve session"));
        // Long commands truncate to 200 chars + ellipsis.
        let long = format_approval_text(&"x".repeat(500), "d", true, true, false);
        assert!(long.contains(&"x".repeat(200)));
        assert!(!long.contains(&"x".repeat(201)));
    }

    #[test]
    fn approval_intercept_flow() {
        let session = "platform-teams-intercept";
        // No pending approvals: informational reply.
        let cmd = parse_approval_command("/approve").unwrap();
        assert_eq!(apply_approval_command(session, cmd), "No pending approvals.");
        // Pending approval resolves oldest-first.
        let mut first = crate::approval_gateway::register(session, "cmd-a", "d", false, true, true);
        let mut second = crate::approval_gateway::register(session, "cmd-b", "d", false, true, true);
        let cmd = parse_approval_command("/approve").unwrap();
        assert_eq!(apply_approval_command(session, cmd), "✅ Allowed (once)");
        assert_eq!(first.rx.try_recv().unwrap(), "once");
        assert!(second.rx.try_recv().is_err());
        let cmd = parse_approval_command("/deny all").unwrap();
        assert_eq!(apply_approval_command(session, cmd), "❌ Denied");
        assert_eq!(second.rx.try_recv().unwrap(), "deny");
    }

    use super::*;

    #[test]
    fn config_defaults_disabled() {
        let cfg = MessagingConfig::default();
        assert!(!cfg.telegram.enabled && !cfg.discord.enabled && !cfg.slack.enabled);
    }

    #[test]
    fn allowlist_gate() {
        assert!(allowlisted(&["123".into()], "123"));
        assert!(!allowlisted(&["123".into()], "456"));
        assert!(!allowlisted(&[], "123"), "empty allowlist fails closed");
    }

    #[test]
    fn extract_media_tags_splits_paths_and_text() {
        let dir = std::env::temp_dir().join(format!(
            "ulnclaw-media-tags-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let media = dir.join("clip.mp4");
        std::fs::write(&media, b"fake").unwrap();

        // Existing path is extracted; missing paths stay as literal text.
        let reply = format!(
            "Here is your clip!\nMEDIA: {}\nMEDIA: /does/not/exist.png\nEnjoy!",
            media.display()
        );
        let (text, paths) = extract_media_tags(&reply);
        assert_eq!(paths, vec![media.clone()]);
        assert!(text.contains("Here is your clip!"));
        assert!(text.contains("Enjoy!"));
        assert!(text.contains("/does/not/exist.png"), "missing files stay literal");
        assert!(!text.contains(&format!("MEDIA: {}", media.display())));

        // Tilde expansion (restore HOME — env is process-global).
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.to_string_lossy().to_string());
        let home_media = dir.join("home.mp3");
        std::fs::write(&home_media, b"x").unwrap();
        let (text, paths) = extract_media_tags("MEDIA: ~/home.mp3");
        assert_eq!(paths, vec![home_media]);
        assert!(text.trim().is_empty());
        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }

        // No tags → unchanged.
        let (text, paths) = extract_media_tags("plain reply");
        assert!(paths.is_empty());
        assert_eq!(text, "plain reply");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn attachment_note_lists_paths_and_hints() {
        assert_eq!(attachment_note(&[]), "");
        let note = attachment_note(&[MediaAttachment {
            path: std::path::PathBuf::from("/tmp/cache/abc.jpg"),
            mime: "image/jpeg".into(),
            bytes: 1234,
            original_name: "pic.jpg".into(),
        }]);
        assert!(note.contains("/tmp/cache/abc.jpg"));
        assert!(note.contains("image/jpeg"));
        assert!(note.contains("1234 bytes"));
        assert!(note.contains("pic.jpg"));
        assert!(note.contains("vision_analyze"));
    }

    #[test]
    fn session_key_is_deterministic() {
        let event = MessageEvent {
            platform: "telegram".into(),
            chat_id: "-100123".into(),
            sender_id: "1".into(),
            sender_name: "u".into(),
            text: "hi".into(),
            message_id: "m".into(),
            attachments: Vec::new(),
        };
        assert_eq!(Dispatcher::session_key(&event), "platform-telegram--100123");
    }

    #[test]
    fn chunk_short_text_unchanged() {
        assert_eq!(chunk_text("hello", 10), vec!["hello".to_string()]);
    }

    #[test]
    fn chunk_prefers_newlines() {
        let text = "aaaa\nbbbb\ncccc";
        let chunks = chunk_text(text, 10);
        assert!(chunks.iter().all(|c| c.chars().count() <= 10));
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn chunk_hard_splits_long_lines() {
        let text = "x".repeat(25);
        let chunks = chunk_text(&text, 10);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= 10));
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn slack_mention_stripped() {
        assert_eq!(slack::strip_mention("<@U123> hello"), "hello");
        assert_eq!(slack::strip_mention("plain text"), "plain text");
    }

    #[test]
    fn token_resolution_prefers_config() {
        assert_eq!(resolve_token("abc", "ULNCLAW_NEVER_SET_XYZ").as_deref(), Some("abc"));
        assert_eq!(resolve_token("  ", "ULNCLAW_NEVER_SET_XYZ"), None);
    }

    #[test]
    fn slack_typing_status_text_selection() {
        use std::time::Duration;
        // Default while fresh; elapsed heartbeat after 30 s (hermes #45702).
        assert_eq!(slack::typing_status_text(Duration::from_secs(0), None), "is thinking...");
        assert_eq!(slack::typing_status_text(Duration::from_secs(29), None), "is thinking...");
        assert_eq!(slack::typing_status_text(Duration::from_secs(30), None), "still working… (30s)");
        assert_eq!(slack::typing_status_text(Duration::from_secs(63), None), "still working… (1m03s)");
        assert_eq!(slack::typing_status_text(Duration::from_secs(3661), None), "still working… (61m01s)");
        // Configured text always wins, even after the heartbeat threshold;
        // an empty configured value falls back to the defaults.
        assert_eq!(slack::typing_status_text(Duration::from_secs(90), Some("brewing…")), "brewing…");
        assert_eq!(slack::typing_status_text(Duration::from_secs(0), Some("")), "is thinking...");
    }

    #[test]
    fn slack_api_base_env_override() {
        let _guard = crate::models_dev::test_env_lock();
        std::env::remove_var("SLACK_API_BASE");
        assert_eq!(slack::slack_api_base(), "https://slack.com/api");
        std::env::set_var("SLACK_API_BASE", "http://127.0.0.1:9");
        assert_eq!(slack::slack_api_base(), "http://127.0.0.1:9");
        std::env::remove_var("SLACK_API_BASE");
    }

    async fn spawn_slack_status_server(
        log: Arc<Mutex<Vec<(String, Value)>>>,
        response_ok: bool,
    ) -> String {
        use axum::extract::State;
        use axum::routing::post;
        let app = axum::Router::new()
            .route(
                "/assistant.threads.setStatus",
                post(
                    move |State(log): State<Arc<Mutex<Vec<(String, Value)>>>>,
                     headers: axum::http::HeaderMap,
                     axum::Json(body): axum::Json<Value>| async move {
                        let auth = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        log.lock().await.push((auth, body));
                        axum::Json(json!({ "ok": response_ok }))
                    },
                ),
            )
            .with_state(log);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn slack_set_thread_status_posts_assistant_payload() {
        let _guard = crate::models_dev::test_env_lock();
        let log: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let base = spawn_slack_status_server(log.clone(), true).await;
        std::env::set_var("SLACK_API_BASE", &base);
        assert!(slack::set_thread_status("xoxb-test", "C123", "111.222", "is thinking...").await);
        std::env::remove_var("SLACK_API_BASE");
        let entries = log.lock().await.clone();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "Bearer xoxb-test");
        assert_eq!(entries[0].1["channel_id"], "C123");
        assert_eq!(entries[0].1["thread_ts"], "111.222");
        assert_eq!(entries[0].1["status"], "is thinking...");
    }

    #[tokio::test]
    async fn slack_set_thread_status_reports_api_failure() {
        let _guard = crate::models_dev::test_env_lock();
        let log: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let base = spawn_slack_status_server(log.clone(), false).await;
        std::env::set_var("SLACK_API_BASE", &base);
        let ok = slack::set_thread_status("xoxb-test", "C1", "1.0", "x").await;
        std::env::remove_var("SLACK_API_BASE");
        assert!(!ok);
    }

    #[tokio::test]
    async fn slack_typing_heartbeat_refreshes_then_clears() {
        let _guard = crate::models_dev::test_env_lock();
        let log: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let base = spawn_slack_status_server(log.clone(), true).await;
        std::env::set_var("SLACK_API_BASE", &base);
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(slack::typing_heartbeat(
            "xoxb-test".to_string(),
            "C123".to_string(),
            "111.222".to_string(),
            None,
            std::time::Duration::from_millis(40),
            stop_rx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        stop_tx.send(true).unwrap();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        std::env::remove_var("SLACK_API_BASE");
        let statuses: Vec<String> = log
            .lock()
            .await
            .iter()
            .map(|(_, body)| body["status"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(statuses.len() >= 3, "expected refreshes, got {statuses:?}");
        assert_eq!(statuses[0], "is thinking...");
        // hermes stop_typing: the final call clears the status.
        assert_eq!(statuses.last().map(String::as_str), Some(""));
        assert!(statuses[..statuses.len() - 1].iter().all(|s| s == "is thinking..."));
    }
}
