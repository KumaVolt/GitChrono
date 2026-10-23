//! AI features for GitChrono: provider configuration, prompt building and a
//! small blocking client. Callers run [`complete`] off the UI thread.
//!
//! Bring-your-own-key only for now: requests go straight from the app to the
//! user's provider. The hosted (credits) provider lands with the backend.

mod config;
mod prompts;
mod provider;

pub use config::{AiConfig, Preset, Protocol, PRESETS, delete_key, load_key, save_key};
pub use prompts::{CommitMessage, branch_name, commit_message, parse_commit_message, sanitize_ref_name, stash_message};
pub use provider::{Completion, Request, complete};
