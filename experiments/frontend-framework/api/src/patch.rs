//! Splits a `git diff` into files, with what the stub needs to know of each.

use serde::Serialize;
use utoipa::ToSchema;

/// How a file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

/// The side of a diff a line is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Deletions,
    Additions,
}

/// One file of a change: its own slice of the patch, and its counts.
#[derive(Clone, Debug)]
pub struct FilePatch {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    /// This file's part of the patch, from its `diff --git` line.
    pub text: String,
    /// The new-file line numbers of the added lines, for seeding threads.
    pub added_lines: Vec<u32>,
}

impl FilePatch {
    /// Parses one file's part of a patch.
    fn parse(text: &str) -> Self {
        let mut path = String::new();
        let mut old_path = None;
        let mut status = FileStatus::Modified;
        let (mut additions, mut deletions, mut new_line) = (0, 0, 0);
        let mut added_lines = Vec::new();
        let mut in_hunk = false;

        for line in text.lines() {
            if let Some(header) = line.strip_prefix("@@ ") {
                in_hunk = true;
                // "@@ -a,b +c,d @@": the hunk starts at new-file line c.
                new_line = header
                    .split_once('+')
                    .and_then(|(_, rest)| rest.split([',', ' ']).next())
                    .and_then(|start| start.parse().ok())
                    .unwrap_or(1);
            } else if in_hunk {
                match line.as_bytes().first() {
                    Some(b'+') => {
                        additions += 1;
                        added_lines.push(new_line);
                        new_line += 1;
                    }
                    Some(b'-') => deletions += 1,
                    Some(b'\\') => {}
                    _ => new_line += 1,
                }
            } else if let Some(header) = line.strip_prefix("diff --git a/") {
                // "a/<old> b/<new>": right for every path without " b/" in it.
                if let Some((_, new)) = header.split_once(" b/") {
                    path = new.to_owned();
                }
            } else if line.starts_with("new file mode") {
                status = FileStatus::Added;
            } else if line.starts_with("deleted file mode") {
                status = FileStatus::Deleted;
            } else if let Some(from) = line.strip_prefix("rename from ") {
                status = FileStatus::Renamed;
                old_path = Some(from.to_owned());
            } else if let Some(to) = line.strip_prefix("rename to ") {
                path = to.to_owned();
            }
        }

        Self {
            path,
            old_path,
            status,
            additions,
            deletions,
            text: text.to_owned(),
            added_lines,
        }
    }

    /// Splits a whole patch into its files, in the patch's order.
    pub fn parse_all(patch: &str) -> Vec<Self> {
        let mut starts: Vec<usize> = patch
            .match_indices("\ndiff --git ")
            .map(|(at, _)| at + 1)
            .collect();
        if patch.starts_with("diff --git ") {
            starts.insert(0, 0);
        }
        starts
            .iter()
            .enumerate()
            .map(|(index, &start)| {
                let end = starts.get(index + 1).copied().unwrap_or(patch.len());
                Self::parse(&patch[start..end])
            })
            .collect()
    }
}
