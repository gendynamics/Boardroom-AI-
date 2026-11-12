//! Minimal client for OpenAI-compatible Chat Completions with function calling.
//!
//! xAI (Grok) and OpenAI both accept this request shape, so one client serves either.
//! `AI_BASE_URL` can point at any other compatible endpoint (the tests use a local mock).

use std::{env, time::Duration};

use reqwest::{Client, Url};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiConfig {
    pub provider: &'static str,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl AiConfig {
    /// A human-readable provider and model, without the key.
    pub fn label(&self) -> String {
        let provider = match self.provider {
            "xai" => "xAI Grok",
            "openai" => "OpenAI",
            _ => "OpenAI-compatible endpoint",
        };
        format!("{provider} · {}", self.model)
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn valid_model(model: &str) -> bool {
    (1..=80).contains(&model.len())
        && model
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'_' | b'-' | b':' | b'/'))
}

fn model_from_env(name: &str, default: &str) -> Result<String, String> {
    let model = non_empty_env(name).unwrap_or_else(|| default.to_owned());
    if valid_model(&model) {
        Ok(model)
    } else {
        Err(format!("{name} must be a model ID such as {default}."))
    }
}

/// Accepts an https endpoint, or plain http only for a local mock/proxy.
pub fn validate_base_url(value: &str) -> Result<String, String> {
    let error = || {
        "AI_BASE_URL must be an https URL (or http://localhost / http://127.0.0.1) without credentials, query, or fragment.".to_owned()
    };
    let url = Url::parse(value).map_err(|_| error())?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1"));
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && local);
    if !scheme_ok
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error());
    }
    Ok(value.trim_end_matches('/').to_owned())
}

/// Reads the AI provider from the environment. Returns `Ok(None)` when no key is configured.
///
/// Precedence: `XAI_API_KEY` (Grok), then `OPENAI_API_KEY`. `AI_BASE_URL` overrides the endpoint.
pub fn config_from_env() -> Result<Option<AiConfig>, String> {
    let (provider, api_key, default_base, model) = if let Some(key) = non_empty_env("XAI_API_KEY") {
        (
            "xai",
            key,
            "https://api.x.ai/v1",
            model_from_env("XAI_MODEL", "grok-4.7")?,
        )
    } else if let Some(key) = non_empty_env("OPENAI_API_KEY") {
        (
            "openai",
            key,
            "https://api.openai.com/v1",
            model_from_env("OPENAI_MODEL", "gpt-4.1-mini")?,
        )
    } else {
        return Ok(None);
    };
    let (provider, base_url) = match non_empty_env("AI_BASE_URL") {
        Some(base) => ("custom", validate_base_url(&base)?),
        None => (provider, default_base.to_owned()),
    };
    Ok(Some(AiConfig {
        provider,
        base_url,
        api_key,
        model,
    }))
}

/// Sends one Chat Completions request and returns the assistant `message` object.
///
/// Errors are short, user-safe strings; details go to the server log.
pub async fn chat(
    client: &Client,
    config: &AiConfig,
    messages: &[Value],
    tools: &[Value],
) -> Result<Value, &'static str> {
    let mut body = json!({ "model": config.model, "messages": messages });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
        body["tool_choice"] = json!("auto");
    }
    let response = client
        .post(format!("{}/chat/completions", config.base_url))
        .timeout(Duration::from_secs(90))
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| {
            eprintln!("AI request failed: {error}");
            "Could not reach the AI provider."
        })?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        let excerpt: String = text.chars().take(500).collect();
        eprintln!("AI provider returned HTTP {status}: {excerpt}");
        return Err(match status.as_u16() {
            401 | 403 => {
                "The AI provider rejected the API key. Check XAI_API_KEY or OPENAI_API_KEY."
            }
            404 => "The AI provider did not recognize the model. Check XAI_MODEL or OPENAI_MODEL.",
            429 => "The AI provider is rate-limiting requests. Wait a moment and try again.",
            _ => "The AI provider returned an error. Check the server log.",
        });
    }
    let payload: Value = response.json().await.map_err(|error| {
        eprintln!("Could not parse the AI response: {error}");
        "The AI provider returned an unreadable response."
    })?;
    let message = &payload["choices"][0]["message"];
    if !message.is_object() {
        eprintln!("AI response had no choices[0].message.");
        return Err("The AI provider returned an unusable response.");
    }
    Ok(message.clone())
}

#[cfg(test)]
mod tests {
    use super::{AiConfig, valid_model, validate_base_url};

    #[test]
    fn base_url_must_be_https_or_local_http() {
        assert_eq!(
            validate_base_url("https://api.x.ai/v1/").as_deref(),
            Ok("https://api.x.ai/v1")
        );
        assert!(validate_base_url("http://127.0.0.1:4010/v1").is_ok());
        assert!(validate_base_url("http://localhost:4010").is_ok());
        assert!(validate_base_url("http://api.example.com/v1").is_err());
        assert!(validate_base_url("https://user:pw@api.example.com/v1").is_err());
        assert!(validate_base_url("https://api.example.com/v1?key=1").is_err());
        assert!(validate_base_url("not a url").is_err());
    }

    #[test]
    fn model_ids_are_restricted_to_safe_characters() {
        assert!(valid_model("grok-4.7"));
        assert!(valid_model("gpt-4.1-mini"));
        assert!(valid_model("org/model:latest"));
        assert!(!valid_model(""));
        assert!(!valid_model("model with spaces"));
        assert!(!valid_model("model\"injection"));
    }

    #[test]
    fn label_never_includes_the_key() {
        let config = AiConfig {
            provider: "xai",
            base_url: "https://api.x.ai/v1".to_owned(),
            api_key: "secret-key".to_owned(),
            model: "grok-4.7".to_owned(),
        };
        assert_eq!(config.label(), "xAI Grok · grok-4.7");
        assert!(!config.label().contains("secret"));
    }
}
