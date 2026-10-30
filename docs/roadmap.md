# Roadmap

Legend: ✅ done · 🟡 partial · ⬜ todo · 💎 Pro (closed)

## Tier 1 — MVP
| Feature | Status | Notes |
|---|---|---|
| Commit graph (all refs, date order, lanes, merges) | ✅ | gix walk in `--date-order` (like GitKraken) + `gitchrono-graph` |
| Ref badges (branch / remote / tag / stash / HEAD) | ✅ | |
| WIP node + staging panel (file-level stage/unstage) | ✅ | WIP row joined to HEAD, with change counts |
| Commit / amend | ✅ | |
| Commit detail + changed files | ✅ | gix tree diff with rename tracking |
| Diff viewer (unified) | ✅ | |
| Sidebar: local / remote / tags / stashes | ✅ | double-click to checkout |
| Fetch / pull / push (+ upstream, ahead/behind) | ✅ | |
| Stash push / pop / apply / drop | ✅ | |
| Checkout, branch here, cherry-pick, revert, reset | ✅ | buttons in detail panel |
| Merge/rebase/cherry-pick in progress banner (continue/abort) | ✅ | |
| Multi-select files in staging (⌘/Shift-click; stage, unstage, discard) | ✅ | |
| Folder view for file lists (staging + commit files) | ✅ | "Folders" toggle; stage a whole folder |
| Hunk- and line-level staging / unstaging | ✅ | `partial_patch` + `git apply --cached --recount`, integration tested |
| Discard changes (file / hunk / lines) | ✅ | with confirmation |
| Right-click context menus (commit, branch, remote, tag, stash, file) | ✅ | branch and `HEAD` labels include their commit's actions; branch, remote and `HEAD` menus click-tested, the rest not yet |
| Branch menu: fast-forward, merge into it, interactive rebase onto it, force delete, set upstream, push and start a PR | ✅ | fast-forward only when the branch is behind HEAD; force delete only when it has commits HEAD doesn't; the PR item needs `gh` |
| Commit menu: drop commit, move commit up / down | ✅ | commits on HEAD with no merges after them; a commit a remote has asks first |
| Create/rename/delete branch & tag dialogs | ✅ | prompt overlay |
| Auto-refresh on file changes (fs watcher) | ✅ | status-only vs full reload |
| Command palette (⌘P / ⌘K) | ✅ | actions, branches, tabs, repos; fuzzy |
| Search commits (message, author, sha) | ✅ | ⌘F, dims non-matches, Enter / ⇧Enter |
| Tab per repo, recent repos, restore on launch | ✅ | `shell.rs`; ⌘T / ⌘W / ctrl-tab |
| Project folders (parent folder → all repos inside) | ✅ | home tab, scan depth 4, "Open all", collapsible groups |
| Embedded terminal (per repo, multiple tabs) | ✅ | `alacritty_terminal` PTY, ⌥T; no mouse selection/copy yet |
| Version-aware tag sorting | ✅ | |
| Lazy paging past 50k commits | ⬜ | |

## Tier 2 — Solid
| Feature | Status |
|---|---|
| Merge conflict editor | ✅ fills the window: ours and theirs side by side with a checkbox per conflict (both = keep both), live output below, jump between conflicts, syntax colors, whole-file take ours/theirs, save & mark resolved, next file opens by itself; output isn't editable yet |
| Drag-and-drop branch → merge/rebase | ✅ sidebar branches and graph labels; options depend on which side is checked out |
| Interactive rebase (reorder/squash/edit) | ✅ pick, reword, squash, fixup, drop, reorder; undoable |
| File history + blame | ✅ history follows renames (diff per commit); blame with jump-to-commit |
| Side-by-side diff, syntax highlighting | ✅ syntect (pure Rust); Split/Unified toggle, line staging in both |
| Undo / redo of operations | ✅ ref snapshots before/after every op; soft undo for commits, `reset --keep` otherwise |
| Submodules, LFS, worktrees | 🟡 worktrees ✅, submodules ✅ (list, init/update, open in tab); LFS todo |
| Commit signing indicators | ✅ GPG/SSH status in commit details |
| Graph compaction for wide histories | 🟡 date order keeps wide histories close to `git log --graph --date-order`; drag the graph column's edge (down to one dot per commit) |
| Packaging: signed .dmg, Windows, Linux, auto-update | 🟡 auto-update from GitHub releases (checksum-verified) ✅; signing todo |

## Tier 3 — Complete
| Feature | Status |
|---|---|
| Launchpad: PRs/issues across GitHub/GitLab/Bitbucket/Azure | 💎 |
| Hosting integrations (OAuth, create PR, clone from) | ⬜ / 💎 sign-in waits for a server for the OAuth flow |
| Workspaces (multi-repo) | 🟡 project folders; Fetch all / Pull all per home group, a conflicting pull is undone |
| Settings page: git identity, defaults, signing, SSH keys | ✅ global or per repo; generate/test Ed25519 keys |
| Profile pictures (Gravatar, GitHub no-reply), opt-in | ✅ |
| Footer: version, update status, About & credits | ✅ |
| Profiles | ⬜ |
| AI commit messages / explain commit | 🟡 commit message (optionally automatic once everything is staged), explain commit, branch & stash names (BYOK) |
| Release notes from tag range → gitchrono.com changelog | 💎 |
| Cloud patches | 💎 |

## Next up (2026-10-23)

Ordered by how much a daily user misses them.

| Feature | Scope | Status |
|---|---|---|
| Command palette | ⌘P: run any action, "Open in" detected IDEs | ✅ (no "Open in" yet) |
| Search / filter graph | message, author, sha, file; highlight matches | ✅ (not by file yet) |
| Left-panel filter | filter branches/remotes/tags as you type | ✅ |
| Terminal: selection + copy, links, drag files in | xterm.js features | 🟡 selection (drag, word, line, ⇧-extend), ⌘C, ⌘A; no links/drag yet |
| Worktrees | create/open/remove, WIP row per worktree, tab per worktree | ✅ (no WIP row per worktree yet) |
| Undo / redo | reset, rebase, checkout, commit via reflog | ✅ |
| File history + blame | history follows renames | ✅ |
| Split / inline / hunk diff views, syntax highlighting | | ✅ |
| Merge conflict tool | 3-way, AI-assisted resolution | ✅ (AI suggestions later) |
| Interactive rebase editor | reorder, squash, reword, drop | ✅ |
| Drag a branch onto another | merge / rebase / PR | ✅ (PR later) |
| Select several commits | combined diff, patch, cherry-pick range | ✅ ⌘/Shift-click: cherry-pick, revert, rebase, copy SHAs, in the panel and on right-click |
| Hide / solo / pin branches | | ✅ hide & solo (saved per repo); pin todo |
| Tab aliases, tab reordering | | ⬜ |
| Clone / init dialogs | including from GitHub/GitLab | ⬜ |
| Submodules and LFS | | 🟡 submodules ✅, LFS todo |
| Commit signing (GPG/SSH) and indicators | | ✅ indicators |
| Zoom levels / font size | 140–300% | ⬜ |
| Integrations and Launchpad | PRs/issues across hosts, Jira | 🟡 GitHub PRs via `gh` (list, open, check out, create) |
| AI: explain commit, compose commits, conflict resolution | | ⬜ / 💎 |
| Agent sessions | status of Claude Code / Codex in worktrees | ⬜ (later) |
