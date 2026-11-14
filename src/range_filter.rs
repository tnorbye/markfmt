// Copyright 2026 Tor Norbye
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::HashMap;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::GitChangeMode;

#[derive(Debug, Clone, Default)]
pub enum RangeFilter {
    #[default]
    All,
    LineRanges(HashMap<PathBuf, Vec<RangeInclusive<usize>>>),
}

impl RangeFilter {
    pub fn includes_file(&self, canonical_path: &Path) -> bool {
        match self {
            RangeFilter::All => true,
            RangeFilter::LineRanges(map) => map
                .get(canonical_path)
                .map(|r| !r.is_empty())
                .unwrap_or(false),
        }
    }

    pub fn overlaps(&self, canonical_path: &Path, start_line: usize, end_line: usize) -> bool {
        match self {
            RangeFilter::All => true,
            RangeFilter::LineRanges(map) => {
                let Some(ranges) = map.get(canonical_path) else {
                    return false;
                };
                ranges
                    .iter()
                    .any(|r| *r.start() <= end_line && *r.end() >= start_line)
            }
        }
    }

    pub fn is_filtered(&self) -> bool {
        matches!(self, RangeFilter::LineRanges(_))
    }

    pub fn from_line_strings(
        canonical_file: &Path,
        range_strings: &[String],
    ) -> Result<Self, String> {
        let mut ranges = Vec::new();
        for raw in range_strings {
            for part in raw.split(',') {
                let s = part.trim();
                if s.is_empty() {
                    continue;
                }
                if let Some((a, b)) = s.split_once(':') {
                    let start: usize = a
                        .trim()
                        .parse()
                        .map_err(|_| format!("Line `{a}` is not a valid positive integer"))?;
                    let end: usize = b
                        .trim()
                        .parse()
                        .map_err(|_| format!("Line `{b}` is not a valid positive integer"))?;
                    if start == 0 || end == 0 {
                        return Err("Line numbers are 1-based and must be >= 1".to_string());
                    }
                    if start > end {
                        return Err(format!("Invalid line range `{s}`: start > end"));
                    }
                    ranges.push(start..=end);
                } else {
                    let line: usize = s
                        .parse()
                        .map_err(|_| format!("Line `{s}` is not a valid positive integer"))?;
                    if line == 0 {
                        return Err("Line numbers are 1-based and must be >= 1".to_string());
                    }
                    ranges.push(line..=line);
                }
            }
        }

        let mut map = HashMap::new();
        if !ranges.is_empty() {
            map.insert(canonical_file.to_path_buf(), ranges);
        }
        Ok(RangeFilter::LineRanges(map))
    }

    pub fn from_git_changes(
        git_path: Option<&str>,
        anchor_path: &Path,
        modes: &[GitChangeMode],
    ) -> Result<Self, String> {
        let git_bin = find_git(git_path)?;
        let anchor_dir = if anchor_path.is_dir() {
            anchor_path.to_path_buf()
        } else {
            anchor_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        };
        let repo_root = find_git_root(&git_bin, &anchor_dir)?;
        let unborn = (modes.contains(&GitChangeMode::Uncommitted)
            || modes.contains(&GitChangeMode::Head))
            && !has_head(&git_bin, &repo_root)?;

        let mut combined: HashMap<PathBuf, Vec<RangeInclusive<usize>>> = HashMap::new();
        for &mode in modes {
            let diff = if unborn && mode == GitChangeMode::Uncommitted {
                String::new()
            } else if unborn && mode == GitChangeMode::Head {
                return Err("Repository has no commits yet (`HEAD` does not exist)".to_string());
            } else {
                run_git_diff(&git_bin, &repo_root, mode)?
            };
            let parsed = parse_unified_diff(&repo_root, &diff);
            for (path, mut ranges) in parsed {
                combined.entry(path).or_default().append(&mut ranges);
            }
            if mode == GitChangeMode::Uncommitted {
                // Without a first commit, staged files are also entirely new. Read their
                // working-tree contents, including any edits made after staging.
                for path in list_new_files(&git_bin, &repo_root, unborn)? {
                    combined.entry(path).or_default().push(1..=usize::MAX);
                }
            }
        }

        Ok(RangeFilter::LineRanges(combined))
    }
}

fn find_git(git_path: Option<&str>) -> Result<PathBuf, String> {
    match git_path {
        Some(explicit) if Path::new(explicit).is_file() => Ok(PathBuf::from(explicit)),
        Some(explicit) => Err(format!("Configured git path `{explicit}` does not exist")),
        // Let the OS resolve `git` from PATH (including PATHEXT on Windows).
        None => Ok(PathBuf::from("git")),
    }
}

/// Formats a failure to spawn git, with a hint when it is not installed.
fn git_spawn_error(err: std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::NotFound {
        "Could not find `git` executable on PATH (use --git-path)".to_string()
    } else {
        format!("Failed to invoke git: {err}")
    }
}

/// Converts a Windows verbatim path from `fs::canonicalize` (`\\?\C:\repo`,
/// `\\?\UNC\server\share\repo`) into the ordinary form git understands.
fn path_for_git_arg(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(s) = path.to_str() {
            return PathBuf::from(strip_verbatim_prefix(s));
        }
    }
    path.to_path_buf()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn strip_verbatim_prefix(s: &str) -> String {
    if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = s.strip_prefix(r"\\?\") {
        local.to_string()
    } else {
        s.to_string()
    }
}

fn find_git_root(git_bin: &Path, start_dir: &Path) -> Result<PathBuf, String> {
    let abs_dir = fs::canonicalize(start_dir).unwrap_or_else(|_| start_dir.to_path_buf());
    let output = Command::new(git_bin)
        .arg("-C")
        .arg(path_for_git_arg(&abs_dir))
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .map_err(git_spawn_error)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Not inside a git repository ({}): {}",
            abs_dir.display(),
            stderr.trim()
        ));
    }

    let root_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let root = PathBuf::from(root_str);
    Ok(fs::canonicalize(&root).unwrap_or(root))
}

fn git_command(git_bin: &Path, repo_root: &Path) -> Command {
    let mut cmd = Command::new(git_bin);
    cmd.arg("-C")
        .arg(path_for_git_arg(repo_root))
        // Emit non-ASCII paths verbatim instead of as quoted octal escapes.
        .args(["-c", "core.quotePath=false"])
        .arg("--no-pager");
    cmd
}

fn run_git(mut cmd: Command) -> Result<String, String> {
    let output = cmd.output().map_err(git_spawn_error)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git command failed: {}", stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git_diff(git_bin: &Path, repo_root: &Path, mode: GitChangeMode) -> Result<String, String> {
    let mut cmd = git_command(git_bin, repo_root);
    let diff_options = ["--no-color", "--no-prefix", "--unified=0"];
    match mode {
        // Index vs. HEAD.
        GitChangeMode::Staged => cmd.args(["diff", "--cached"]).args(diff_options),
        // The most recent commit.
        GitChangeMode::Head => cmd
            .args(["show", "--format="])
            .args(diff_options)
            .arg("HEAD"),
        // Working tree (staged or not) vs. HEAD.
        GitChangeMode::Uncommitted => cmd.arg("diff").args(diff_options).arg("HEAD"),
    };
    run_git(cmd)
}

fn has_head(git_bin: &Path, repo_root: &Path) -> Result<bool, String> {
    let output = git_command(git_bin, repo_root)
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .output()
        .map_err(git_spawn_error)?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

/// Untracked files that are not ignored, as absolute (canonical when possible) paths.
/// In an unborn repository, include staged files too.
fn list_new_files(
    git_bin: &Path,
    repo_root: &Path,
    include_staged: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut cmd = git_command(git_bin, repo_root);
    cmd.args(["ls-files", "--others", "--exclude-standard", "-z"]);
    if include_staged {
        cmd.arg("--cached");
    }
    Ok(run_git(cmd)?
        .split('\0')
        .filter(|rel| !rel.is_empty())
        .map(|rel| {
            let joined = repo_root.join(rel);
            fs::canonicalize(&joined).unwrap_or(joined)
        })
        .collect())
}

pub fn parse_unified_diff(
    repo_root: &Path,
    diff: &str,
) -> HashMap<PathBuf, Vec<RangeInclusive<usize>>> {
    let mut map: HashMap<PathBuf, Vec<RangeInclusive<usize>>> = HashMap::new();
    let mut current_path: Option<PathBuf> = None;
    let mut remaining = (0usize, 0usize);

    for line in diff.lines() {
        // Hunk contents can look exactly like file headers (`+++ filename`). Count old/new
        // lines so only headers outside hunks can change the current path.
        if remaining != (0, 0) {
            match line.as_bytes().first() {
                Some(b'+') => remaining.1 = remaining.1.saturating_sub(1),
                Some(b'-') => remaining.0 = remaining.0.saturating_sub(1),
                Some(b' ') => {
                    remaining.0 = remaining.0.saturating_sub(1);
                    remaining.1 = remaining.1.saturating_sub(1);
                }
                _ => {}
            }
            continue;
        }
        if let Some(raw_rel) = line.strip_prefix("+++ ") {
            let trimmed = raw_rel.trim();
            if trimmed == "/dev/null" {
                current_path = None;
            } else {
                let unquoted = unquote_git_path(trimmed);
                let joined = repo_root.join(unquoted);
                current_path = Some(fs::canonicalize(&joined).unwrap_or(joined));
            }
        } else if line.starts_with("@@ ") {
            let Some(ref path) = current_path else {
                continue;
            };
            if let Some((start, count)) = parse_hunk_range(line, '+') {
                remaining = (
                    parse_hunk_range(line, '-').map_or(0, |(_, count)| count),
                    count,
                );
                let ranges = map.entry(path.clone()).or_default();
                if count > 0 {
                    ranges.push(start..=start.saturating_add(count - 1));
                } else {
                    // A deletion touches the boundary between the surviving lines. Selecting
                    // both sides also covers deleting a blank line that joins two paragraphs.
                    ranges.push(start.max(1)..=start.saturating_add(1).max(1));
                }
            }
        }
    }

    map
}

fn parse_hunk_range(line: &str, sign: char) -> Option<(usize, usize)> {
    let plus_pos = line.find(sign)?;
    let after_plus = &line[plus_pos + 1..];
    let end_pos = after_plus.find(' ')?;
    let spec = &after_plus[..end_pos];
    if let Some((s, c)) = spec.split_once(',') {
        let start = s.parse::<usize>().ok()?;
        let count = c.parse::<usize>().ok()?;
        Some((start, count))
    } else {
        let start = spec.parse::<usize>().ok()?;
        Some((start, 1))
    }
}

/// Decodes a C-style quoted path as emitted by git (`"caf\303\251.md"`), including octal
/// byte escapes, which git still uses for control characters even with `core.quotePath=false`.
fn unquote_git_path(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) else {
        return s.to_string();
    };
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let esc = bytes[i + 1];
        i += 2;
        match esc {
            b'0'..=b'7' => {
                let mut value = u32::from(esc - b'0');
                let mut digits = 1;
                while digits < 3 && i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                    value = value * 8 + u32::from(bytes[i] - b'0');
                    i += 1;
                    digits += 1;
                }
                out.push(value as u8);
            }
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(0x07),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'v' => out.push(0x0b),
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{strip_verbatim_prefix, unquote_git_path};

    #[test]
    fn strips_windows_verbatim_prefixes() {
        assert_eq!(strip_verbatim_prefix(r"\\?\C:\repo"), r"C:\repo");
        assert_eq!(
            strip_verbatim_prefix(r"\\?\UNC\server\share\repo"),
            r"\\server\share\repo"
        );
        assert_eq!(strip_verbatim_prefix(r"C:\repo"), r"C:\repo");
        assert_eq!(strip_verbatim_prefix("/home/repo"), "/home/repo");
    }

    #[test]
    fn unquotes_git_paths() {
        assert_eq!(unquote_git_path("docs/plain.md"), "docs/plain.md");
        assert_eq!(unquote_git_path(r#""caf\303\251.md""#), "café.md");
        assert_eq!(unquote_git_path(r#""a\"b\\c\tд.md""#), "a\"b\\c\tд.md");
    }
}
