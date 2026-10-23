# GitChrono

A fast, native Git client in the spirit of GitKraken — 100% Rust, GPU-rendered with
[gpui](https://gpui-kit.com).

```bash
cargo run -p gitchrono-app -- /path/to/repo     # or open one from the welcome screen
```

Requires a `git` binary (2.40+) on `PATH`.

## Architecture

```
crates/
  gitchrono-core/   git engine — reads via gitoxide, writes via the git CLI
  gitchrono-graph/  pure commit-graph lane layout (unit tested)
  gitchrono-app/    gpui desktop app (gpui-kit + gpui-component)
```

**Hybrid engine.** History walk (`--topo-order`), refs and commit trees come from
[gitoxide](https://github.com/GitoxideLabs/gitoxide) — that's what keeps 85k-commit repos
fast. Everything that mutates the repo (commit, push, rebase, stash, …) and porcelain-exact
reads (status, patches) shell out to `git`, so hooks, credential helpers, SSH config and
signing behave exactly like the terminal. As gix grows (push/rebase are still missing upstream)
more can move in-process.

Benchmark: `cargo run --release -p gitchrono-core --example snapshot -- <repo>`
(git.git: 85,767 commits loaded in ~2.8s, graph layout ~80ms, Apple Silicon).

**Debug hook.** `GITCHRONO_SELECT=<row>[:diff]` selects a row (and opens its first diff)
after the first load — used for headless screenshots.

## Licensing (open-core)

The crates in this repository are Apache-2.0. Paid Pro features (hosted integrations,
team features, AI) will live in a separate crate outside this repo.

See [docs/parity.md](docs/parity.md) for the GitKraken parity roadmap.
