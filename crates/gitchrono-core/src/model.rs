//! Plain data types shared by the engine and the UI. No gix types leak out.

/// A commit as shown in the graph.
#[derive(Debug, Clone)]
pub struct Commit {
    pub id: String,
    pub short_id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Seconds since the Unix epoch (author time).
    pub time: i64,
    pub summary: String,
    pub body: String,
    pub refs: Vec<RefLabel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Head,
    Local,
    Remote,
    Tag,
    Stash,
}

/// A decoration drawn on a commit row (branch tip, tag, HEAD...).
#[derive(Debug, Clone)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
    /// True for the branch HEAD points at.
    pub current: bool,
}

#[derive(Debug, Clone)]
pub struct Branch {
    /// Short name, e.g. `main` or `origin/main`.
    pub name: String,
    pub full_name: String,
    pub target: String,
    pub current: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// Every commit of this local branch is in HEAD.
    pub in_head: bool,
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct Stash {
    pub index: usize,
    pub message: String,
    pub target: String,
}

#[derive(Debug, Clone, Default)]
pub struct Refs {
    pub head_branch: Option<String>,
    pub head_id: Option<String>,
    pub local: Vec<Branch>,
    pub remote: Vec<Branch>,
    pub tags: Vec<Tag>,
    pub stashes: Vec<Stash>,
    /// Configured remote names, as `git remote` lists them.
    pub remotes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
    Untracked,
    Conflicted,
}

impl FileStatus {
    pub fn letter(self) -> &'static str {
        match self {
            FileStatus::Added => "A",
            FileStatus::Modified => "M",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Copied => "C",
            FileStatus::TypeChange => "T",
            FileStatus::Untracked => "U",
            FileStatus::Conflicted => "!",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Operation {
    #[default]
    None,
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub conflicted: Vec<FileChange>,
    pub operation: Operation,
}

impl Status {
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty() && self.conflicted.is_empty()
    }

    pub fn change_count(&self) -> usize {
        self.staged.len() + self.unstaged.len() + self.conflicted.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// Followed by `\ No newline at end of file` in the patch.
    pub no_eol: bool,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    pub path: String,
    /// Raw patch lines before the first hunk (`diff --git`, modes, `---`/`+++`).
    pub header: String,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
    pub additions: usize,
    pub deletions: usize,
}

/// A linked (or the main) working tree of the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: std::path::PathBuf,
    pub head: Option<String>,
    /// Short branch name; `None` when detached.
    pub branch: Option<String>,
    pub main: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// One commit in a file's history, with the file's path at that commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCommit {
    pub id: String,
    pub author_name: String,
    pub time: i64,
    pub summary: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    pub id: String,
    pub author_name: String,
    pub time: i64,
    pub summary: String,
    /// Line number in the blamed revision (1-based).
    pub line_no: usize,
    pub text: String,
}

/// GPG/SSH signature check of a commit (`%G?`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// G good, U good but unknown validity, X/Y expired, R revoked, B bad, E can't check.
    pub status: char,
    pub signer: String,
    pub key: String,
}

impl Signature {
    pub fn is_good(&self) -> bool {
        matches!(self.status, 'G' | 'U')
    }

    pub fn describe(&self) -> &'static str {
        match self.status {
            'G' => "Good signature",
            'U' => "Good signature (unknown validity)",
            'X' => "Good signature, expired",
            'Y' => "Good signature, expired key",
            'R' => "Signed with a revoked key",
            'B' => "Bad signature",
            _ => "Signed, but the key isn't available to verify",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submodule {
    pub path: String,
    pub sha: String,
    /// ' ' up to date, '-' not initialized, '+' checked-out commit differs, 'U' conflicts.
    pub state: char,
}
