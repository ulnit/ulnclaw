//! Mixture-of-Agents (MoA) — port of hermes' `agent/moa_loop.py` synthesis
//! path (`aggregate_moa_context`) plus the preset model of
//! `hermes_cli/moa_config.py`.
//!
//! A preset fans the user prompt out to several *reference* models in
//! parallel, joins their answers, and asks an *aggregator* model to
//! synthesize concise guidance. Failed references degrade loudly (or
//! silently, per preset) instead of aborting the turn.

use crate::config::{default_base_url, MoaPreset, MoaSlot, UlncLawConfig};
use crate::error::{AgentError, Result};
use crate::provider::auxiliary::{build_task_provider, is_keyless};
use crate::provider::{Message, Provider, ProviderRequest, Role};
use std::sync::Arc;

/// Sentinel prefix marking a failed reference output (hermes `_is_failed_reference`).
const FAILED_PREFIX: &str = "[failed:";
/// Sentinel prefix marking a skipped reference output.
const SKIPPED_PREFIX: &str = "[skipped:";

/// One reference model's outcome.
#[derive(Debug, Clone)]
pub struct MoaReferenceOutcome {
    /// `provider:model` label.
    pub label: String,
    /// Reference text, or the failure note when the call failed.
    pub text: String,
}

impl MoaReferenceOutcome {
    /// True when the output is a failure/skip sentinel, not real advice.
    pub fn failed(&self) -> bool {
        let trimmed = self.text.trim_start().to_lowercase();
        trimmed.starts_with(FAILED_PREFIX) || trimmed.starts_with(SKIPPED_PREFIX)
    }
}

/// Full MoA run result.
#[derive(Debug, Clone)]
pub struct MoaOutcome {
    /// Aggregator synthesis (joined references when aggregation fails).
    pub synthesis: String,
    /// The wrapped "[Mixture of Agents context …]" block (hermes format).
    pub wrapped: String,
    /// Per-reference outcomes in preset order.
    pub references: Vec<MoaReferenceOutcome>,
    /// Aggregator label.
    pub aggregator_label: String,
}

/// Builds the provider for one slot (injectable for tests).
pub type SlotProviderFactory =
    Arc<dyn Fn(&MoaSlot) -> Result<Arc<dyn Provider>> + Send + Sync>;

/// Production slot factory: builds an OpenAI-compatible or Anthropic client
/// per slot, with credential fallback to the main runtime (mirrors the
/// auxiliary resolution rules).
pub fn default_slot_factory(config: UlncLawConfig) -> SlotProviderFactory {
    Arc::new(move |slot: &MoaSlot| {
        let provider = slot.provider.trim().to_string();
        let model = slot.model.trim().to_string();
        if provider.is_empty() || model.is_empty() {
            return Err(AgentError::config(format!(
                "MoA slot needs provider and model (got '{}')",
                slot.label()
            )));
        }
        let api_key = slot.resolved_api_key(&config);
        if api_key.is_none() && !is_keyless(&provider) {
            return Err(AgentError::config(format!(
                "MoA slot {}: no API key (set api_key, key_env, or the main provider key)",
                slot.label()
            )));
        }
        let base_url = slot
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| default_base_url(&provider));
        build_task_provider(
            &provider,
            &model,
            &base_url,
            api_key.as_deref(),
            config.model.max_retries,
        )
    })
}

fn simple_user_request(prompt: &str, model: &str, temperature: Option<f32>, max_tokens: Option<u32>) -> ProviderRequest {
    ProviderRequest {
        messages: vec![Message {
            role: Role::User,
            content: Some(prompt.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }],
        tools: vec![],
        model: model.to_string(),
        max_tokens,
        temperature,
        stream: false,
        stop: None,
    }
}

/// Run the reference fan-out for one preset.
async fn run_references(
    preset: &MoaPreset,
    prompt: &str,
    factory: &SlotProviderFactory,
) -> Vec<MoaReferenceOutcome> {
    let mut handles = Vec::new();
    for slot in preset.reference_models.iter().filter(|s| s.enabled) {
        let slot = slot.clone();
        let prompt = prompt.to_string();
        let factory = factory.clone();
        let temperature = preset.reference_temperature;
        let max_tokens = preset.reference_max_tokens;
        handles.push(tokio::spawn(async move {
            let label = slot.label();
            let outcome = async {
                let provider = factory(&slot)?;
                let request = simple_user_request(
                    &prompt,
                    &slot.model.trim().to_string(),
                    temperature,
                    max_tokens,
                );
                let response = provider.chat_completion(request).await?;
                Ok::<String, AgentError>(response.content.unwrap_or_default())
            }
            .await;
            match outcome {
                Ok(text) if !text.trim().is_empty() => MoaReferenceOutcome { label, text },
                Ok(_) => MoaReferenceOutcome {
                    label,
                    text: format!("{} empty response]", FAILED_PREFIX),
                },
                Err(e) => MoaReferenceOutcome {
                    label,
                    text: format!("{} {}]", FAILED_PREFIX, e),
                },
            }
        }));
    }
    let mut outcomes = Vec::new();
    for handle in handles {
        if let Ok(outcome) = handle.await {
            outcomes.push(outcome);
        }
    }
    outcomes
}

/// Join successful reference outputs (hermes "Reference {idx} — {label}:").
pub fn join_references(references: &[MoaReferenceOutcome]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut index = 0usize;
    for outcome in references {
        if outcome.failed() {
            continue;
        }
        index += 1;
        parts.push(format!("Reference {} — {}:\n{}", index, outcome.label, outcome.text));
    }
    parts.join("\n\n")
}

/// Failed-reference notice (hermes `_degraded_notice`).
pub fn degraded_notice(references: &[MoaReferenceOutcome], policy: &str) -> String {
    let failed: Vec<&str> = references
        .iter()
        .filter(|r| r.failed())
        .map(|r| r.label.as_str())
        .collect();
    if failed.is_empty() || policy.trim().eq_ignore_ascii_case("silent") {
        return String::new();
    }
    format!("[Reference models unavailable: {}]", failed.join(", "))
}

/// Aggregator synthesis prompt (port of hermes `aggregate_moa_context`).
pub fn aggregator_prompt(user_prompt: &str, joined_references: &str) -> String {
    format!(
        "You are the aggregator in a Mixture of Agents process. Synthesize the \
reference responses into concise, actionable guidance for the main agent. \
Focus on next steps, tool-use strategy, risks, and any disagreements. Do not \
answer the user directly unless that is all that is needed; produce context \
the main agent should use in its normal loop.\n\n\
Original user prompt:\n{}\n\n\
Reference responses:\n{}",
        user_prompt, joined_references
    )
}

/// Run one MoA turn: fan-out → join → aggregate (hermes
/// `aggregate_moa_context` semantics, including the all-references-failed
/// early return and the joined-fallback when the aggregator fails).
pub async fn run_moa_with(
    preset_name: Option<&str>,
    prompt: &str,
    config: &UlncLawConfig,
    factory: SlotProviderFactory,
) -> Result<MoaOutcome> {
    let (preset_name, preset) = config.moa.resolve_preset(preset_name)?;
    let references = run_references(preset, prompt, &factory).await;
    let active_refs: Vec<&MoaReferenceOutcome> = references.iter().collect();
    let successful: Vec<&&MoaReferenceOutcome> =
        active_refs.iter().filter(|r| !r.failed()).collect();
    let degraded = degraded_notice(&references, &preset.degraded_reference_policy);
    let ref_labels: Vec<String> = preset
        .reference_models
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.label())
        .collect();
    let aggregator_label = preset.aggregator.label();

    // All references failed → skip the aggregator call entirely (hermes).
    if successful.is_empty() {
        let notice = if degraded.is_empty() {
            "[Reference models unavailable]".to_string()
        } else {
            degraded
        };
        let wrapped = format!(
            "[Mixture of Agents context — all reference models failed. \
Proceeding without aggregated guidance.]\nReferences: {}\n\n{}",
            ref_labels.join(", "),
            notice
        );
        return Ok(MoaOutcome {
            synthesis: notice.clone(),
            wrapped,
            references,
            aggregator_label,
        });
    }

    let joined = join_references(&references);
    let joined_with_notice = if degraded.is_empty() {
        joined.clone()
    } else {
        format!("{}\n\n{}", joined, degraded)
    };

    let synthesis = match factory(&preset.aggregator) {
        Ok(provider) => {
            let request = ProviderRequest {
                messages: vec![Message {
                    role: Role::User,
                    content: Some(aggregator_prompt(prompt, &joined_with_notice)),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                }],
                tools: vec![],
                model: preset.aggregator.model.trim().to_string(),
                max_tokens: None,
                temperature: preset.aggregator_temperature,
                stream: false,
                stop: None,
            };
            match provider.chat_completion(request).await {
                Ok(response) => response.content.unwrap_or_default(),
                Err(e) => {
                    tracing::warn!("MoA aggregator {} failed: {}", aggregator_label, e);
                    String::new()
                }
            }
        }
        Err(e) => {
            tracing::warn!("MoA aggregator resolution failed: {}", e);
            String::new()
        }
    };
    let synthesis = if synthesis.trim().is_empty() {
        joined_with_notice.clone()
    } else {
        synthesis.trim().to_string()
    };

    let wrapped = format!(
        "[Mixture of Agents context — use this as private guidance for the \
normal agent loop. You may call tools, continue reasoning, or finish \
normally.]\nAggregator: {}\nReferences: {}\n\n{}",
        aggregator_label,
        ref_labels.join(", "),
        synthesis
    );
    let _ = preset_name;
    Ok(MoaOutcome {
        synthesis,
        wrapped,
        references,
        aggregator_label,
    })
}

/// Run one MoA turn with the production slot factory.
pub async fn run_moa(
    config: &UlncLawConfig,
    prompt: &str,
    preset_name: Option<&str>,
) -> Result<MoaOutcome> {
    run_moa_with(preset_name, prompt, config, default_slot_factory(config.clone())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MoaConfig, MoaPreset, MoaSlot};
    use crate::provider::{ProviderResponse, Usage};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Stub provider returning canned text; records request prompts.
    struct StubProvider {
        label: String,
        reply: String,
        log: Arc<Mutex<Vec<(String, String)>>>,
    }

    #[async_trait]
    impl Provider for StubProvider {
        async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse> {
            let prompt = request
                .messages
                .first()
                .and_then(|m| m.content.clone())
                .unwrap_or_default();
            self.log
                .lock()
                .unwrap()
                .push((self.label.clone(), prompt.clone()));
            if self.reply == "<fail>" {
                return Err(AgentError::provider("stub failure"));
            }
            Ok(ProviderResponse {
                content: Some(self.reply.clone()),
                tool_calls: vec![],
                usage: Some(Usage::default()),
                model: request.model,
                reasoning: None,
                finish_reason: Some("stop".into()),
            })
        }
        fn model(&self) -> &str {
            "stub"
        }
        fn name(&self) -> &str {
            "stub"
        }
    }

    fn slot(provider: &str, model: &str) -> MoaSlot {
        MoaSlot {
            provider: provider.into(),
            model: model.into(),
            enabled: true,
            base_url: None,
            api_key: None,
            key_env: None,
        }
    }

    fn moa_config(preset: MoaPreset) -> UlncLawConfig {
        let mut config = UlncLawConfig::default();
        let mut presets = HashMap::new();
        presets.insert("default".to_string(), preset);
        config.moa = MoaConfig {
            default_preset: None,
            presets,
        };
        config
    }

    fn preset(replies: &[(&str, &str)], aggregator: (&str, &str)) -> MoaPreset {
        MoaPreset {
            reference_models: replies
                .iter()
                .map(|(model, _)| slot("stubprov", model))
                .collect(),
            aggregator: slot("stubprov", aggregator.0),
            reference_temperature: None,
            reference_max_tokens: None,
            aggregator_temperature: None,
            degraded_reference_policy: "loud".into(),
        }
    }

    fn factory_for(
        replies: Vec<(&'static str, &'static str)>,
        log: Arc<Mutex<Vec<(String, String)>>>,
    ) -> SlotProviderFactory {
        Arc::new(move |slot: &MoaSlot| {
            let reply = replies
                .iter()
                .find(|(model, _)| *model == slot.model.trim())
                .map(|(_, reply)| *reply)
                .unwrap_or("<fail>");
            Ok(Arc::new(StubProvider {
                label: slot.label(),
                reply: reply.to_string(),
                log: log.clone(),
            }) as Arc<dyn Provider>)
        })
    }

    #[tokio::test]
    async fn fan_out_and_aggregate() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let config = moa_config(preset(
            &[("ref-a", "advice A"), ("ref-b", "advice B")],
            ("agg", "SYNTHESIS"),
        ));
        let factory = factory_for(
            vec![
                ("ref-a", "advice A"),
                ("ref-b", "advice B"),
                ("agg", "SYNTHESIS"),
            ],
            log.clone(),
        );
        let outcome = run_moa_with(None, "test prompt", &config, factory)
            .await
            .unwrap();
        assert_eq!(outcome.synthesis, "SYNTHESIS");
        assert!(outcome.wrapped.starts_with("[Mixture of Agents context"));
        assert!(outcome.wrapped.contains("Aggregator: stubprov:agg"));
        assert_eq!(outcome.references.len(), 2);
        // Aggregator saw both references and the original prompt.
        let agg_call = &log.lock().unwrap()[2];
        assert!(agg_call.1.contains("Reference 1 — stubprov:ref-a"));
        assert!(agg_call.1.contains("Reference 2 — stubprov:ref-b"));
        assert!(agg_call.1.contains("test prompt"));
    }

    #[tokio::test]
    async fn failed_reference_degrades_loudly() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let config = moa_config(preset(
            &[("ref-a", "advice A"), ("ref-b", "<fail>")],
            ("agg", "SYNTH"),
        ));
        let factory = factory_for(
            vec![("ref-a", "advice A"), ("agg", "SYNTH")],
            log.clone(),
        );
        let outcome = run_moa_with(None, "p", &config, factory).await.unwrap();
        let failed = &outcome.references[1];
        assert!(failed.failed());
        let calls = log.lock().unwrap();
        let agg_call = calls.last().expect("aggregator call");
        assert!(agg_call.1.contains("[Reference models unavailable: stubprov:ref-b]"));
        assert_eq!(outcome.synthesis, "SYNTH");
    }

    #[tokio::test]
    async fn silent_policy_hides_failures() {
        let mut preset = preset(&[("ref-a", "advice A"), ("ref-b", "<fail>")], ("agg", "S"));
        preset.degraded_reference_policy = "silent".into();
        let config = moa_config(preset);
        let log = Arc::new(Mutex::new(Vec::new()));
        let factory = factory_for(vec![("ref-a", "advice A"), ("agg", "S")], log.clone());
        run_moa_with(None, "p", &config, factory).await.unwrap();
        let calls = log.lock().unwrap();
        let agg_call = calls.last().expect("aggregator call");
        assert!(!agg_call.1.contains("Reference models unavailable"));
    }

    #[tokio::test]
    async fn all_references_failed_skips_aggregator() {
        let config = moa_config(preset(&[("ref-a", "<fail>")], ("agg", "S")));
        let log = Arc::new(Mutex::new(Vec::new()));
        let factory = factory_for(vec![], log.clone());
        let outcome = run_moa_with(None, "p", &config, factory).await.unwrap();
        assert!(outcome.wrapped.contains("all reference models failed"));
        // Only the reference call happened — no aggregator call.
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn aggregator_failure_falls_back_to_joined() {
        let config = moa_config(preset(&[("ref-a", "advice A")], ("agg", "<fail>")));
        let log = Arc::new(Mutex::new(Vec::new()));
        let factory = factory_for(vec![("ref-a", "advice A")], log.clone());
        let outcome = run_moa_with(None, "p", &config, factory).await.unwrap();
        assert!(outcome.synthesis.contains("Reference 1 — stubprov:ref-a"));
        assert!(outcome.synthesis.contains("advice A"));
    }

    #[tokio::test]
    async fn unknown_preset_lists_available() {
        let config = moa_config(preset(&[("ref-a", "x")], ("agg", "y")));
        let error = run_moa_with(Some("nope"), "p", &config, factory_for(vec![], Arc::new(Mutex::new(Vec::new()))))
            .await
            .err()
            .unwrap();
        let message = error.to_string();
        assert!(message.contains("'nope'"), "{}", message);
        assert!(message.contains("default"), "{}", message);
    }

    #[test]
    fn preset_resolution_prefers_default_preset() {
        let mut config = moa_config(preset(&[("ref-a", "x")], ("agg", "y")));
        config.moa.presets.insert(
            "other".to_string(),
            preset(&[("ref-b", "x")], ("agg", "y")),
        );
        config.moa.default_preset = Some("other".into());
        let (name, preset) = config.moa.resolve_preset(None).unwrap();
        assert_eq!(name, "other");
        assert_eq!(preset.reference_models[0].model, "ref-b");
    }
}
