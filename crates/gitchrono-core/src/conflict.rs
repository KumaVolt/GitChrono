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

impl Resolution {
    /// `choice` after ticking or unticking one side of a conflict. Ticking the
    /// second side keeps both, the side ticked first going first.
    pub fn toggle(choice: Option<Self>, ours: bool) -> Option<Self> {
        match (choice, ours) {
            (None, true) => Some(Self::Ours),
            (None, false) => Some(Self::Theirs),
            (Some(Self::Ours), true) | (Some(Self::Theirs), false) => None,
            (Some(Self::Ours), false) => Some(Self::OursThenTheirs),
            (Some(Self::Theirs), true) => Some(Self::TheirsThenOurs),
            (Some(Self::OursThenTheirs | Self::TheirsThenOurs), true) => Some(Self::Theirs),
            (Some(Self::OursThenTheirs | Self::TheirsThenOurs), false) => Some(Self::Ours),
        }
    }

    pub fn includes(self, ours: bool) -> bool {
        match self {
            Self::Ours => ours,
            Self::Theirs => !ours,
            Self::OursThenTheirs | Self::TheirsThenOurs => true,
        }
    }
}

/// A line and its 1-based number in the file it's from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub number: usize,
    pub text: String,
}

/// One row of the side-by-side panes. Shared lines sit next to each other,
/// and a conflict's shorter side is padded, so the panes stay aligned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideRow {
    pub ours: Option<Line>,
    pub theirs: Option<Line>,
    /// Which conflict this row is part of.
    pub conflict: Option<usize>,
    /// The conflict's first row, which carries its checkboxes.
    pub starts_conflict: bool,
}

/// One row of the merged output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRow {
    /// The merged line, numbered as in the output; `None` stands in for a
    /// conflict with nothing chosen yet.
    pub line: Option<Line>,
    pub conflict: Option<usize>,
    /// Which side the line came from (ours for shared lines), and its number there.
    pub ours: bool,
    pub source: usize,
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

    /// The file as the two sides have it, row by row.
    pub fn side_rows(&self) -> Vec<SideRow> {
        let mut rows = Vec::new();
        let (mut ours_at, mut theirs_at, mut index) = (0, 0, 0);
        let take = |at: &mut usize, text: &String| {
            *at += 1;
            Line { number: *at, text: text.clone() }
        };
        for chunk in &self.chunks {
            match chunk {
                Chunk::Common(lines) => {
                    for text in lines {
                        let (ours, theirs) = (take(&mut ours_at, text), take(&mut theirs_at, text));
                        rows.push(SideRow { ours: Some(ours), theirs: Some(theirs), conflict: None, starts_conflict: false });
                    }
                }
                Chunk::Conflict(c) => {
                    // Never no rows: a side may be empty, and the conflict still needs its checkboxes.
                    for i in 0..c.ours.len().max(c.theirs.len()).max(1) {
                        let ours = c.ours.get(i).map(|text| take(&mut ours_at, text));
                        let theirs = c.theirs.get(i).map(|text| take(&mut theirs_at, text));
                        rows.push(SideRow { ours, theirs, conflict: Some(index), starts_conflict: i == 0 });
                    }
                    index += 1;
                }
            }
        }
        rows
    }

    /// The merged file with `choices` applied, for the output pane.
    pub fn output_rows(&self, choices: &[Option<Resolution>]) -> Vec<OutputRow> {
        let mut rows: Vec<OutputRow> = Vec::new();
        let (mut number, mut ours_at, mut theirs_at, mut index) = (0, 0, 0, 0);
        let mut push = |rows: &mut Vec<OutputRow>, text: &String, conflict, ours, source| {
            number += 1;
            rows.push(OutputRow { line: Some(Line { number, text: text.clone() }), conflict, ours, source });
        };
        for chunk in &self.chunks {
            match chunk {
                Chunk::Common(lines) => {
                    for text in lines {
                        ours_at += 1;
                        theirs_at += 1;
                        push(&mut rows, text, None, true, ours_at);
                    }
                }
                Chunk::Conflict(c) => {
                    let (ours_start, theirs_start) = (ours_at, theirs_at);
                    ours_at += c.ours.len();
                    theirs_at += c.theirs.len();
                    let order: &[bool] = match choices.get(index).copied().flatten() {
                        None => &[],
                        Some(Resolution::Ours) => &[true],
                        Some(Resolution::Theirs) => &[false],
                        Some(Resolution::OursThenTheirs) => &[true, false],
                        Some(Resolution::TheirsThenOurs) => &[false, true],
                    };
                    if order.is_empty() {
                        rows.push(OutputRow { line: None, conflict: Some(index), ours: true, source: 0 });
                    }
                    for &ours in order {
                        let (lines, start) = if ours { (&c.ours, ours_start) } else { (&c.theirs, theirs_start) };
                        for (i, text) in lines.iter().enumerate() {
                            push(&mut rows, text, Some(index), ours, start + i + 1);
                        }
                    }
                    index += 1;
                }
            }
        }
        rows
    }

    /// What the markers call the two sides, from the first conflict.
    pub fn conflict_labels(&self) -> Option<(&str, &str)> {
        self.chunks.iter().find_map(|chunk| match chunk {
            Chunk::Conflict(c) => Some((c.ours_label.as_str(), c.theirs_label.as_str())),
            Chunk::Common(_) => None,
        })
    }

    /// The text of one side: shared lines plus that side of every conflict.
    pub fn side_text(&self, ours: bool) -> Vec<&str> {
        let mut lines = Vec::new();
        for chunk in &self.chunks {
            match chunk {
                Chunk::Common(common) => lines.extend(common.iter().map(String::as_str)),
                Chunk::Conflict(c) => lines.extend((if ours { &c.ours } else { &c.theirs }).iter().map(String::as_str)),
            }
        }
        lines
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

    #[test]
    fn the_sides_line_up_row_by_row() {
        let file = ConflictFile::parse("f", TEXT);
        let rows = file.side_rows();
        let side = |r: &Option<Line>| r.as_ref().map(|l| (l.number, l.text.clone()));
        assert_eq!(rows.len(), 5, "a, 2 for the longer side, b, x/y");
        assert_eq!((side(&rows[0].ours), side(&rows[0].theirs)), (Some((1, "a".into())), Some((1, "a".into()))));

        assert_eq!((rows[1].conflict, rows[1].starts_conflict), (Some(0), true));
        assert_eq!((side(&rows[1].ours), side(&rows[1].theirs)), (Some((2, "ours 1".into())), Some((2, "theirs".into()))));
        assert_eq!((rows[2].conflict, rows[2].starts_conflict), (Some(0), false));
        assert_eq!((side(&rows[2].ours), side(&rows[2].theirs)), (Some((3, "ours 2".into())), None), "the shorter side is padded");

        assert_eq!((side(&rows[3].ours), side(&rows[3].theirs)), (Some((4, "b".into())), Some((3, "b".into()))), "each side keeps its own numbers");
        assert_eq!((rows[4].conflict, side(&rows[4].ours), side(&rows[4].theirs)), (Some(1), Some((5, "x".into())), Some((4, "y".into()))));
    }

    #[test]
    fn a_conflict_with_an_empty_side_keeps_a_row() {
        let file = ConflictFile::parse("f", "<<<<<<< HEAD\n=======\n>>>>>>> other\nend\n");
        let rows = file.side_rows();
        assert_eq!((rows[0].conflict, rows[0].starts_conflict, rows[0].ours.is_none()), (Some(0), true, true));
    }

    #[test]
    fn the_output_follows_the_choices() {
        let file = ConflictFile::parse("f", TEXT);
        fn text(rows: &[OutputRow]) -> Vec<Option<&str>> {
            rows.iter().map(|r| r.line.as_ref().map(|l| l.text.as_str())).collect()
        }

        let open = file.output_rows(&[None, None]);
        assert_eq!(text(&open), [Some("a"), None, Some("b"), None], "undecided conflicts show as gaps");
        assert_eq!(open[2].line.as_ref().unwrap().number, 2, "they take no line number");

        let both = file.output_rows(&[Some(Resolution::TheirsThenOurs), Some(Resolution::Ours)]);
        assert_eq!(text(&both), [Some("a"), Some("theirs"), Some("ours 1"), Some("ours 2"), Some("b"), Some("x")]);
        assert_eq!(both.iter().map(|r| r.line.as_ref().unwrap().number).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
        assert_eq!((both[1].ours, both[1].source, both[1].conflict), (false, 2, Some(0)));
        assert_eq!((both[3].ours, both[3].source), (true, 3));
        assert_eq!((both[5].ours, both[5].source, both[5].conflict), (true, 5, Some(1)), "numbered as in that side");
    }

    #[test]
    fn the_output_matches_what_gets_saved() {
        let file = ConflictFile::parse("f", TEXT);
        let choices = [Some(Resolution::OursThenTheirs), Some(Resolution::Theirs)];
        let shown: Vec<String> = file.output_rows(&choices).into_iter().filter_map(|r| r.line).map(|l| l.text).collect();
        assert_eq!(shown.join("\n") + "\n", file.render(&choices));
    }

    #[test]
    fn ticking_sides_builds_a_resolution() {
        use Resolution::*;
        assert_eq!(Resolution::toggle(None, true), Some(Ours));
        assert_eq!(Resolution::toggle(Some(Ours), true), None);
        assert_eq!(Resolution::toggle(Some(Ours), false), Some(OursThenTheirs));
        assert_eq!(Resolution::toggle(Some(Theirs), true), Some(TheirsThenOurs));
        assert_eq!(Resolution::toggle(Some(OursThenTheirs), true), Some(Theirs));
        assert_eq!(Resolution::toggle(Some(TheirsThenOurs), false), Some(Ours));
        assert!(OursThenTheirs.includes(true) && OursThenTheirs.includes(false));
        assert!(Ours.includes(true) && !Ours.includes(false));
    }

    #[test]
    fn each_side_reads_as_its_own_file() {
        let file = ConflictFile::parse("f", TEXT);
        assert_eq!(file.side_text(true), ["a", "ours 1", "ours 2", "b", "x"]);
        assert_eq!(file.side_text(false), ["a", "theirs", "b", "y"]);
    }
}
