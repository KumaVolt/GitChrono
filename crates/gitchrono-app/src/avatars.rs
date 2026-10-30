//! Profile pictures, opt-in (Settings → Profile pictures). Authors' pictures
//! come from Gravatar, which draws a pattern for addresses without one;
//! GitHub no-reply addresses use the GitHub picture. Files are cached on disk
//! for a week and views re-render when one arrives.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use gpui_kit::*;
use sha2::{Digest as _, Sha256};

/// App-wide preferences that views read while rendering.
#[derive(Clone, Copy, Default)]
pub struct Prefs {
    pub gravatar: bool,
    /// What the toolbar's Pull button does; `None` follows git's `pull.rebase`.
    pub pull: Option<crate::projects::PullAction>,
    /// Write the commit message with AI once every change is staged.
    pub auto_commit_message: bool,
}

impl Prefs {
    pub fn of(session: &crate::projects::Session) -> Self {
        Self { gravatar: session.gravatar_enabled(), pull: session.pull_action, auto_commit_message: session.auto_commit_message }
    }
}

impl Global for Prefs {}

pub struct AvatarStore(pub Entity<Avatars>);

impl Global for AvatarStore {}

pub fn init(prefs: Prefs, cx: &mut App) {
    cx.set_global(prefs);
    let store = cx.new(|_| Avatars::default());
    cx.set_global(AvatarStore(store));
}

enum Slot {
    Loading,
    Ready(PathBuf),
    Missing,
}

#[derive(Default)]
pub struct Avatars {
    slots: HashMap<String, Slot>,
}

/// Where an author's picture comes from.
pub fn url_for_email(email: &str) -> String {
    let email = email.trim().to_lowercase();
    if let Some(local) = email.strip_suffix("@users.noreply.github.com") {
        // `12345+login@…` (newer) or `login@…` (older).
        return match local.split_once('+') {
            Some((id, _)) if id.chars().all(|c| c.is_ascii_digit()) => format!("https://avatars.githubusercontent.com/u/{id}?s=96"),
            _ => format!("https://github.com/{}.png?size=96", local.rsplit('+').next().unwrap_or(local)),
        };
    }
    format!("https://gravatar.com/avatar/{}?s=96&d=identicon", hex::encode(Sha256::digest(email.as_bytes())))
}

/// AI coding assistants that sign commits (as author or `Co-authored-by`)
/// show the image their maker publishes on GitHub for that assistant.
/// Matched on the address, or on a name that is clearly the assistant
/// ("Claude", "Claude Opus 5.5") rather than a person called Claude.
pub fn ai_assistant(name: &str, email: &str) -> Option<&'static str> {
    const CLAUDE: &str = "https://avatars.githubusercontent.com/in/1236702?s=96"; // anthropics: Claude
    const OPENAI: &str = "https://avatars.githubusercontent.com/in/1144995?s=96"; // openai: ChatGPT Codex
    const GEMINI: &str = "https://avatars.githubusercontent.com/in/956858?s=96"; // google: Gemini Code Assist
    const COPILOT: &str = "https://avatars.githubusercontent.com/in/1143301?s=96"; // github: Copilot agent
    const XAI: &str = "https://avatars.githubusercontent.com/u/130314967?s=96"; // xai-org (Grok)
    let email = email.trim().to_lowercase();
    let domain = email.rsplit_once('@').map_or("", |(_, d)| d);
    match domain {
        "anthropic.com" => return Some(CLAUDE),
        "openai.com" => return Some(OPENAI),
        "x.ai" => return Some(XAI),
        _ => {}
    }
    if email.ends_with("+copilot@users.noreply.github.com") {
        return Some(COPILOT);
    }
    let name = name.trim().to_lowercase();
    let mut words = name.split_whitespace();
    let first = words.next()?;
    let rest: Vec<&str> = words.collect();
    // "Claude", "Claude Code", "Claude Opus 4.5", "Gemini CLI", "GPT-5 Codex"…
    let qualifiers = ["code", "cli", "opus", "sonnet", "haiku", "fable", "assist", "agent", "assistant", "pro", "flash", "mini", "codex", "ai"];
    let assistant_like = rest.iter().all(|w| qualifiers.contains(w) || w.chars().any(|c| c.is_ascii_digit()));
    match first {
        "claude" if assistant_like => Some(CLAUDE),
        "chatgpt" | "codex" | "openai" => Some(OPENAI),
        f if f.starts_with("gpt-") => Some(OPENAI),
        "gemini" if assistant_like => Some(GEMINI),
        "grok" if assistant_like => Some(XAI),
        "copilot" => Some(COPILOT),
        "github" if rest.first() == Some(&"copilot") => Some(COPILOT),
        _ => None,
    }
}

fn cache_path(url: &str) -> PathBuf {
    let name = hex::encode(Sha256::digest(url.as_bytes()));
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("gitchrono").join("avatars").join(format!("{}.img", &name[..32]))
}

fn fetch(url: &str) -> anyhow::Result<PathBuf> {
    let path = cache_path(url);
    let fresh = std::fs::metadata(&path).and_then(|m| m.modified()).is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() < Duration::from_secs(7 * 86_400));
    if fresh {
        return Ok(path);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
    let bytes = agent.get(url).call()?.body_mut().with_config().limit(2 * 1024 * 1024).read_to_vec()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, bytes)?;
    Ok(path)
}

impl Avatars {
    /// The cached picture at `url`, starting a download the first time.
    pub fn get(&mut self, url: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        match self.slots.get(url) {
            Some(Slot::Ready(path)) => return Some(path.clone()),
            Some(_) => return None,
            None => {}
        }
        self.slots.insert(url.to_string(), Slot::Loading);
        let owned = url.to_string();
        let task = cx.background_spawn(async move { fetch(&owned) });
        let url = url.to_string();
        cx.spawn(async move |this, cx| {
            let slot = match task.await {
                Ok(path) => Slot::Ready(path),
                Err(_) => Slot::Missing,
            };
            this.update(cx, |this, cx| {
                this.slots.insert(url, slot);
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }
}

/// The person's picture: an AI assistant's logo (always), otherwise their
/// Gravatar or GitHub picture if profile pictures are on. `None` until it's
/// downloaded.
pub fn for_person(name: &str, email: &str, cx: &mut App) -> Option<PathBuf> {
    if let Some(url) = ai_assistant(name, email) {
        return for_url(url, cx);
    }
    for_email(email, cx)
}

/// The author's picture if profile pictures are on and it's downloaded.
pub fn for_email(email: &str, cx: &mut App) -> Option<PathBuf> {
    if !cx.global::<Prefs>().gravatar {
        return None;
    }
    let store = cx.global::<AvatarStore>().0.clone();
    store.update(cx, |a, cx| a.get(&url_for_email(email), cx))
}

/// Any picture by URL (e.g. contributors' GitHub avatars), regardless of the
/// profile-picture setting since the URL is already public.
pub fn for_url(url: &str, cx: &mut App) -> Option<PathBuf> {
    let store = cx.global::<AvatarStore>().0.clone();
    store.update(cx, |a, cx| a.get(url, cx))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's own `test` attribute.
    use super::{ai_assistant, url_for_email};

    #[test]
    fn recognises_ai_assistants_but_not_people_named_like_them() {
        let claude = ai_assistant("Claude Opus 5.5", "noreply@anthropic.com");
        assert!(claude.is_some_and(|u| u.contains("in/1236702")));
        assert_eq!(ai_assistant("Claude", "someone@example.com"), claude);
        assert_eq!(ai_assistant("Claude Code", ""), claude);
        assert_eq!(ai_assistant("Claude Monet", "claude@monet.fr"), None);
        assert!(ai_assistant("Codex", "codex@openai.com").is_some_and(|u| u.contains("in/1144995")));
        assert!(ai_assistant("ChatGPT", "").is_some_and(|u| u.contains("in/1144995")));
        assert!(ai_assistant("Gemini CLI", "").is_some_and(|u| u.contains("in/956858")));
        assert_eq!(ai_assistant("Gemini Smith", "g@example.com"), None);
        assert!(ai_assistant("Copilot", "198982749+Copilot@users.noreply.github.com").is_some_and(|u| u.contains("in/1143301")));
        assert!(ai_assistant("Grok 4", "").is_some_and(|u| u.contains("130314967")));
        assert_eq!(ai_assistant("Ada Park", "ada@example.com"), None);
    }

    #[test]
    fn picks_github_for_noreply_and_gravatar_otherwise() {
        assert_eq!(url_for_email("12345+octo@users.noreply.github.com"), "https://avatars.githubusercontent.com/u/12345?s=96");
        assert_eq!(url_for_email("octo@users.noreply.github.com"), "https://github.com/octo.png?size=96");
        let g = url_for_email(" Ada@Example.com ");
        assert!(g.starts_with("https://gravatar.com/avatar/") && g.ends_with("?s=96&d=identicon"));
        assert_eq!(g, url_for_email("ada@example.com"), "trimmed and lowercased before hashing");
    }
}
