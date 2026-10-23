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
| Command palette (⌘P) / keyboard navigation | ⬜ | |
| Search commits (message, author, sha) | ⬜ | |
| Tab per repo, recent repos | ⬜ | |
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
| Undo / redo of operations (reflog) | ⬜ |
| Submodules, LFS, worktrees | ⬜ |
| Commit signing indicators | ⬜ |
| Graph compaction for wide histories | ⬜ |
| Packaging: signed .dmg, Windows, Linux, auto-update | ⬜ |

## Tier 3 — Parity
| Feature | Status |
|---|---|
| Launchpad: PRs/issues across GitHub/GitLab/Bitbucket/Azure | 💎 |
| Hosting integrations (OAuth, create PR, clone from) | ⬜ / 💎 |
| Workspaces (multi-repo) | 💎 |
| Profiles | ⬜ |
| AI commit messages / explain commit | 💎 |
| Release notes from tag range → gitchrono.com changelog | 💎 |
| Cloud patches | 💎 |
