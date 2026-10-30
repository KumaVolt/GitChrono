//! Branch actions from the context menus, against a real `git`.

use std::path::{Path, PathBuf};

use gitchrono_core::{Operation, RebaseAction, Repo, Rewritable, UndoEntry, git, rewritable};

fn repo(name: &str) -> (PathBuf, Repo) {
    let dir = std::env::temp_dir().join(format!("gitchrono-branches-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Test"],
        &["config", "user.email", "test@example.com"],
        &["config", "commit.gpgsign", "false"],
        &["config", "core.autocrlf", "false"],
    ] {
        git(&dir, args).unwrap();
    }
    commit(&dir, "base");
    let repo = Repo::open(&dir).unwrap();
    (dir, repo)
}

fn commit(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), name).unwrap();
    git(dir, &["add", name]).unwrap();
    git(dir, &["commit", "-qm", name]).unwrap();
}

fn tip(dir: &Path, rev: &str) -> String {
    git(dir, &["rev-parse", rev]).unwrap().trim().to_string()
}

fn current(dir: &Path) -> String {
    git(dir, &["symbolic-ref", "--short", "HEAD"]).unwrap().trim().to_string()
}

fn in_head(repo: &Repo, name: &str) -> bool {
    repo.snapshot(100).unwrap().refs.local.iter().find(|b| b.name == name).unwrap().in_head
}

/// Run `op`, returning its undo entry.
fn record(repo: &Repo, op: impl FnOnce()) -> UndoEntry {
    let before = repo.ref_state().unwrap();
    op();
    UndoEntry::new("op", before, repo.ref_state().unwrap(), false).expect("op changed refs")
}

#[test]
fn fast_forward_moves_a_branch_that_is_not_checked_out() {
    let (dir, repo) = repo("ff");
    git(&dir, &["switch", "-qc", "feature"]).unwrap();
    commit(&dir, "one");
    let wanted = tip(&dir, "feature");
    let old_main = tip(&dir, "main");
    assert!(in_head(&repo, "main"));

    let entry = record(&repo, || repo.fast_forward("main", "feature").unwrap());
    assert_eq!(tip(&dir, "main"), wanted);
    assert_eq!(current(&dir), "feature", "stays on the branch it was on");

    repo.undo(&entry).unwrap();
    assert_eq!(tip(&dir, "main"), old_main);
}

#[test]
fn fast_forward_is_refused_when_branches_diverged() {
    let (dir, repo) = repo("ff-diverged");
    git(&dir, &["switch", "-qc", "feature"]).unwrap();
    commit(&dir, "one");
    git(&dir, &["switch", "-q", "main"]).unwrap();
    commit(&dir, "two");
    let old = tip(&dir, "feature");
    git(&dir, &["switch", "-q", "feature"]).unwrap();
    assert!(!in_head(&repo, "main"), "main has a commit feature doesn't");

    let main = tip(&dir, "main");
    assert!(repo.fast_forward("main", "feature").is_err());
    assert_eq!((tip(&dir, "main"), tip(&dir, "feature")), (main, old));
}

#[test]
fn fast_forward_is_refused_for_a_checked_out_branch() {
    let (dir, repo) = repo("ff-worktree");
    git(&dir, &["branch", "behind"]).unwrap();
    commit(&dir, "one");
    let linked = dir.with_extension("linked");
    let _ = std::fs::remove_dir_all(&linked);
    repo.add_worktree(&linked, None, Some("behind")).unwrap();

    let before = tip(&dir, "behind");
    assert!(repo.fast_forward("behind", "main").is_err());
    assert_eq!(tip(&dir, "behind"), before);
    repo.remove_worktree(&linked, true).unwrap();
}

#[test]
fn fast_forward_targets_the_branch_when_a_tag_has_its_name() {
    let (dir, repo) = repo("ff-tag");
    git(&dir, &["branch", "same"]).unwrap();
    git(&dir, &["tag", "same"]).unwrap();
    let tag = tip(&dir, "refs/tags/same");
    commit(&dir, "one");

    repo.fast_forward("same", "main").unwrap();
    assert_eq!(tip(&dir, "refs/heads/same"), tip(&dir, "main"));
    assert_eq!(tip(&dir, "refs/tags/same"), tag, "the tag stays");
    assert!(in_head(&repo, "same"), "matched by full ref name");
}

#[test]
fn merge_into_ends_on_the_target_and_undo_comes_back() {
    let (dir, repo) = repo("merge-into");
    git(&dir, &["switch", "-qc", "feature"]).unwrap();
    commit(&dir, "one");
    let feature = tip(&dir, "feature");
    git(&dir, &["switch", "-q", "main"]).unwrap();
    commit(&dir, "two");
    let main = tip(&dir, "main");
    git(&dir, &["switch", "-q", "feature"]).unwrap();

    let entry = record(&repo, || repo.merge_into("feature", "main").unwrap());
    assert_eq!(current(&dir), "main");
    assert!(dir.join("one").exists() && dir.join("two").exists());
    assert_eq!(tip(&dir, "feature"), feature, "the source doesn't move");

    repo.undo(&entry).unwrap();
    assert_eq!(current(&dir), "feature");
    assert_eq!(tip(&dir, "main"), main);
}

#[test]
fn merge_into_keeps_uncommitted_changes_when_the_branches_agree_on_the_file() {
    let (dir, repo) = repo("merge-into-dirty");
    git(&dir, &["switch", "-qc", "feature"]).unwrap();
    commit(&dir, "one");
    std::fs::write(dir.join("base"), "changed").unwrap();

    repo.merge_into("feature", "main").unwrap();
    assert_eq!(current(&dir), "main");
    assert_eq!(std::fs::read_to_string(dir.join("base")).unwrap(), "changed");
    assert!(dir.join("one").exists());
}

#[test]
fn force_delete_removes_an_unmerged_branch_and_undo_restores_it() {
    let (dir, repo) = repo("force-delete");
    git(&dir, &["switch", "-qc", "extra"]).unwrap();
    commit(&dir, "one");
    let extra = tip(&dir, "extra");
    git(&dir, &["switch", "-q", "main"]).unwrap();

    assert!(repo.delete_branch("extra", false).is_err(), "a plain delete refuses");
    let entry = record(&repo, || repo.delete_branch("extra", true).unwrap());
    assert!(git(&dir, &["rev-parse", "--verify", "-q", "refs/heads/extra"]).is_err());

    repo.undo(&entry).unwrap();
    assert_eq!(tip(&dir, "refs/heads/extra"), extra);
}

#[test]
fn interactive_rebase_onto_a_branch() {
    let (dir, repo) = repo("rebase-onto");
    git(&dir, &["switch", "-qc", "topic"]).unwrap();
    commit(&dir, "one");
    commit(&dir, "two");
    git(&dir, &["switch", "-qc", "feature/ñandú", "main"]).unwrap();
    commit(&dir, "three");
    git(&dir, &["switch", "-q", "topic"]).unwrap();

    let onto = "feature/ñandú";
    let mut plan = repo.rebase_plan(onto).unwrap();
    assert_eq!(plan.iter().map(|s| s.summary.as_str()).collect::<Vec<_>>(), ["one", "two"]);
    plan[1].action = RebaseAction::Drop;
    repo.interactive_rebase(onto, &plan).unwrap();

    let log = git(&dir, &["log", "--format=%s", "--reverse"]).unwrap();
    assert_eq!(log.lines().collect::<Vec<_>>(), ["base", "three", "one"]);
}

/// Byte 7 of this name is inside the `ñ`.
#[test]
fn interactive_rebase_error_names_a_non_ascii_branch_whole() {
    let (dir, repo) = repo("rebase-merges");
    git(&dir, &["switch", "-qc", "abcdefñ"]).unwrap();
    git(&dir, &["switch", "-qc", "side", "main"]).unwrap();
    commit(&dir, "one");
    git(&dir, &["switch", "-qc", "topic", "main"]).unwrap();
    commit(&dir, "two");
    git(&dir, &["merge", "--no-edit", "side"]).unwrap();

    let error = repo.rebase_plan("abcdefñ").unwrap_err().to_string();
    assert!(error.contains("abcdefñ"), "{error}");
}

fn subjects(dir: &Path) -> Vec<String> {
    git(dir, &["log", "--format=%s", "--reverse"]).unwrap().lines().map(str::to_string).collect()
}

/// `base`, `a`, `b`, `c`, each adding its own file.
fn linear(name: &str) -> (PathBuf, Repo) {
    let (dir, repo) = repo(name);
    for c in ["a", "b", "c"] {
        commit(&dir, c);
    }
    (dir, repo)
}

#[test]
fn drop_commit_keeps_the_commits_around_it() {
    let (dir, repo) = linear("drop");
    let entry = record(&repo, || repo.drop_commit("HEAD~1").unwrap());
    assert_eq!(subjects(&dir), ["base", "a", "c"]);
    assert!(!dir.join("b").exists() && dir.join("c").exists());

    repo.undo(&entry).unwrap();
    assert_eq!(subjects(&dir), ["base", "a", "b", "c"]);
}

#[test]
fn drop_commit_works_on_the_newest_and_the_first_commit() {
    let (dir, repo) = linear("drop-ends");
    repo.drop_commit("HEAD").unwrap();
    assert_eq!(subjects(&dir), ["base", "a", "b"]);
    repo.drop_commit("HEAD~2").unwrap();
    assert_eq!(subjects(&dir), ["a", "b"]);
}

#[test]
fn drop_commit_refuses_the_only_commit() {
    let (dir, repo) = repo("drop-only");
    assert!(repo.drop_commit("HEAD").unwrap_err().to_string().contains("only commit"));
    assert_eq!(subjects(&dir), ["base"]);
}

#[test]
fn drop_commit_keeps_uncommitted_changes() {
    let (dir, repo) = linear("drop-dirty");
    std::fs::write(dir.join("a"), "edited").unwrap();
    repo.drop_commit("HEAD~1").unwrap();
    assert_eq!(subjects(&dir), ["base", "a", "c"]);
    assert_eq!(std::fs::read_to_string(dir.join("a")).unwrap(), "edited");
}

#[test]
fn move_commit_swaps_with_its_neighbour() {
    let (dir, repo) = linear("move");
    let up = record(&repo, || repo.move_commit("HEAD~2", true).unwrap());
    assert_eq!(subjects(&dir), ["base", "b", "a", "c"]);
    repo.undo(&up).unwrap();

    repo.move_commit("HEAD~1", false).unwrap();
    assert_eq!(subjects(&dir), ["base", "b", "a", "c"], "b down is a up");
    repo.move_commit("HEAD", false).unwrap();
    assert_eq!(subjects(&dir), ["base", "b", "c", "a"]);
}

#[test]
fn move_commit_can_replace_the_first_commit() {
    let (dir, repo) = linear("move-root");
    repo.move_commit("HEAD~3", true).unwrap();
    assert_eq!(subjects(&dir), ["a", "base", "b", "c"]);
    repo.move_commit("HEAD~2", false).unwrap();
    assert_eq!(subjects(&dir), ["base", "a", "b", "c"]);
}

#[test]
fn move_commit_refuses_the_ends_of_history() {
    let (dir, repo) = linear("move-ends");
    assert!(repo.move_commit("HEAD", true).is_err());
    assert!(repo.move_commit("HEAD~3", false).is_err());
    assert_eq!(subjects(&dir), ["base", "a", "b", "c"]);
}

#[test]
fn move_commit_stops_on_a_conflict_and_can_be_aborted() {
    let (dir, repo) = repo("move-conflict");
    for text in ["one", "two"] {
        std::fs::write(dir.join("same"), text).unwrap();
        git(&dir, &["add", "same"]).unwrap();
        git(&dir, &["commit", "-qm", text]).unwrap();
    }
    let before = tip(&dir, "HEAD");
    assert!(repo.move_commit("HEAD", false).is_err());
    assert_eq!(repo.status().unwrap().operation, Operation::Rebase);

    repo.resolve_operation(Operation::Rebase, true).unwrap();
    assert_eq!(tip(&dir, "HEAD"), before);
}

#[test]
fn rewrites_refuse_merges_and_other_branches() {
    let (dir, repo) = repo("rewrite-refuses");
    git(&dir, &["switch", "-qc", "side"]).unwrap();
    commit(&dir, "side");
    git(&dir, &["switch", "-q", "main"]).unwrap();
    commit(&dir, "main-commit");
    commit(&dir, "before-merge");
    let elsewhere = tip(&dir, "side");
    git(&dir, &["merge", "--no-edit", "side"]).unwrap();
    commit(&dir, "after-merge");

    assert!(repo.drop_commit("HEAD~1").unwrap_err().to_string().contains("merge commit"));
    assert!(repo.drop_commit("HEAD~2").is_err(), "a merge after it would be flattened");
    assert!(repo.move_commit("HEAD", false).is_err(), "its neighbour is a merge");
    git(&dir, &["switch", "-qc", "other", "HEAD~1"]).unwrap();
    git(&dir, &["reset", "-q", "--hard", "main~2"]).unwrap();
    assert!(repo.drop_commit(&elsewhere).unwrap_err().to_string().contains("isn't on the current branch"));
}

#[test]
fn rewritable_follows_the_branch_from_head() {
    let (dir, repo) = repo("rewritable");
    git(&dir, &["switch", "-qc", "side"]).unwrap();
    commit(&dir, "side");
    git(&dir, &["switch", "-q", "main"]).unwrap();
    commit(&dir, "old");
    git(&dir, &["merge", "--no-edit", "side"]).unwrap();
    commit(&dir, "two");
    commit(&dir, "three");

    let commits = repo.snapshot(100).unwrap().commits;
    let head = tip(&dir, "HEAD");
    let check = |rev: &str| rewritable(&commits, &head, &tip(&dir, rev));
    let flags = |drop, move_up, move_down| Some(Rewritable { drop, move_up, move_down });
    assert_eq!(check("HEAD"), flags(true, false, true));
    assert_eq!(check("HEAD~1"), flags(true, true, false), "its parent is the merge");
    assert_eq!(check("HEAD~2"), None, "the merge itself");
    assert_eq!(check("HEAD~3"), None, "below the merge");
    git(&dir, &["branch", "away", "side"]).unwrap();
    assert_eq!(rewritable(&commits, &tip(&dir, "side"), &head), None, "not an ancestor of that HEAD");

    let (dir, repo) = self::repo("rewritable-root");
    let commits = repo.snapshot(100).unwrap().commits;
    let only = tip(&dir, "HEAD");
    assert_eq!(rewritable(&commits, &only, &only), Some(Rewritable { drop: false, move_up: false, move_down: false }));
}

#[test]
fn set_upstream_points_a_branch_at_a_remote_branch() {
    let (dir, repo) = repo("set-upstream");
    let bare = dir.with_extension("origin");
    let _ = std::fs::remove_dir_all(&bare);
    git(&dir, &["init", "-q", "--bare", bare.to_str().unwrap()]).unwrap();
    git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]).unwrap();
    git(&dir, &["push", "-q", "origin", "main:elsewhere"]).unwrap();

    assert!(repo.set_upstream("main", "origin/missing").is_err());
    repo.set_upstream("main", "origin/elsewhere").unwrap();
    assert_eq!(repo.status().unwrap().upstream.as_deref(), Some("origin/elsewhere"));
}
