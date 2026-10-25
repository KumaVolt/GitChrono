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
    // Lanes in use per row: how wide the graph gets on average, not just at its peak.
    let widths: Vec<usize> = graph.rows.iter().map(|r| r.edges.iter().map(|e| e.from_lane.max(e.to_lane)).max().unwrap_or(0).max(r.lane) + 1).collect();
    let mut sorted = widths.clone();
    sorted.sort_unstable();
    println!(
        "  lanes per row: avg {:.1}, p50 {}, p95 {}",
        widths.iter().sum::<usize>() as f64 / widths.len().max(1) as f64,
        sorted.get(sorted.len() / 2).copied().unwrap_or(0),
        sorted.get(sorted.len() * 95 / 100).copied().unwrap_or(0)
    );
    for c in snap.commits.iter().take(3) {
        println!("  {} {} {:?}", c.short_id, c.summary, c.refs.iter().map(|r| &r.name).collect::<Vec<_>>());
    }
    Ok(())
}
