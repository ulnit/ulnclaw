//! Text-to-speech pipeline — lean port of hermes `tts:` config +
//! `/api/audio/speak` @ v2026.8.3.
//!
//! Hermes ships a wide provider chain (edge, openai, elevenlabs,
//! minimax, xai, mistral, gemini, local piper/neutts/kittentts). The
//! static Rust binary ports the two API-backed providers that need no
//! local models or websocket protocols: **openai** (`/v1/audio/speech`,
//! OPENAI_API_KEY) and **elevenlabs** (`/v1/text-to-speech/<voice>`,
//! ELEVENLABS_API_KEY). The free `edge` provider is a Microsoft
//! websocket protocol and stays out of scope (documented in the parity
//! matrix).

use serde::{Deserialize, Serialize};

fn default_provider() -> String {
    "openai".into()
}
fn default_openai_model() -> String {
    "gpt-4o-mini-tts".into()
}
fn default_openai_voice() -> String {
    "alloy".into()
}
fn default_elevenlabs_voice_id() -> String {
    // hermes config_defaults: Adam.
    "pNInz6obpgDQGcFmaJgB".into()
}
fn default_elevenlabs_model_id() -> String {
    "eleven_multilingual_v2".into()
}

/// `[tts.openai]` (hermes tts.openai).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsOpenaiConfig {
    pub model: String,
    pub voice: String,
}

impl Default for TtsOpenaiConfig {
    fn default() -> Self {
        Self {
            model: default_openai_model(),
            voice: default_openai_voice(),
        }
    }
}

/// `[tts.elevenlabs]` (hermes tts.elevenlabs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsElevenlabsConfig {
    pub voice_id: String,
    pub model_id: String,
}

impl Default for TtsElevenlabsConfig {
    fn default() -> Self {
        Self {
            voice_id: default_elevenlabs_voice_id(),
            model_id: default_elevenlabs_model_id(),
        }
    }
}

/// `[tts]` config block (hermes `tts:`). Provider choice: `openai` or
/// `elevenlabs` (edge/local providers out of scope — see module docs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsConfig {
    pub provider: String,
    pub openai: TtsOpenaiConfig,
    pub elevenlabs: TtsElevenlabsConfig,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            openai: TtsOpenaiConfig::default(),
            elevenlabs: TtsElevenlabsConfig::default(),
        }
    }
}

/// Synthesized audio ready for a data URL.
#[derive(Debug)]
pub struct TtsOutput {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub provider: String,
}

/// Synthesize `text` with the configured provider.
pub async fn synthesize(tts: &TtsConfig, text: &str) -> Result<TtsOutput, String> {
    match tts.provider.as_str() {
        "openai" => synthesize_openai(&tts.openai, text).await,
        "elevenlabs" => synthesize_elevenlabs(&tts.elevenlabs, text).await,
        other => Err(format!(
            "unsupported tts provider '{other}' (expected openai or elevenlabs)"
        )),
    }
}

async fn synthesize_openai(config: &TtsOpenaiConfig, text: &str) -> Result<TtsOutput, String> {
    // Custom-endpoint escape hatch shared with the agent-side
    // `text_to_speech` tool: ULNCLAW_TTS_ENDPOINT (+ optional
    // ULNCLAW_TTS_KEY) wins before OPENAI_API_KEY.
    if let Some(endpoint) = crate::config::get_env_value("ULNCLAW_TTS_ENDPOINT") {
        let mut request = reqwest::Client::new()
            .post(&endpoint)
            .timeout(std::time::Duration::from_secs(60))
            .json(&serde_json::json!({ "text": text, "voice": config.voice }));
        if let Some(key) = crate::config::get_env_value("ULNCLAW_TTS_KEY") {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("custom tts endpoint request failed: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "custom tts endpoint failed ({}): {}",
                status,
                truncate_for_error(&body)
            ));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("custom tts endpoint body failed: {e}"))?
            .to_vec();
        return Ok(TtsOutput {
            bytes,
            mime: "audio/mpeg",
            provider: "openai".into(),
        });
    }
    let Some(key) = crate::config::get_env_value("OPENAI_API_KEY") else {
        return Err("OPENAI_API_KEY not set".into());
    };
    let base = crate::config::get_env_value("OPENAI_BASE_URL")
        .unwrap_or_else(|| "https://api.openai.com/v1".into());
    let url = format!("{}/audio/speech", base.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .post(&url)
        .timeout(std::time::Duration::from_secs(60))
        .bearer_auth(&key)
        .json(&serde_json::json!({
            "model": config.model,
            "voice": config.voice,
            "input": text,
            "response_format": "mp3",
        }))
        .send()
        .await
        .map_err(|e| format!("openai tts request failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "openai tts failed ({}): {}",
            status,
            truncate_for_error(&body)
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("openai tts body failed: {e}"))?
        .to_vec();
    Ok(TtsOutput {
        bytes,
        mime: "audio/mpeg",
        provider: "openai".into(),
    })
}

async fn synthesize_elevenlabs(
    config: &TtsElevenlabsConfig,
    text: &str,
) -> Result<TtsOutput, String> {
    let Some(key) = crate::config::get_env_value("ELEVENLABS_API_KEY") else {
        return Err("ELEVENLABS_API_KEY not set".into());
    };
    let url = format!(
        "https://api.elevenlabs.io/v1/text-to-speech/{}",
        urlencoding(&config.voice_id)
    );
    let response = reqwest::Client::new()
        .post(&url)
        .timeout(std::time::Duration::from_secs(60))
        .header("xi-api-key", &key)
        .json(&serde_json::json!({
            "text": text,
            "model_id": config.model_id,
        }))
        .send()
        .await
        .map_err(|e| format!("elevenlabs tts request failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "elevenlabs tts failed ({}): {}",
            status,
            truncate_for_error(&body)
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("elevenlabs tts body failed: {e}"))
        .map(|b| b.to_vec())?;
    Ok(TtsOutput {
        bytes,
        mime: "audio/mpeg",
        provider: "elevenlabs".into(),
    })
}

/// Voice-list failure modes (401/403 answer `available: false` +
/// `error: "unauthorized"` per hermes; anything else is a 502).
pub enum VoicesError {
    Unauthorized,
    Other(String),
}

/// `GET https://api.elevenlabs.io/v1/voices` — non-secret voice
/// metadata for the desktop voice picker (hermes
/// `/api/audio/elevenlabs/voices` parity).
pub async fn elevenlabs_voices(api_key: &str) -> Result<Vec<serde_json::Value>, VoicesError> {
    let response = reqwest::Client::new()
        .get("https://api.elevenlabs.io/v1/voices")
        .timeout(std::time::Duration::from_secs(10))
        .header("Accept", "application/json")
        .header("xi-api-key", api_key)
        .send()
        .await
        .map_err(|e| VoicesError::Other(format!("voices request failed: {e}")))?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(VoicesError::Unauthorized);
    }
    if !status.is_success() {
        return Err(VoicesError::Other(format!("voices failed ({status})")));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|e| VoicesError::Other(format!("voices body failed: {e}")))?;
    let mut voices: Vec<serde_json::Value> = Vec::new();
    for voice in payload["voices"].as_array().cloned().unwrap_or_default() {
        let voice_id = voice["voice_id"].as_str().unwrap_or_default().trim().to_string();
        if voice_id.is_empty() {
            continue;
        }
        let name = voice["name"]
            .as_str()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(&voice_id)
            .to_string();
        let category = voice["category"].as_str().unwrap_or_default().trim().to_string();
        let label = if category.is_empty() {
            name.clone()
        } else {
            format!("{name} ({category})")
        };
        voices.push(serde_json::json!({
            "voice_id": voice_id,
            "name": name,
            "label": label,
        }));
    }
    voices.sort_by(|a, b| {
        a["label"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase()
            .cmp(&b["label"].as_str().unwrap_or_default().to_lowercase())
    });
    Ok(voices)
}

/// Percent-encode a voice id for a URL path segment (alnum + `-_.~`
/// pass through — enough for ElevenLabs ids).
fn urlencoding(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn truncate_for_error(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.len() <= 300 {
        trimmed.to_string()
    } else {
        format!("{}…", &trimmed[..297])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tts_config_defaults_match_hermes() {
        let config = TtsConfig::default();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.openai.model, "gpt-4o-mini-tts");
        assert_eq!(config.openai.voice, "alloy");
        assert_eq!(config.elevenlabs.voice_id, "pNInz6obpgDQGcFmaJgB");
        assert_eq!(config.elevenlabs.model_id, "eleven_multilingual_v2");
    }

    #[test]
    fn tts_config_parses_toml_overrides() {
        let parsed: TtsConfig = toml::from_str(
            "provider = \"elevenlabs\"\n\n[elevenlabs]\nvoice_id = \"abc\"\nmodel_id = \"eleven_turbo_v2\"\n",
        )
        .unwrap();
        assert_eq!(parsed.provider, "elevenlabs");
        assert_eq!(parsed.elevenlabs.voice_id, "abc");
        assert_eq!(parsed.elevenlabs.model_id, "eleven_turbo_v2");
        // Untouched sections keep hermes defaults.
        assert_eq!(parsed.openai.voice, "alloy");
    }

    #[tokio::test]
    async fn synthesize_rejects_unknown_provider() {
        let mut config = TtsConfig::default();
        config.provider = "edge".into();
        let error = synthesize(&config, "hello").await.unwrap_err();
        assert!(error.contains("unsupported tts provider"), "{error}");
    }

    #[tokio::test]
    async fn synthesize_openai_without_key_fails_clean() {
        let saved = std::env::var("OPENAI_API_KEY").ok();
        std::env::remove_var("OPENAI_API_KEY");
        // The home .env under a temp dir is empty too.
        let dir = tempfile::tempdir().expect("tempdir");
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());
        let config = TtsConfig::default();
        let error = synthesize(&config, "hello").await.unwrap_err();
        assert!(error.contains("OPENAI_API_KEY"), "{error}");
        match saved {
            Some(v) => std::env::set_var("OPENAI_API_KEY", v),
            None => {}
        }
        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[test]
    fn urlencoding_passes_elevenlabs_ids() {
        assert_eq!(urlencoding("pNInz6obpgDQGcFmaJgB"), "pNInz6obpgDQGcFmaJgB");
        assert_eq!(urlencoding("a b/c"), "a%20b%2Fc");
    }
}
