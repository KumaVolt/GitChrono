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
}

impl Global for Prefs {}

pub struct AvatarStore(pub Entity<Avatars>);

impl Global for AvatarStore {}

pub fn init(gravatar: bool, cx: &mut App) {
    cx.set_global(Prefs { gravatar });
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
    use super::url_for_email;

    #[test]
    fn picks_github_for_noreply_and_gravatar_otherwise() {
        assert_eq!(url_for_email("12345+octo@users.noreply.github.com"), "https://avatars.githubusercontent.com/u/12345?s=96");
        assert_eq!(url_for_email("octo@users.noreply.github.com"), "https://github.com/octo.png?size=96");
        let g = url_for_email(" Ada@Example.com ");
        assert!(g.starts_with("https://gravatar.com/avatar/") && g.ends_with("?s=96&d=identicon"));
        assert_eq!(g, url_for_email("ada@example.com"), "trimmed and lowercased before hashing");
    }
}
