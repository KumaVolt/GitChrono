# GitKraken parity map

Legend: ✅ done · 🟡 partial · ⬜ todo · 💎 Pro (closed)

## Tier 1 — MVP
| Feature | Status | Notes |
|---|---|---|
| Commit graph (all refs, topo order, lanes, merges) | ✅ | gix topo walk + `gitchrono-graph` |
| Ref badges (branch / remote / tag / stash / HEAD) | ✅ | |
| WIP node + staging panel (file-level stage/unstage) | ✅ | |
| Commit / amend | ✅ | |
| Commit detail + changed files | ✅ | gix tree diff with rename tracking |
| Diff viewer (unified) | ✅ | |
| Sidebar: local / remote / tags / stashes | ✅ | double-click to checkout |
| Fetch / pull / push (+ upstream, ahead/behind) | ✅ | |
| Stash push / pop / apply / drop | ✅ | |
| Checkout, branch here, cherry-pick, revert, reset | ✅ | buttons in detail panel |
| Merge/rebase/cherry-pick in progress banner (continue/abort) | ✅ | |
| Hunk- and line-level staging / unstaging | ✅ | `partial_patch` + `git apply --cached --recount`, integration tested |
| Discard changes (file / hunk / lines) | ✅ | with confirmation |
| Right-click context menus (commit, branch, remote, tag, stash, file) | ✅ | not yet click-tested |
| Create/rename/delete branch & tag dialogs | ✅ | prompt overlay |
| Auto-refresh on file changes (fs watcher) | ✅ | status-only vs full reload |
| Command palette (⌘P / ⌘K) | ✅ | actions, branches, tabs, repos; fuzzy |
| Search commits (message, author, sha) | ✅ | ⌘F, dims non-matches, Enter / ⇧Enter |
| Tab per repo, recent repos, restore on launch | ✅ | `shell.rs`; ⌘T / ⌘W / ctrl-tab |
| Project folders (parent folder → all repos inside) | ✅ | home tab, scan depth 4, "Open all" |
| Embedded terminal (per repo, multiple tabs) | ✅ | `alacritty_terminal` PTY, ⌥T; no mouse selection/copy yet |
| Version-aware tag sorting | ✅ | |
| Lazy paging past 50k commits | ⬜ | |

## Tier 2 — Solid
| Feature | Status |
|---|---|
| 3-way merge conflict editor | ⬜ (gpui-component has a diff/conflict widget) |
| Drag-and-drop branch → merge/rebase | ⬜ |
| Interactive rebase (reorder/squash/edit) | ⬜ |
| File history + blame | ⬜ (gix-blame) |
| Side-by-side diff, syntax highlighting | ⬜ (tree-sitter feature) |
| Undo / redo of operations | ✅ ref snapshots before/after every op; soft undo for commits, `reset --keep` otherwise |
| Submodules, LFS, worktrees | 🟡 worktrees ✅ (sidebar, create from branch/new, open in tab, remove); submodules & LFS todo |
| Commit signing indicators | ⬜ |
| Graph compaction for wide histories | ⬜ |
| Packaging: signed .dmg, Windows, Linux, auto-update | ⬜ |

## Tier 3 — Parity
| Feature | Status |
|---|---|
| Launchpad: PRs/issues across GitHub/GitLab/Bitbucket/Azure | 💎 |
| Hosting integrations (OAuth, create PR, clone from) | ⬜ / 💎 |
| Workspaces (multi-repo) | 🟡 project folders; multi-repo fetch/pull still todo |
| Profiles | ⬜ |
| AI commit messages / explain commit | 🟡 commit message, branch & stash names (BYOK) |
| Release notes from tag range → gitchrono.com changelog | 💎 |
| Cloud patches | 💎 |

## Gaps found in GitKraken Desktop 12.x (researched 2026-10-23)

Ordered by how much a daily user misses them.

| Feature | GitKraken | Status |
|---|---|---|
| Command palette | ⌘P: run any action, "Open in" detected IDEs | ✅ (no "Open in" yet) |
| Search / filter graph | message, author, sha, file; highlight matches | ✅ (not by file yet) |
| Left-panel filter | filter branches/remotes/tags as you type | ✅ |
| Terminal: selection + copy, links, drag files in | xterm.js features | 🟡 selection (drag, word, line, ⇧-extend), ⌘C, ⌘A; no links/drag yet |
| Worktrees | create/open/remove, WIP row per worktree, tab per worktree | ✅ (no WIP row per worktree yet) |
| Undo / redo | reset, rebase, checkout, commit via reflog | ✅ |
| File history + blame | history follows renames | ⬜ |
| Split / inline / hunk diff views, syntax highlighting | | ⬜ |
| Merge conflict tool | 3-way, AI-assisted resolution | ⬜ |
| Interactive rebase editor | reorder, squash, reword, drop | ⬜ |
| Drag a branch onto another | merge / rebase / PR | ⬜ |
| Select several commits | combined diff, patch, cherry-pick range | ⬜ |
| Hide / solo / pin branches | | ⬜ |
| Tab aliases, tab reordering | | ⬜ |
| Clone / init dialogs | including from GitHub/GitLab | ⬜ |
| Submodules and LFS | | ⬜ |
| Commit signing (GPG/SSH) and indicators | | ⬜ |
| Zoom levels / font size | 140–300% | ⬜ |
| Integrations and Launchpad | PRs/issues across hosts, Jira | ⬜ / 💎 |
| AI: explain commit, compose commits, conflict resolution | | ⬜ / 💎 |
| Agent sessions | status of Claude Code / Codex in worktrees | ⬜ (later) |
