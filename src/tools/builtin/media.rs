//! Media tools — vision_analyze, image_generate, text_to_speech
//!
//! Ports of hermes' vision/image/tts tools. vision_analyze routes through
//! the configured chat provider; image_generate/text_to_speech use OpenAI
//! REST endpoints and write artifacts under `<home>/images` / `<home>/audio`.

use crate::error::Result;
use crate::tools::{tool, ToolAvailability, ToolContext, ToolRegistry};
use serde_json::json;
use std::sync::Arc;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(vision_analyze_tool());
    registry.register(image_generate_tool());
    registry.register(text_to_speech_tool());
}

fn openai_key() -> Option<String> {
    crate::config::get_env_value("OPENAI_API_KEY")
}

// ---------------------------------------------------------------------------
// vision_analyze
// ---------------------------------------------------------------------------

fn vision_analyze_tool() -> crate::tools::Tool {
    tool("vision_analyze")
        .description(
            "Analyze an image with the vision model. Provide a local file path or URL plus a \
             prompt describing what to extract (OCR, description, diagram reading, screenshot \
             analysis).",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "image": {"type": "string", "description": "Path to a local image file or an http(s) URL"},
                "prompt": {"type": "string", "description": "What to extract or describe (default: detailed description)"}
            },
            "required": ["image"]
        }))
        .handler(|args, ctx| async move {
            let Some(image) = args.get("image").and_then(|v| v.as_str()) else {
                return Ok(json!({"success": false, "error": "vision_analyze: 'image' is required"}));
            };
            let prompt = args
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("Describe this image in detail.")
                .to_string();

            let image_url = if image.starts_with("http://") || image.starts_with("https://") {
                image.to_string()
            } else {
                let path = ctx.resolve_path(image);
                let bytes = match std::fs::read(&path) {
                    Ok(b) => b,
                    Err(e) => return Ok(json!({"success": false, "error": format!("read image: {}", e)})),
                };
                let mime = match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
                    "png" => "image/png",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    _ => "image/jpeg",
                };
                let b64 = base64_engine::STANDARD.encode(&bytes);
                format!("data:{};base64,{}", mime, b64)
            };

            let Some(provider) = ctx.provider.clone() else {
                return Ok(json!({"success": false, "error": "vision_analyze: no provider wired into this run"}));
            };
            // Auxiliary model routing: [auxiliary.vision] override (hermes
            // resolve_vision_provider_client); falls back to the main provider.
            let provider = match crate::provider::auxiliary::resolve_aux_task(
                &ctx.config,
                crate::provider::auxiliary::TASK_VISION,
                provider.clone(),
            ) {
                Ok(aux) => aux.provider,
                Err(e) => {
                    tracing::warn!("auxiliary vision routing failed: {}; using main provider", e);
                    provider
                }
            };
            match provider.analyze_image(&prompt, &image_url).await {
                Ok(answer) => Ok(json!({"success": true, "analysis": answer})),
                Err(e) => Ok(json!({"success": false, "error": format!("vision provider: {}", e)})),
            }
        })
        .toolset("vision")
        .emoji("👁️")
        .build()
        .expect("vision_analyze builds")
}

// ---------------------------------------------------------------------------
// image_generate
// ---------------------------------------------------------------------------

fn check_image_gen() -> ToolAvailability {
    if openai_key().is_some() {
        ToolAvailability::available()
    } else {
        ToolAvailability::unavailable("OPENAI_API_KEY not set (image generation needs an image API)")
    }
}

fn image_generate_tool() -> crate::tools::Tool {
    tool("image_generate")
        .description(
            "Generate an image from a text prompt. Returns the local path of the saved PNG. \
             Use detailed prompts describing subject, style, lighting, and composition.",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "Text description of the image to generate"},
                "size": {"type": "string", "enum": ["1024x1024", "1024x1792", "1792x1024"], "description": "Output size (default 1024x1024)", "default": "1024x1024"},
                "filename": {"type": "string", "description": "Optional output filename (without directory)"}
            },
            "required": ["prompt"]
        }))
        .handler(|args, ctx| async move {
            let Some(prompt) = args.get("prompt").and_then(|v| v.as_str()) else {
                return Ok(json!({"success": false, "error": "image_generate: 'prompt' is required"}));
            };
            let size = args.get("size").and_then(|v| v.as_str()).unwrap_or("1024x1024");
            let Some(api_key) = openai_key() else {
                return Ok(json!({"success": false, "error": "OPENAI_API_KEY not configured"}));
            };
            let base = ctx.config.resolve_base_url();
            let client = reqwest::Client::new();
            let response = client
                .post(format!("{}/images/generations", base.trim_end_matches('/')))
                .bearer_auth(api_key)
                .json(&json!({
                    "model": crate::config::get_env_value("ULNCLAW_IMAGE_MODEL").unwrap_or_else(|| "dall-e-3".into()),
                    "prompt": prompt,
                    "n": 1,
                    "size": size,
                    "response_format": "b64_json",
                }))
                .send()
                .await;
            let response = match response {
                Ok(r) => r,
                Err(e) => return Ok(json!({"success": false, "error": format!("image API: {}", e)})),
            };
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Ok(json!({"success": false, "error": format!("image API {}: {}", status, &body[..body.len().min(300)])}));
            }
            let body: serde_json::Value = match response.json().await {
                Ok(v) => v,
                Err(e) => return Ok(json!({"success": false, "error": format!("parse image response: {}", e)})),
            };
            let Some(b64) = body.pointer("/data/0/b64_json").and_then(|v| v.as_str()) else {
                // Some endpoints return a URL instead.
                if let Some(url) = body.pointer("/data/0/url").and_then(|v| v.as_str()) {
                    return Ok(json!({"success": true, "url": url, "note": "Provider returned a URL; download it if a local file is needed."}));
                }
                return Ok(json!({"success": false, "error": "image API returned no data"}));
            };
            let bytes = match base64_engine::STANDARD.decode(b64) {
                Ok(b) => b,
                Err(e) => return Ok(json!({"success": false, "error": format!("decode image: {}", e)})),
            };
            let dir = ctx.home.join("images");
            std::fs::create_dir_all(&dir).ok();
            let filename = args
                .get("filename")
                .and_then(|v| v.as_str())
                .map(|f| f.replace(['/', '\\'], "_"))
                .unwrap_or_else(|| format!("img-{}.png", &uuid::Uuid::new_v4().to_string()[..8]));
            let path = dir.join(filename);
            if let Err(e) = std::fs::write(&path, &bytes) {
                return Ok(json!({"success": false, "error": format!("save image: {}", e)}));
            }
            Ok(json!({
                "success": true,
                "path": path.display().to_string(),
                "bytes": bytes.len(),
            }))
        })
        .toolset("image_gen")
        .emoji("🎨")
        .check_fn(check_image_gen)
        .build()
        .expect("image_generate builds")
}

// ---------------------------------------------------------------------------
// text_to_speech
// ---------------------------------------------------------------------------

fn check_tts() -> ToolAvailability {
    if openai_key().is_some() || crate::config::get_env_value("ULNCLAW_TTS_ENDPOINT").is_some() {
        ToolAvailability::available()
    } else {
        ToolAvailability::unavailable("no TTS backend configured (set OPENAI_API_KEY or ULNCLAW_TTS_ENDPOINT)")
    }
}

fn text_to_speech_tool() -> crate::tools::Tool {
    tool("text_to_speech")
        .description(
            "Convert text to speech audio. Returns the local path of the saved audio file. \
             Use for voice replies, narration, or audio artifacts.",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "The text to synthesize"},
                "voice": {"type": "string", "description": "Voice name (default: alloy)", "default": "alloy"},
                "filename": {"type": "string", "description": "Optional output filename"}
            },
            "required": ["text"]
        }))
        .handler(|args, ctx| async move {
            let Some(text) = args.get("text").and_then(|v| v.as_str()) else {
                return Ok(json!({"success": false, "error": "text_to_speech: 'text' is required"}));
            };
            let voice = args.get("voice").and_then(|v| v.as_str()).unwrap_or("alloy");

            // Custom endpoint takes precedence (ULNCLAW_TTS_ENDPOINT + ULNCLAW_TTS_KEY).
            if let Some(endpoint) = crate::config::get_env_value("ULNCLAW_TTS_ENDPOINT") {
                let client = reqwest::Client::new();
                let mut request = client.post(&endpoint).json(&json!({"text": text, "voice": voice}));
                if let Some(key) = crate::config::get_env_value("ULNCLAW_TTS_KEY") {
                    request = request.bearer_auth(key);
                }
                let response = match request.send().await {
                    Ok(r) => r,
                    Err(e) => return Ok(json!({"success": false, "error": format!("TTS endpoint: {}", e)})),
                };
                if !response.status().is_success() {
                    return Ok(json!({"success": false, "error": format!("TTS endpoint returned {}", response.status())}));
                }
                let bytes = response.bytes().await.map_err(|e| crate::error::AgentError::tool(e.to_string()))?;
                return save_audio(&ctx, &bytes, args.get("filename").and_then(|v| v.as_str()), "mp3");
            }

            let Some(api_key) = openai_key() else {
                return Ok(json!({"success": false, "error": "no TTS backend configured"}));
            };
            let base = ctx.config.resolve_base_url();
            let client = reqwest::Client::new();
            let response = client
                .post(format!("{}/audio/speech", base.trim_end_matches('/')))
                .bearer_auth(api_key)
                .json(&json!({
                    "model": crate::config::get_env_value("ULNCLAW_TTS_MODEL").unwrap_or_else(|| "tts-1".into()),
                    "input": text,
                    "voice": voice,
                    "response_format": "mp3",
                }))
                .send()
                .await;
            let response = match response {
                Ok(r) => r,
                Err(e) => return Ok(json!({"success": false, "error": format!("TTS API: {}", e)})),
            };
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Ok(json!({"success": false, "error": format!("TTS API {}: {}", status, &body[..body.len().min(300)])}));
            }
            let bytes = response.bytes().await.map_err(|e| crate::error::AgentError::tool(e.to_string()))?;
            save_audio(&ctx, &bytes, args.get("filename").and_then(|v| v.as_str()), "mp3")
        })
        .toolset("tts")
        .emoji("🔊")
        .check_fn(check_tts)
        .build()
        .expect("text_to_speech builds")
}

fn save_audio(
    ctx: &Arc<ToolContext>,
    bytes: &[u8],
    filename: Option<&str>,
    ext: &str,
) -> Result<serde_json::Value> {
    let dir = ctx.home.join("audio");
    std::fs::create_dir_all(&dir).ok();
    let filename = filename
        .map(|f| f.replace(['/', '\\'], "_"))
        .unwrap_or_else(|| format!("tts-{}.{}", &uuid::Uuid::new_v4().to_string()[..8], ext));
    let filename = if filename.contains('.') {
        filename
    } else {
        format!("{}.{}", filename, ext)
    };
    let path = dir.join(filename);
    std::fs::write(&path, bytes)
        .map_err(|e| crate::error::AgentError::tool(format!("save audio: {}", e)))?;
    Ok(json!({
        "success": true,
        "path": path.display().to_string(),
        "bytes": bytes.len(),
    }))
}

/// base64 engine alias (uses reqwest's re-exported base64 when available;
/// falls back to a tiny local implementation).
mod base64_engine {
    pub struct Standard;
    pub const STANDARD: Standard = Standard;

    impl Standard {
        pub fn encode(&self, input: &[u8]) -> String {
            const ALPHABET: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
            for chunk in input.chunks(3) {
                let b0 = chunk[0] as u32;
                let b1 = *chunk.get(1).unwrap_or(&0) as u32;
                let b2 = *chunk.get(2).unwrap_or(&0) as u32;
                let triple = (b0 << 16) | (b1 << 8) | b2;
                out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
                out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
                out.push(if chunk.len() > 1 {
                    ALPHABET[((triple >> 6) & 0x3F) as usize] as char
                } else {
                    '='
                });
                out.push(if chunk.len() > 2 {
                    ALPHABET[(triple & 0x3F) as usize] as char
                } else {
                    '='
                });
            }
            out
        }

        pub fn decode(&self, input: &str) -> std::result::Result<Vec<u8>, String> {
            let mut output = Vec::with_capacity(input.len() / 4 * 3);
            let mut buffer: u32 = 0;
            let mut bits = 0;
            for ch in input.chars() {
                if ch == '=' || ch.is_whitespace() {
                    continue;
                }
                let value = match ch {
                    'A'..='Z' => ch as u32 - 'A' as u32,
                    'a'..='z' => ch as u32 - 'a' as u32 + 26,
                    '0'..='9' => ch as u32 - '0' as u32 + 52,
                    '+' => 62,
                    '/' => 63,
                    other => return Err(format!("invalid base64 char: {}", other)),
                };
                buffer = (buffer << 6) | value;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    output.push(((buffer >> bits) & 0xFF) as u8);
                }
            }
            Ok(output)
        }
    }

    #[allow(dead_code)]
    pub trait Engine {
        fn encode(&self, input: &[u8]) -> String;
        fn decode(&self, input: &str) -> std::result::Result<Vec<u8>, String>;
    }

    impl Engine for Standard {
        fn encode(&self, input: &[u8]) -> String {
            Standard::encode(self, input)
        }
        fn decode(&self, input: &str) -> std::result::Result<Vec<u8>, String> {
            Standard::decode(self, input)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::base64_engine::STANDARD;

    #[test]
    fn test_base64_roundtrip() {
        let data = b"hello ulnclaw \xF0\x9F\xA6\x80";
        let encoded = STANDARD.encode(data);
        let decoded = STANDARD.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }
}
