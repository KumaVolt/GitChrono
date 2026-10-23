//! Generate a commit message for a repo's staged changes with the saved AI config:
//!   cargo run -p gitchrono-ai --example commit_message -- <repo>
//! Needs `gitchrono-core` only as a dev-dependency.

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let repo = gitchrono_core::Repo::open(std::path::Path::new(&path))?;
    let config = gitchrono_ai::AiConfig::load().ok_or_else(|| anyhow::anyhow!("AI is not configured"))?;
    let diff = repo.staged_patch()?;
    let subjects = gitchrono_core::git(repo.root(), &["log", "-30", "--format=%s"]).unwrap_or_default();
    let subjects: Vec<&str> = subjects.lines().collect();
    let request = gitchrono_ai::commit_message(&diff, &subjects, None);
    let completion = gitchrono_ai::complete(&config, config.key().as_deref(), &request)?;
    let message = gitchrono_ai::parse_commit_message(&completion.text);
    println!("{}\n\n{}\n\n[{} in / {} out tokens]", message.summary, message.body, completion.input_tokens, completion.output_tokens);
    Ok(())
}
