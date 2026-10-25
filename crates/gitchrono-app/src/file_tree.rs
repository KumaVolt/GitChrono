//! Folder view for file lists: groups paths into directories, folders first,
//! with single-child chains joined ("src/app") like most editors.

use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    /// `path` is the full directory path (the collapse key); `files` are the
    /// indices of every file below it, for staging a whole folder.
    Dir { path: String, name: String, depth: usize, files: Vec<usize>, collapsed: bool },
    File { index: usize, name: String, depth: usize },
}

#[derive(Default)]
struct Node {
    dirs: BTreeMap<String, Node>,
    files: Vec<(String, usize)>,
}

impl Node {
    fn all_files(&self, out: &mut Vec<usize>) {
        for dir in self.dirs.values() {
            dir.all_files(out);
        }
        out.extend(self.files.iter().map(|(_, i)| *i));
    }
}

/// Rows for `paths` (indices refer to that slice); folders in `collapsed`
/// show without their contents.
pub fn build(paths: &[&str], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = Node::default();
    for (index, path) in paths.iter().enumerate() {
        let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let Some(name) = parts.pop() else { continue };
        let mut node = &mut root;
        for part in parts {
            node = node.dirs.entry(part.to_string()).or_default();
        }
        node.files.push((name.to_string(), index));
    }
    let mut rows = Vec::new();
    emit(&root, "", 0, collapsed, &mut rows);
    rows
}

fn emit(node: &Node, prefix: &str, depth: usize, collapsed: &HashSet<String>, rows: &mut Vec<TreeRow>) {
    for (name, mut dir) in &node.dirs {
        let mut name = name.clone();
        while dir.files.is_empty() && dir.dirs.len() == 1 {
            let (child, next) = dir.dirs.iter().next().expect("one child");
            name = format!("{name}/{child}");
            dir = next;
        }
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let mut files = Vec::new();
        dir.all_files(&mut files);
        let is_collapsed = collapsed.contains(&path);
        rows.push(TreeRow::Dir { path: path.clone(), name, depth, files, collapsed: is_collapsed });
        if !is_collapsed {
            emit(dir, &path, depth + 1, collapsed, rows);
        }
    }
    let mut files = node.files.clone();
    files.sort();
    rows.extend(files.into_iter().map(|(name, index)| TreeRow::File { index, name, depth }));
}

/// File indices in the order the rows show them (for Shift-click ranges).
pub fn visible_files(rows: &[TreeRow]) -> Vec<usize> {
    rows.iter().filter_map(|r| if let TreeRow::File { index, .. } = r { Some(*index) } else { None }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                TreeRow::Dir { name, depth, .. } => format!("{}{name}/", "  ".repeat(*depth)),
                TreeRow::File { name, depth, .. } => format!("{}{name}", "  ".repeat(*depth)),
            })
            .collect()
    }

    #[test]
    fn groups_folders_first_and_joins_chains() {
        let paths = ["README.md", "crates/app/src/main.rs", "crates/app/src/shell.rs", "crates/core/lib.rs", "Cargo.toml"];
        let rows = build(&paths, &HashSet::new());
        assert_eq!(names(&rows), ["crates/", "  app/src/", "    main.rs", "    shell.rs", "  core/", "    lib.rs", "Cargo.toml", "README.md"]);
        let TreeRow::Dir { files, path, .. } = &rows[0] else { panic!() };
        assert_eq!((path.as_str(), files.len()), ("crates", 3));
        let TreeRow::Dir { path, .. } = &rows[1] else { panic!() };
        assert_eq!(path, "crates/app/src");
        assert_eq!(visible_files(&rows), [1, 2, 3, 4, 0]);
    }

    #[test]
    fn collapsed_folders_hide_contents() {
        let paths = ["a/x", "a/y", "b"];
        let rows = build(&paths, &HashSet::from(["a".to_string()]));
        assert_eq!(names(&rows), ["a/", "b"]);
        assert!(matches!(&rows[0], TreeRow::Dir { collapsed: true, files, .. } if files.len() == 2));
    }
}
