//! Provider settings (a JSON file in the config dir) and API keys (the OS keychain).

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

const KEYCHAIN_SERVICE: &str = "com.gitchrono.app";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// `POST {base}/chat/completions`: OpenAI, Mistral, Gemini, OpenRouter, Ollama, DeepSeek, Groq…
    OpenAi,
    /// `POST {base}/messages`.
    Anthropic,
}

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub protocol: Protocol,
    pub base_url: &'static str,
    pub model: &'static str,
    pub needs_key: bool,
    /// Checked when no key is stored in the keychain.
    pub env_key: &'static str,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "mistral",
        name: "Mistral",
        protocol: Protocol::OpenAi,
        base_url: "https://api.mistral.ai/v1",
        model: "mistral-small-latest",
        needs_key: true,
        env_key: "MISTRAL_API_KEY",
    },
    Preset {
        id: "openai",
        name: "OpenAI",
        protocol: Protocol::OpenAi,
        base_url: "https://api.openai.com/v1",
        model: "gpt-5-mini",
        needs_key: true,
        env_key: "OPENAI_API_KEY",
    },
    Preset {
        id: "anthropic",
        name: "Anthropic",
        protocol: Protocol::Anthropic,
        base_url: "https://api.anthropic.com/v1",
        model: "claude-haiku-5-5",
        needs_key: true,
        env_key: "ANTHROPIC_API_KEY",
    },
    Preset {
        id: "gemini",
        name: "Gemini",
        protocol: Protocol::OpenAi,
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        model: "gemini-flash-lite-latest",
        needs_key: true,
        env_key: "GEMINI_API_KEY",
    },
    Preset {
        id: "openrouter",
        name: "OpenRouter",
        protocol: Protocol::OpenAi,
        base_url: "https://openrouter.ai/api/v1",
        model: "mistralai/mistral-small-3.2-24b-instruct",
        needs_key: true,
        env_key: "OPENROUTER_API_KEY",
    },
    Preset {
        id: "ollama",
        name: "Ollama (local)",
        protocol: Protocol::OpenAi,
        base_url: "http://localhost:11434/v1",
        model: "llama3.2",
        needs_key: false,
        env_key: "",
    },
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConfig {
    /// A [`Preset::id`].
    pub provider: String,
    pub protocol: Protocol,
    pub base_url: String,
    pub model: String,
    /// Master switch; also off until the user has configured a provider.
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl AiConfig {
    pub fn from_preset(preset: &Preset) -> Self {
        Self {
            provider: preset.id.into(),
            protocol: preset.protocol,
            base_url: preset.base_url.into(),
            model: preset.model.into(),
            enabled: true,
        }
    }

    pub fn preset(&self) -> Option<&'static Preset> {
        PRESETS.iter().find(|p| p.id == self.provider)
    }

    pub fn needs_key(&self) -> bool {
        self.preset().is_none_or(|p| p.needs_key)
    }

    fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("gitchrono").join("ai.json"))
    }

    /// The saved configuration, or `None` when AI was never set up.
    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()?).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("no config directory")?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// The API key: keychain first, then the preset's environment variable.
    pub fn key(&self) -> Option<String> {
        load_key(&self.provider).or_else(|| {
            let var = self.preset()?.env_key;
            std::env::var(var).ok().filter(|k| !var.is_empty() && !k.is_empty())
        })
    }
}

fn entry(provider: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYCHAIN_SERVICE, &format!("ai-{provider}"))?)
}

pub fn load_key(provider: &str) -> Option<String> {
    entry(provider).ok()?.get_password().ok().filter(|k| !k.is_empty())
}

pub fn save_key(provider: &str, key: &str) -> Result<()> {
    entry(provider)?.set_password(key).context("saving the API key to the keychain")
}

pub fn delete_key(provider: &str) -> Result<()> {
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
