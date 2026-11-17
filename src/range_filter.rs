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

use similar::{capture_diff_slices, Algorithm, DiffOp};

use crate::cli::GitChangeMode;

#[derive(Debug, Clone, Default)]
pub enum RangeFilter {
    #[default]
    All,
    LineRanges(HashMap<PathBuf, Vec<RangeInclusive<usize>>>),
    /// Blocks must overlap both filters (e.g. `--lines` combined with git-change filtering).
    Both(Box<RangeFilter>, Box<RangeFilter>),
}

impl RangeFilter {
    pub fn includes_file(&self, canonical_path: &Path) -> bool {
        match self {
            RangeFilter::All => true,
            RangeFilter::LineRanges(map) => map
                .get(canonical_path)
                .map(|r| !r.is_empty())
                .unwrap_or(false),
            RangeFilter::Both(a, b) => {
                a.includes_file(canonical_path) && b.includes_file(canonical_path)
            }
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
            RangeFilter::Both(a, b) => {
                a.overlaps(canonical_path, start_line, end_line)
                    && b.overlaps(canonical_path, start_line, end_line)
            }
        }
    }

    pub fn is_filtered(&self) -> bool {
        !matches!(self, RangeFilter::All)
    }

    pub fn intersect(self, other: Self) -> Self {
        match (self, other) {
            (RangeFilter::All, other) | (other, RangeFilter::All) => other,
            (a, b) => RangeFilter::Both(Box::new(a), Box::new(b)),
        }
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

    /// Line ranges touched by git changes in `modes`, in working-tree coordinates, except for
    /// `stdin` (its filter path and contents), whose ranges are matched to its contents.
    pub fn from_git_changes(
        git_path: Option<&str>,
        anchor_path: &Path,
        modes: &[GitChangeMode],
        stdin: Option<(&Path, &str)>,
    ) -> Result<Self, String> {
        let git_bin = find_git(git_path)?;
        // The nearest folder that exists: a virtual stdin path (or a deleted file) may name
        // folders that are missing from the working tree.
        let anchor_dir = anchor_path
            .ancestors()
            .find(|dir| !dir.as_os_str().is_empty() && dir.is_dir())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let repo_root = find_git_root(&git_bin, &anchor_dir)?;
        let needs_head = modes.iter().any(|m| {
            matches!(m, GitChangeMode::Uncommitted)
                || matches!(m, GitChangeMode::Commit(r) if r == "HEAD")
        });
        let unborn = needs_head && !has_head(&git_bin, &repo_root)?;

        let mut combined: HashMap<PathBuf, Vec<RangeInclusive<usize>>> = HashMap::new();
        for mode in modes {
            let (diff, base) = match mode {
                GitChangeMode::Uncommitted if unborn => {
                    (String::new(), LineBase::Worktree { baseline: None })
                }
                GitChangeMode::Commit(rev) if unborn && rev == "HEAD" => {
                    return Err("Repository has no commits yet (`HEAD` does not exist)".to_string());
                }
                _ => run_git_diff(&git_bin, &repo_root, mode)?,
            };
            let diff_files = parse_diff(&repo_root, &diff);
            let mut parsed = new_side_ranges(&diff_files);
            match &base {
                LineBase::Worktree { baseline } => {
                    if let Some((path, text)) = stdin {
                        // Stdin IS the working-tree buffer for `path`: compare its contents
                        // directly against `HEAD` / `<rev>` instead of the file on disk.
                        parsed.remove(path);
                        if path.starts_with(&repo_root) {
                            let base_path = diff_files
                                .iter()
                                .find(|file| file.new.as_deref() == Some(path))
                                .and_then(|file| file.old.clone())
                                .unwrap_or_else(|| path.to_path_buf());
                            let ranges = match baseline {
                                None => std::iter::once(1..=usize::MAX).collect(),
                                Some(oid) => match base_contents(
                                    &git_bin,
                                    &repo_root,
                                    &LineBase::Commit(oid.clone()),
                                    &base_path,
                                )? {
                                    Some(old) => diff_text_ranges(&old, text),
                                    None => std::iter::once(1..=usize::MAX).collect(),
                                },
                            };
                            if !ranges.is_empty() {
                                parsed.insert(path.to_path_buf(), ranges);
                            }
                        }
                    }
                }
                LineBase::Index | LineBase::Commit(_) => {
                    // The ranges number lines in the index or in a commit; the formatter reads
                    // the working tree, so they are shifted past any edits (and renames) made
                    // since.
                    let mut cmd = git_command(&git_bin, &repo_root);
                    cmd.arg("diff").args(DIFF_OPTIONS);
                    if let LineBase::Commit(oid) = &base {
                        cmd.arg(oid);
                    }
                    cmd.arg("--");
                    let to_worktree = parse_diff(&repo_root, &run_git(cmd)?);
                    // Stdin is not the working-tree file, so its ranges are instead moved by
                    // comparing the version they number lines in (under its name there, if
                    // renamed since) with stdin's actual contents.
                    let stdin_ranges = stdin.and_then(|(path, _)| {
                        let base_path = to_worktree
                            .iter()
                            .find(|file| file.new.as_deref() == Some(path))
                            .and_then(|file| file.old.clone())
                            .unwrap_or_else(|| path.to_path_buf());
                        // Cloned, not removed: a file on disk may still need the same ranges
                        // (stdin named after a file that was since renamed on disk).
                        parsed
                            .get(&base_path)
                            .cloned()
                            .map(|ranges| (base_path, ranges))
                    });
                    parsed = remap_to_worktree(parsed, &to_worktree, |old_path| {
                        base_contents(&git_bin, &repo_root, &base, old_path)
                    })?;
                    if let Some((path, text)) = stdin {
                        // Disk coordinates do not apply to the buffer: replace them.
                        parsed.remove(path);
                        if let Some((base_path, ranges)) = stdin_ranges {
                            let ranges =
                                match base_contents(&git_bin, &repo_root, &base, &base_path)? {
                                    Some(old) => remap_by_words(&old, text, &ranges),
                                    None => ranges,
                                };
                            if !ranges.is_empty() {
                                parsed.insert(path.to_path_buf(), ranges);
                            }
                        }
                    }
                }
            }
            for (path, mut ranges) in parsed {
                combined.entry(path).or_default().append(&mut ranges);
            }
            match mode {
                GitChangeMode::Uncommitted => {
                    // Without a first commit, staged files are also entirely new. Read their
                    // working-tree contents, including any edits made after staging.
                    for path in list_new_files(&git_bin, &repo_root, unborn)? {
                        if stdin.is_some_and(|(sp, _)| sp == path) {
                            continue;
                        }
                        combined.entry(path).or_default().push(1..=usize::MAX);
                    }
                }
                GitChangeMode::Since(_) => {
                    for path in list_new_files(&git_bin, &repo_root, false)? {
                        if stdin.is_some_and(|(sp, _)| sp == path) {
                            continue;
                        }
                        combined.entry(path).or_default().push(1..=usize::MAX);
                    }
                }
                GitChangeMode::Staged | GitChangeMode::Commit(_) | GitChangeMode::Range { .. } => {}
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

/// Environment variables that locate the repository. Git resolves relative values against its
/// own working directory, which `-C <dir>` changes, so they are passed on as absolute paths.
const GIT_LOCATION_VARS: [&str; 3] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// The repository location variables markfmt was started with, resolved against its own
/// working directory.
#[derive(Default)]
struct GitLocationEnv {
    /// Absolute values for every variable that is set, including a `GIT_WORK_TREE` filled in
    /// for a `GIT_DIR` without one (Git would otherwise use the `-C` folder as the worktree).
    vars: Vec<(&'static str, PathBuf)>,
    /// The worktree and repository folder those variables select from markfmt's working
    /// directory (canonical), if Git finds them; a bare repository has no worktree. Folders
    /// outside both belong to another repository, which the variables must not redirect.
    worktree: Option<PathBuf>,
    git_dir: Option<PathBuf>,
}

fn git_location_env(git_bin: &Path) -> &'static GitLocationEnv {
    static ENV: std::sync::OnceLock<GitLocationEnv> = std::sync::OnceLock::new();
    ENV.get_or_init(|| {
        let Ok(cwd) = std::env::current_dir() else {
            return GitLocationEnv::default();
        };
        let mut vars = Vec::new();
        for name in GIT_LOCATION_VARS {
            if let Some(value) = std::env::var_os(name).filter(|v| !v.is_empty()) {
                vars.push((name, cwd.join(value)));
            }
        }
        if vars.is_empty() {
            return GitLocationEnv::default();
        }
        // Run from markfmt's own directory, so Git resolves the variables as the user meant.
        let rev_parse = |arg: &str| {
            Command::new(git_bin)
                .args(["rev-parse", arg])
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| {
                    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim_end());
                    fs::canonicalize(&path).unwrap_or(path)
                })
        };
        let worktree = rev_parse("--show-toplevel");
        let git_dir = rev_parse("--absolute-git-dir");
        if let Some(worktree) = &worktree {
            if vars.iter().any(|(name, _)| *name == "GIT_DIR")
                && !vars.iter().any(|(name, _)| *name == "GIT_WORK_TREE")
            {
                vars.push(("GIT_WORK_TREE", worktree.clone()));
            }
        }
        GitLocationEnv {
            vars,
            worktree,
            git_dir,
        }
    })
}

/// Passes `GIT_DIR`, `GIT_WORK_TREE`, and `GIT_INDEX_FILE` to a git command run with `-C dir`
/// as absolute paths, or removes them when `dir` is in a different repository than the one
/// they select (such as a symlink target in another checkout, or a checkout run from inside a
/// bare repository).
pub fn apply_git_location_env(cmd: &mut Command, git_bin: &Path, dir: &Path) {
    let env = git_location_env(git_bin);
    if env.vars.is_empty() {
        return;
    }
    let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let known = [&env.worktree, &env.git_dir];
    let inside = known
        .iter()
        .any(|d| d.as_ref().is_some_and(|d| dir.starts_with(d)));
    // When Git finds neither, pass the variables on and let it report the problem.
    if !inside && known.iter().any(|d| d.is_some()) {
        for name in GIT_LOCATION_VARS {
            cmd.env_remove(name);
        }
        return;
    }
    for (name, value) in &env.vars {
        // Canonical paths are verbatim (`\\?\C:\...`) on Windows, which Git does not accept.
        cmd.env(name, path_for_git_arg(value));
    }
}

fn find_git_root(git_bin: &Path, start_dir: &Path) -> Result<PathBuf, String> {
    let abs_dir = fs::canonicalize(start_dir).unwrap_or_else(|_| start_dir.to_path_buf());
    let mut cmd = Command::new(git_bin);
    apply_git_location_env(&mut cmd, git_bin, &abs_dir);
    let output = cmd
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
    apply_git_location_env(&mut cmd, git_bin, repo_root);
    // `GIT_DIFF_OPTS` (such as `--unified=3`) overrides the context options markfmt parses.
    cmd.env_remove("GIT_DIFF_OPTS");
    cmd.arg("-C")
        .arg(path_for_git_arg(repo_root))
        // Emit non-ASCII paths verbatim instead of as quoted octal escapes.
        .args(["-c", "core.quotePath=false"])
        // Follow renames, but never report copies (which would list one source twice), whatever
        // the user's `diff.renames` says.
        .args(["-c", "diff.renames=true"])
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

/// Options for every diff markfmt parses. External diff drivers, textconv filters, and merged
/// nearby hunks (`diff.interHunkContext`) would change the output or the lines it selects, so
/// configuration can never enable them.
const DIFF_OPTIONS: [&str; 6] = [
    "--no-color",
    "--no-prefix",
    "--unified=0",
    "--inter-hunk-context=0",
    "--no-ext-diff",
    "--no-textconv",
];

/// Resolves `rev` to a commit object ID, or `None` if it does not name a commit.
fn try_resolve_commit(
    git_bin: &Path,
    repo_root: &Path,
    rev: &str,
) -> Result<Option<String>, String> {
    // Never let a revision be read as an option. (`--end-of-options` would also do this, but
    // `git rev-parse` only understands it in recent Git versions.)
    if rev.is_empty() || rev.starts_with('-') || rev.contains('\0') {
        return Err(format!("Invalid git revision `{rev}`"));
    }
    let output = git_command(git_bin, repo_root)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{rev}^{{commit}}"))
        .output()
        .map_err(git_spawn_error)?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        )),
        Some(1) if output.stderr.is_empty() => Ok(None),
        // Out-of-range reflog entries (`HEAD@{99999}`) fail without a message.
        _ if output.stderr.trim_ascii().is_empty() => Err(format!("Unknown git revision `{rev}`")),
        _ => Err(format!(
            "Cannot resolve git revision `{rev}`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

fn resolve_commit(git_bin: &Path, repo_root: &Path, rev: &str) -> Result<String, String> {
    try_resolve_commit(git_bin, repo_root, rev)?
        .ok_or_else(|| format!("Unknown git revision `{rev}`"))
}

/// The version of a file whose lines a diff's new side numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LineBase {
    Worktree { baseline: Option<String> },
    Index,
    Commit(String),
}

fn run_git_diff(
    git_bin: &Path,
    repo_root: &Path,
    mode: &GitChangeMode,
) -> Result<(String, LineBase), String> {
    let mut cmd = git_command(git_bin, repo_root);
    // Every revision list ends with `--`, so a file named like a revision is never ambiguous.
    let base = match mode {
        // Index vs. HEAD.
        GitChangeMode::Staged => {
            cmd.args(["diff", "--cached"]).args(DIFF_OPTIONS).arg("--");
            LineBase::Index
        }
        // Working tree (staged or not) vs. HEAD.
        GitChangeMode::Uncommitted => {
            cmd.arg("diff").args(DIFF_OPTIONS).args(["HEAD", "--"]);
            LineBase::Worktree {
                baseline: Some("HEAD".to_string()),
            }
        }
        // Working tree (staged or not) vs. `<rev>`.
        GitChangeMode::Since(rev) => {
            let oid = resolve_commit(git_bin, repo_root, rev)?;
            cmd.arg("diff").args(DIFF_OPTIONS).arg(&oid).arg("--");
            LineBase::Worktree {
                baseline: Some(oid),
            }
        }
        // Changes introduced by a single commit. Merges are diffed against their first parent:
        // `git show` would print a combined diff, which only lists conflict resolutions.
        GitChangeMode::Commit(rev) => {
            let oid = resolve_commit(git_bin, repo_root, rev)?;
            match try_resolve_commit(git_bin, repo_root, &format!("{oid}^1"))? {
                Some(parent) => {
                    cmd.arg("diff")
                        .args(DIFF_OPTIONS)
                        .arg(parent)
                        .arg(&oid)
                        .arg("--");
                }
                // A root commit: everything in it is new (`--root` overrides `log.showRoot`).
                None => {
                    cmd.args(["show", "--format=", "--root"])
                        .args(DIFF_OPTIONS)
                        .arg(&oid)
                        .arg("--");
                }
            }
            LineBase::Commit(oid)
        }
        // Committed changes in `<base>..<head>` (endpoint diff) or `<base>...<head>` (since
        // the merge base).
        GitChangeMode::Range {
            base,
            head,
            merge_base,
        } => {
            let base_oid = resolve_commit(git_bin, repo_root, base)?;
            let head_oid = resolve_commit(git_bin, repo_root, head)?;
            let sep = if *merge_base { "..." } else { ".." };
            cmd.arg("diff")
                .args(DIFF_OPTIONS)
                .arg(format!("{base_oid}{sep}{head_oid}"))
                .arg("--");
            LineBase::Commit(head_oid)
        }
    };
    Ok((run_git(cmd)?, base))
}

/// The contents of `path` in `base`, or `None` if it does not exist there.
fn base_contents(
    git_bin: &Path,
    repo_root: &Path,
    base: &LineBase,
    path: &Path,
) -> Result<Option<String>, String> {
    if matches!(base, LineBase::Worktree { .. }) {
        return Ok(fs::read_to_string(path).ok());
    }
    let Ok(rel) = path.strip_prefix(repo_root) else {
        return Ok(None);
    };
    let rel: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let spec = match base {
        LineBase::Commit(oid) => format!("{oid}:{}", rel.join("/")),
        _ => format!(":{}", rel.join("/")),
    };
    let output = git_command(git_bin, repo_root)
        .args(["cat-file", "blob"])
        .arg(spec)
        .output()
        .map_err(git_spawn_error)?;
    Ok(output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned()))
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
        .map(|rel| fs_path(repo_root, rel))
        .collect())
}

/// One `@@ -old_start,old_count +new_start,new_count @@` hunk, with its removed and added
/// lines (without the `-`/`+` prefix).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

/// One file in a unified diff. `None` is `/dev/null` (an added or deleted file).
#[derive(Debug, Default)]
struct FileDiff {
    old: Option<PathBuf>,
    new: Option<PathBuf>,
    hunks: Vec<Hunk>,
}

/// The filesystem path for a repository-relative path from git, canonical (as formatting jobs
/// are) unless it is a symlink. Git tracks a symlink's target text, not the document it points
/// to, so its changes must never select lines in that document.
fn fs_path(repo_root: &Path, rel: &str) -> PathBuf {
    let joined = repo_root.join(rel);
    if fs::symlink_metadata(&joined).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return joined;
    }
    fs::canonicalize(&joined).unwrap_or(joined)
}

/// A path from a `---`/`+++`/`rename` header. Names are kept exactly (they can start or end
/// with spaces), except for the tab git appends to names that contain a space.
fn diff_path(repo_root: &Path, raw: &str) -> Option<PathBuf> {
    let name = raw.strip_suffix('\t').unwrap_or(raw);
    if name == "/dev/null" {
        return None;
    }
    Some(fs_path(repo_root, &unquote_git_path(name)))
}

/// Parses `git diff --no-prefix --unified=0` output.
fn parse_diff(repo_root: &Path, diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut remaining = (0usize, 0usize);

    for line in diff.lines() {
        // Hunk contents can look exactly like file headers (`+++ filename`). Count old/new
        // lines so only headers outside hunks can change the current file.
        if remaining != (0, 0) {
            let hunk = files.last_mut().and_then(|file| file.hunks.last_mut());
            match line.as_bytes().first() {
                Some(b'+') => {
                    remaining.1 = remaining.1.saturating_sub(1);
                    if let Some(hunk) = hunk {
                        hunk.new_lines.push(line[1..].to_string());
                    }
                }
                Some(b'-') => {
                    remaining.0 = remaining.0.saturating_sub(1);
                    if let Some(hunk) = hunk {
                        hunk.old_lines.push(line[1..].to_string());
                    }
                }
                Some(b' ') => {
                    remaining.0 = remaining.0.saturating_sub(1);
                    remaining.1 = remaining.1.saturating_sub(1);
                }
                _ => {}
            }
            continue;
        }
        if line.starts_with("diff ") {
            files.push(FileDiff::default());
        } else if let Some(raw) = line
            .strip_prefix("--- ")
            .or_else(|| line.strip_prefix("rename from "))
        {
            if files.last().is_none_or(|f| !f.hunks.is_empty()) {
                files.push(FileDiff::default());
            }
            if let Some(file) = files.last_mut() {
                file.old = diff_path(repo_root, raw);
            }
        } else if let Some(raw) = line
            .strip_prefix("+++ ")
            .or_else(|| line.strip_prefix("rename to "))
        {
            if files.last().is_none_or(|f| !f.hunks.is_empty()) {
                files.push(FileDiff::default());
            }
            if let Some(file) = files.last_mut() {
                file.new = diff_path(repo_root, raw);
            }
        } else if line.starts_with("@@ ") {
            let (Some(file), Some((new_start, new_count))) =
                (files.last_mut(), parse_hunk_range(line, '+'))
            else {
                continue;
            };
            let (old_start, old_count) = parse_hunk_range(line, '-').unwrap_or((0, 0));
            remaining = (old_count, new_count);
            file.hunks.push(Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
                ..Hunk::default()
            });
        }
    }

    files
}

/// Parses `git diff --no-prefix --unified=0` output into the new-side lines each file's hunks
/// touch.
pub fn parse_unified_diff(
    repo_root: &Path,
    diff: &str,
) -> HashMap<PathBuf, Vec<RangeInclusive<usize>>> {
    new_side_ranges(&parse_diff(repo_root, diff))
}

/// The new-side lines each hunk touches, by file.
fn new_side_ranges(files: &[FileDiff]) -> HashMap<PathBuf, Vec<RangeInclusive<usize>>> {
    let mut map: HashMap<PathBuf, Vec<RangeInclusive<usize>>> = HashMap::new();
    for file in files {
        let Some(path) = &file.new else { continue };
        if file.hunks.is_empty() {
            continue;
        }
        let ranges = map.entry(path.clone()).or_default();
        for hunk in &file.hunks {
            ranges.push(new_side_range(hunk));
        }
    }
    map
}

fn new_side_range(hunk: &Hunk) -> RangeInclusive<usize> {
    let start = hunk.new_start;
    if hunk.new_count > 0 {
        start..=start.saturating_add(hunk.new_count - 1)
    } else {
        // A deletion touches the boundary between the surviving lines. Selecting both sides
        // also covers deleting a blank line that joins two paragraphs.
        start.max(1)..=start.saturating_add(1).max(1)
    }
}

/// Line ranges in `new` that differ from `old`, comparing lines with `\r\n` normalized to `\n` so
/// CRLF/LF editor-buffer differences alone do not mark untouched paragraphs as changed.
fn diff_text_ranges(old: &str, new: &str) -> Vec<RangeInclusive<usize>> {
    fn split_lines(text: &str) -> Vec<String> {
        text.split_inclusive('\n')
            .map(|line| match line.strip_suffix("\r\n") {
                Some(stem) => format!("{stem}\n"),
                None => line.to_string(),
            })
            .collect()
    }
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let mut out = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &old_lines, &new_lines) {
        let hunk = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete { new_index, .. } => Hunk {
                new_start: new_index,
                new_count: 0,
                ..Hunk::default()
            },
            DiffOp::Insert {
                new_index, new_len, ..
            }
            | DiffOp::Replace {
                new_index, new_len, ..
            } => Hunk {
                new_start: new_index + 1,
                new_count: new_len,
                ..Hunk::default()
            },
        };
        out.push(new_side_range(&hunk));
    }
    out
}

/// Moves old-side line `line` through `hunks` (sorted, from one file's diff). Lines inside a
/// changed hunk map to the lines that replaced them; `at_end` picks that range's last line.
/// Inside a deletion, a range start moves to the first surviving line after it and a range end
/// to the last surviving line before it, so a fully deleted range ends before it starts.
fn remap_line(line: usize, hunks: &[Hunk], at_end: bool) -> usize {
    if line == usize::MAX {
        return line;
    }
    let mut delta: isize = 0;
    for hunk in hunks {
        if hunk.old_count == 0 {
            // Pure insertion after old line `old_start`.
            if line <= hunk.old_start {
                break;
            }
            delta += hunk.new_count as isize;
            continue;
        }
        if line < hunk.old_start {
            break;
        }
        if line < hunk.old_start + hunk.old_count {
            // With `--unified=0`, a deletion's `new_start` is the surviving line before it.
            return match (hunk.new_count, at_end) {
                (0, false) => hunk.new_start + 1,
                (0, true) => hunk.new_start,
                (_, false) => hunk.new_start,
                (count, true) => hunk.new_start + count - 1,
            };
        }
        delta += hunk.new_count as isize - hunk.old_count as isize;
    }
    line.saturating_add_signed(delta).max(1)
}

/// Moves old-side line ranges through `hunks` (sorted, from one file's diff). A range is split
/// around lines inserted inside it, so it never selects blocks that were not part of it, and
/// lines deleted since disappear. Selected lines inside a changed hunk map only to the lines
/// that replaced them (see `replaced_lines`).
fn remap_ranges(
    ranges: impl IntoIterator<Item = RangeInclusive<usize>>,
    hunks: &[Hunk],
) -> Vec<RangeInclusive<usize>> {
    fn shifted(out: &mut Vec<RangeInclusive<usize>>, first: usize, last: usize, delta: isize) {
        let first = first.saturating_add_signed(delta).max(1);
        let last = last.saturating_add_signed(delta).max(1);
        out.push(first..=last);
    }

    let mut out = Vec::new();
    for range in ranges {
        let (start, end) = (*range.start(), *range.end());
        if end == usize::MAX {
            // A whole new file stays whole.
            out.push(remap_line(start, hunks, false)..=end);
            continue;
        }
        // `cursor` is the first old line of the range not yet placed.
        let mut cursor = start;
        let mut delta: isize = 0;
        for hunk in hunks {
            if hunk.old_count == 0 {
                // Pure insertion after old line `old_start`.
                if hunk.old_start >= end {
                    break;
                }
                if hunk.old_start >= cursor {
                    shifted(&mut out, cursor, hunk.old_start, delta);
                    cursor = hunk.old_start + 1;
                }
                delta += hunk.new_count as isize;
                continue;
            }
            let (first_old, last_old) = (hunk.old_start, hunk.old_start + hunk.old_count - 1);
            if first_old > end {
                break;
            }
            if last_old >= cursor {
                if cursor < first_old {
                    shifted(&mut out, cursor, first_old - 1, delta);
                }
                out.extend(replaced_lines(
                    hunk,
                    cursor.max(first_old) - first_old,
                    end.min(last_old) - first_old,
                ));
                cursor = last_old + 1;
            }
            delta += hunk.new_count as isize - hunk.old_count as isize;
        }
        if cursor <= end {
            shifted(&mut out, cursor, end, delta);
        }
    }
    out
}

/// The new lines in `hunk` that replaced its old lines `first..=last` (0-based within the hunk).
///
/// They are found by matching words, so that whitespace and line-break changes (such as
/// formatting) keep a block selected, while lines added next to it in the same hunk are not
/// selected. Unmatched lines between matched ones are included unless a blank line separates
/// them (a separate paragraph). If no words match, the whole replacement is selected.
fn replaced_lines(hunk: &Hunk, first: usize, last: usize) -> Vec<RangeInclusive<usize>> {
    if hunk.new_count == 0 {
        return Vec::new();
    }
    let whole = std::iter::once(hunk.new_start..=hunk.new_start + hunk.new_count - 1).collect();
    if hunk.old_lines.len() != hunk.old_count || hunk.new_lines.len() != hunk.new_count {
        return whole;
    }
    let words = |lines: &[String]| -> (Vec<String>, Vec<usize>) {
        lines
            .iter()
            .enumerate()
            .flat_map(|(i, line)| line.split_whitespace().map(move |w| (w.to_string(), i)))
            .unzip()
    };
    let (old_words, old_line_of) = words(&hunk.old_lines);
    let (new_words, new_line_of) = words(&hunk.new_lines);
    let mut selected = vec![false; hunk.new_count];
    for op in capture_diff_slices(Algorithm::Myers, &old_words, &new_words) {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            for k in 0..len {
                let old_line = old_line_of[old_index + k];
                if (first..=last).contains(&old_line) {
                    selected[new_line_of[new_index + k]] = true;
                }
            }
        }
    }
    let marked: Vec<usize> = (0..hunk.new_count).filter(|&i| selected[i]).collect();
    if marked.is_empty() {
        return whole;
    }
    let is_blank = |i: usize| hunk.new_lines[i].trim().is_empty();
    let mut out: Vec<RangeInclusive<usize>> = Vec::new();
    let (mut run_start, mut run_end) = (marked[0], marked[0]);
    for &i in &marked[1..] {
        let gap = run_end + 1..i;
        let separate = !gap.is_empty() && gap.clone().any(is_blank) && !gap.clone().all(is_blank);
        if separate {
            out.push(hunk.new_start + run_start..=hunk.new_start + run_end);
            run_start = i;
        }
        run_end = i;
    }
    out.push(hunk.new_start + run_start..=hunk.new_start + run_end);
    out
}

/// Translates line ranges keyed by old-side paths of `to_worktree` into working-tree lines.
/// `old_contents` reads a file's version the ranges number lines in.
fn remap_to_worktree(
    ranges: HashMap<PathBuf, Vec<RangeInclusive<usize>>>,
    to_worktree: &[FileDiff],
    mut old_contents: impl FnMut(&Path) -> Result<Option<String>, String>,
) -> Result<HashMap<PathBuf, Vec<RangeInclusive<usize>>>, String> {
    let changes: HashMap<&Path, &FileDiff> = to_worktree
        .iter()
        .filter_map(|file| file.old.as_deref().map(|old| (old, file)))
        .collect();
    let mut out: HashMap<PathBuf, Vec<RangeInclusive<usize>>> = HashMap::new();
    for (path, file_ranges) in ranges {
        let Some(change) = changes.get(path.as_path()) else {
            out.entry(path).or_default().extend(file_ranges);
            continue;
        };
        // Deleted from the working tree: nothing left to format.
        let Some(new_path) = &change.new else {
            continue;
        };
        let mapped = match (old_contents(&path)?, fs::read_to_string(new_path).ok()) {
            (Some(old), Some(new)) => remap_by_words(&old, &new, &file_ranges),
            // Unreadable (such as non-UTF-8) contents: fall back to git's line hunks.
            _ => remap_ranges(file_ranges, &change.hunks),
        };
        out.entry(new_path.clone()).or_default().extend(mapped);
    }
    Ok(out)
}

/// Moves line ranges in `old` to the lines of `new` holding the same text.
///
/// Both texts are compared word by word, with each blank line as a marker, so the result
/// depends only on words and paragraph breaks. Formatting changes neither, so formatting the
/// selected blocks never moves the selection (even onto an identical copy of them), and text
/// added since is never selected. Selected words replaced since select their replacements,
/// paired paragraph by paragraph, so a paragraph added next to a rewritten one is not selected;
/// deleted words select nothing.
fn remap_by_words(
    old: &str,
    new: &str,
    ranges: &[RangeInclusive<usize>],
) -> Vec<RangeInclusive<usize>> {
    if ranges.iter().any(|range| *range.end() == usize::MAX) {
        // A whole new file stays whole.
        return std::iter::once(1..=usize::MAX).collect();
    }
    fn tokens(text: &str) -> (Vec<&str>, Vec<usize>) {
        let mut words = Vec::new();
        let mut lines = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                // Words are never empty, so `""` only ever matches another blank line.
                words.push("");
                lines.push(index + 1);
            }
            for word in line.split_whitespace() {
                words.push(word);
                lines.push(index + 1);
            }
        }
        (words, lines)
    }
    let (old_words, old_lines) = tokens(old);
    let (new_words, new_lines) = tokens(new);
    let selected = |index: usize| ranges.iter().any(|range| range.contains(&old_lines[index]));
    let mut marked = std::collections::BTreeSet::new();
    for op in capture_diff_slices(Algorithm::Myers, &old_words, &new_words) {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for k in 0..len {
                    if selected(old_index + k) {
                        marked.insert(new_lines[new_index + k]);
                    }
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                let old_paragraphs = paragraph_runs(&old_words, old_index..old_index + old_len);
                let new_paragraphs = paragraph_runs(&new_words, new_index..new_index + new_len);
                for (i, new_run) in new_paragraphs.iter().enumerate() {
                    // Paragraphs are paired in order. The last new one also stands for any
                    // further old ones (paragraphs joined since); new ones past the old count
                    // were added since and stay unselected.
                    let old_runs = if i + 1 == new_paragraphs.len() {
                        old_paragraphs.get(i..).unwrap_or_default()
                    } else {
                        old_paragraphs.get(i..=i).unwrap_or_default()
                    };
                    if old_runs.iter().flat_map(Clone::clone).any(selected) {
                        marked.extend(&new_lines[new_run.clone()]);
                    }
                }
            }
            DiffOp::Delete { .. } | DiffOp::Insert { .. } => {}
        }
    }
    let mut out: Vec<RangeInclusive<usize>> = Vec::new();
    for line in marked {
        match out.last_mut() {
            Some(last) if *last.end() + 1 == line => *last = *last.start()..=line,
            _ => out.push(line..=line),
        }
    }
    out
}

/// Splits the token indices in `span` into runs of words between blank-line markers (`""`),
/// dropping the markers and empty runs.
fn paragraph_runs(words: &[&str], span: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start = span.start;
    for index in span.clone() {
        if words[index].is_empty() {
            if start < index {
                runs.push(start..index);
            }
            start = index + 1;
        }
    }
    if start < span.end {
        runs.push(start..span.end);
    }
    runs
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
