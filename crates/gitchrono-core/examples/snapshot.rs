//! `cargo run --release -p gitchrono-core --example snapshot -- <repo> [limit]`
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| ".".into());
    let limit: usize = args.next().and_then(|l| l.parse().ok()).unwrap_or(10_000);
    let t = std::time::Instant::now();
    let repo = gitchrono_core::Repo::open(&path)?;
    let snap = repo.snapshot(limit)?;
    let loaded = t.elapsed();
    let graph = gitchrono_graph::layout(snap.commits.iter().map(|c| (c.id.as_str(), c.parents.iter().map(String::as_str))));
    println!(
        "{}: {} commits, {} local / {} remote branches, {} tags, {} changes | load {:?}, layout {:?}, max lane {}",
        repo.name(), snap.commits.len(), snap.refs.local.len(), snap.refs.remote.len(), snap.refs.tags.len(),
        snap.status.change_count(), loaded, t.elapsed() - loaded, graph.max_lane
    );
    for c in snap.commits.iter().take(3) {
        println!("  {} {} {:?}", c.short_id, c.summary, c.refs.iter().map(|r| &r.name).collect::<Vec<_>>());
    }
    Ok(())
}
