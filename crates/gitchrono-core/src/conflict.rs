//! Parse a conflicted working-tree file and write it back with choices applied.

use std::path::Path;

use anyhow::Result;

use crate::repo::Repo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chunk {
    Common(Vec<String>),
    Conflict(Conflict),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub ours_label: String,
    pub theirs_label: String,
    pub ours: Vec<String>,
    /// Present with `merge.conflictStyle = diff3 / zdiff3`.
    pub base: Option<Vec<String>>,
    pub theirs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Ours,
    Theirs,
    OursThenTheirs,
    TheirsThenOurs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFile {
    pub path: String,
    pub chunks: Vec<Chunk>,
    /// The file ended with a newline.
    pub trailing_newline: bool,
}

impl ConflictFile {
    pub fn parse(path: &str, text: &str) -> Self {
        let trailing_newline = text.ends_with('\n');
        let mut chunks = Vec::new();
        let mut common = Vec::new();
        let mut current: Option<(Conflict, u8)> = None; // 0 ours, 1 base, 2 theirs
        for line in text.lines() {
            if let Some((conflict, part)) = &mut current {
                if line.starts_with("|||||||") && *part == 0 {
                    conflict.base = Some(Vec::new());
                    *part = 1;
                } else if line == "=======" && *part < 2 {
                    *part = 2;
                } else if let Some(label) = line.strip_prefix(">>>>>>>").filter(|_| *part == 2) {
                    conflict.theirs_label = label.trim().to_string();
                    chunks.push(Chunk::Conflict(current.take().unwrap().0));
                } else {
                    let target = match part {
                        0 => &mut conflict.ours,
                        1 => conflict.base.get_or_insert_with(Vec::new),
                        _ => &mut conflict.theirs,
                    };
                    target.push(line.to_string());
                }
            } else if let Some(label) = line.strip_prefix("<<<<<<<") {
                if !common.is_empty() {
                    chunks.push(Chunk::Common(std::mem::take(&mut common)));
                }
                let conflict = Conflict {
                    ours_label: label.trim().to_string(),
                    theirs_label: String::new(),
                    ours: Vec::new(),
                    base: None,
                    theirs: Vec::new(),
                };
                current = Some((conflict, 0));
            } else {
                common.push(line.to_string());
            }
        }
        // An unterminated marker block is kept as plain text.
        if let Some((conflict, _)) = current {
            common.push(format!("<<<<<<< {}", conflict.ours_label));
            common.extend(conflict.ours);
        }
        if !common.is_empty() {
            chunks.push(Chunk::Common(common));
        }
        Self { path: path.to_string(), chunks, trailing_newline }
    }

    pub fn conflict_count(&self) -> usize {
        self.chunks.iter().filter(|c| matches!(c, Chunk::Conflict(_))).count()
    }

    /// File text with `choices[i]` applied to the i-th conflict; `None` keeps its markers.
    pub fn render(&self, choices: &[Option<Resolution>]) -> String {
        let mut out = Vec::new();
        let mut i = 0;
        for chunk in &self.chunks {
            match chunk {
                Chunk::Common(lines) => out.extend(lines.iter().cloned()),
                Chunk::Conflict(c) => {
                    match choices.get(i).copied().flatten() {
                        Some(Resolution::Ours) => out.extend(c.ours.iter().cloned()),
                        Some(Resolution::Theirs) => out.extend(c.theirs.iter().cloned()),
                        Some(Resolution::OursThenTheirs) => out.extend(c.ours.iter().chain(&c.theirs).cloned()),
                        Some(Resolution::TheirsThenOurs) => out.extend(c.theirs.iter().chain(&c.ours).cloned()),
                        None => {
                            out.push(format!("<<<<<<< {}", c.ours_label));
                            out.extend(c.ours.iter().cloned());
                            if let Some(base) = &c.base {
                                out.push("||||||| base".to_string());
                                out.extend(base.iter().cloned());
                            }
                            out.push("=======".to_string());
                            out.extend(c.theirs.iter().cloned());
                            out.push(format!(">>>>>>> {}", c.theirs_label));
                        }
                    }
                    i += 1;
                }
            }
        }
        let mut text = out.join("\n");
        if self.trailing_newline && !text.is_empty() {
            text.push('\n');
        }
        text
    }
}

impl Repo {
    pub fn conflict_file(&self, path: &str) -> Result<ConflictFile> {
        let text = std::fs::read_to_string(self.root().join(path))?;
        Ok(ConflictFile::parse(path, &text))
    }

    /// Write the resolved text and, when no markers remain, mark it resolved.
    pub fn save_resolution(&self, file: &ConflictFile, choices: &[Option<Resolution>]) -> Result<bool> {
        std::fs::write(self.root().join(&file.path), file.render(choices))?;
        let resolved = choices.len() == file.conflict_count() && choices.iter().all(Option::is_some);
        if resolved {
            self.git(&["add", "--", &file.path])?;
        }
        Ok(resolved)
    }

    /// Take one side for the whole file (`git checkout --ours/--theirs`) and mark it resolved.
    pub fn resolve_whole_file(&self, path: &str, ours: bool) -> Result<()> {
        let side = if ours { "--ours" } else { "--theirs" };
        self.git(&["checkout", side, "--", path])?;
        self.git(&["add", "--", path]).map(drop)
    }

    pub fn file_exists(&self, path: &str) -> bool {
        Path::new(&self.root().join(path)).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "a\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> feature\nb\n<<<<<<< HEAD\nx\n||||||| base\nw\n=======\ny\n>>>>>>> feature\n";

    #[test]
    fn parses_and_renders() {
        let file = ConflictFile::parse("f", TEXT);
        assert_eq!(file.conflict_count(), 2);
        let Chunk::Conflict(c) = &file.chunks[1] else { panic!() };
        assert_eq!((c.ours_label.as_str(), c.theirs_label.as_str()), ("HEAD", "feature"));
        assert_eq!(c.ours, ["ours 1", "ours 2"]);
        let Chunk::Conflict(c2) = &file.chunks[3] else { panic!() };
        assert_eq!(c2.base.as_deref(), Some(&["w".to_string()][..]));

        assert_eq!(file.render(&[Some(Resolution::Theirs), Some(Resolution::OursThenTheirs)]), "a\ntheirs\nb\nx\ny\n");
        // Unresolved conflicts round-trip with markers.
        let partial = file.render(&[Some(Resolution::Ours), None]);
        assert!(partial.starts_with("a\nours 1\nours 2\nb\n<<<<<<< HEAD\nx\n"));
        assert!(partial.ends_with(">>>>>>> feature\n"));
    }
}
