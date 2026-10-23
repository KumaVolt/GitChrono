//! One-shot chat completions over the two wire protocols providers speak.

use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::config::{AiConfig, Protocol};

pub struct Request<'a> {
    pub system: &'a str,
    pub user: String,
    pub max_tokens: u32,
}

#[derive(Debug, Default)]
pub struct Completion {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Blocking: call from a background thread.
pub fn complete(config: &AiConfig, key: Option<&str>, request: &Request) -> Result<Completion> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(90)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let base = config.base_url.trim_end_matches('/');
    let (url, body) = match config.protocol {
        Protocol::OpenAi => (
            format!("{base}/chat/completions"),
            json!({
                "model": config.model,
                "max_tokens": request.max_tokens,
                "temperature": 0.2,
                "messages": [
                    {"role": "system", "content": request.system},
                    {"role": "user", "content": request.user},
                ],
            }),
        ),
        Protocol::Anthropic => (
            format!("{base}/messages"),
            json!({
                "model": config.model,
                "max_tokens": request.max_tokens,
                "temperature": 0.2,
                "system": request.system,
                "messages": [{"role": "user", "content": request.user}],
            }),
        ),
    };
    let mut req = agent.post(&url).header("Content-Type", "application/json");
    if let Some(key) = key {
        req = match config.protocol {
            Protocol::OpenAi => req.header("Authorization", &format!("Bearer {key}")),
            Protocol::Anthropic => req.header("x-api-key", key).header("anthropic-version", "2023-06-01"),
        };
    }
    let mut response = req.send_json(&body)?;
    let status = response.status();
    let text = response.body_mut().read_to_string()?;
    if !status.is_success() {
        bail!("{} returned {}: {}", config.base_url, status.as_u16(), error_message(&text));
    }
    let value: Value = serde_json::from_str(&text)?;
    parse_response(config.protocol, &value)
}

fn error_message(body: &str) -> String {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let message = value["error"]["message"].as_str().or(value["message"].as_str()).or(value["error"].as_str());
    match message {
        Some(m) => m.to_string(),
        None => body.chars().take(300).collect(),
    }
}

fn parse_response(protocol: Protocol, value: &Value) -> Result<Completion> {
    let (text, input, output) = match protocol {
        Protocol::OpenAi => (
            value["choices"][0]["message"]["content"].as_str().map(str::to_string),
            &value["usage"]["prompt_tokens"],
            &value["usage"]["completion_tokens"],
        ),
        Protocol::Anthropic => (
            value["content"].as_array().map(|blocks| {
                blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("")
            }),
            &value["usage"]["input_tokens"],
            &value["usage"]["output_tokens"],
        ),
    };
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        bail!("the model returned no text");
    };
    Ok(Completion { text, input_tokens: input.as_u64().unwrap_or(0), output_tokens: output.as_u64().unwrap_or(0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_protocols() {
        let openai = json!({"choices": [{"message": {"content": "feat: x"}}], "usage": {"prompt_tokens": 10, "completion_tokens": 3}});
        let c = parse_response(Protocol::OpenAi, &openai).unwrap();
        assert_eq!((c.text.as_str(), c.input_tokens, c.output_tokens), ("feat: x", 10, 3));

        let anthropic = json!({"content": [{"type": "text", "text": "fix: y"}], "usage": {"input_tokens": 7, "output_tokens": 2}});
        let c = parse_response(Protocol::Anthropic, &anthropic).unwrap();
        assert_eq!((c.text.as_str(), c.input_tokens, c.output_tokens), ("fix: y", 7, 2));

        assert!(parse_response(Protocol::OpenAi, &json!({"choices": []})).is_err());
    }

    #[test]
    fn extracts_error_messages() {
        assert_eq!(error_message(r#"{"error":{"message":"bad key"}}"#), "bad key");
        assert_eq!(error_message(r#"{"message":"Unauthorized"}"#), "Unauthorized");
        assert_eq!(error_message("plain"), "plain");
    }
}
