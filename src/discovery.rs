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

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// File extensions (lowercase) discovered as Markdown. `.pre-commit-hooks.yaml` mirrors this.
pub const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn", "mdwn"];

/// Extensionless documentation names sniffed for Markdown. `.pre-commit-hooks.yaml` mirrors
/// this.
pub const EXTENSIONLESS_DOC_NAMES: &[&str] = &[
    "README",
    "CHANGELOG",
    "CONTRIBUTING",
    "HACKING",
    "SECURITY",
    "PULL_REQUEST_TEMPLATE",
    "RELEASE_NOTES",
    "NEWS",
];

const SKIP_BASENAME_PREFIXES: &[&str] = &["LICENSE", "COPYING", "NOTICE", "PATENTS"];

const VCS_DIRS: &[&str] = &[".git", ".hg", ".svn"];

const COMMIT_MESSAGE_NAMES: &[&str] = &[
    "COMMIT_EDITMSG",
    "MERGE_MSG",
    "SQUASH_MSG",
    "TAG_EDITMSG",
    "NOTES_EDITMSG",
    "EDIT_DESCRIPTION",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputTarget {
    Stdin,
    File {
        display_path: PathBuf,
        write_path: PathBuf,
    },
}

/// Files to format, plus non-fatal problems found while walking directories.
#[derive(Debug, Default)]
pub struct Discovered {
    pub targets: Vec<InputTarget>,
    /// Unreadable directories or entries that were skipped during traversal.
    pub warnings: Vec<String>,
    /// Reasons inputs were skipped, populated only by diagnostic discovery.
    pub explanations: Vec<String>,
}

/// Expands command-line inputs into format targets. Explicit paths that do not exist are an
/// error; problems inside directory walks are collected as warnings.
pub fn discover_targets(raw_inputs: &[String], no_ignore: bool) -> Result<Discovered, String> {
    discover_targets_with_options(
        raw_inputs,
        DiscoveryOptions {
            no_ignore,
            ..DiscoveryOptions::default()
        },
    )
}

/// Discovery switches beyond the inputs themselves.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscoveryOptions {
    /// Do not respect `.gitignore` and related ignore files when walking directories.
    pub no_ignore: bool,
    /// Apply Markdown discovery rules to explicitly named files too (`--filter-files`).
    pub filter_files: bool,
    /// Explain unsupported files, ignore rules, and duplicate paths. This also visits ignored
    /// directories so it can report their Markdown files.
    pub explain: bool,
}

/// Discovers inputs according to `options`, collecting warnings and optional explanations.
pub fn discover_targets_with_options(
    raw_inputs: &[String],
    options: DiscoveryOptions,
) -> Result<Discovered, String> {
    let DiscoveryOptions {
        no_ignore,
        filter_files,
        explain,
    } = options;
    let mut targets = Vec::new();
    let mut warnings = Vec::new();
    let mut explanations = Vec::new();
    let mut seen_canonical: HashSet<PathBuf> = HashSet::new();
    let mut saw_stdin = false;

    for raw in raw_inputs {
        if raw == "-" {
            if !saw_stdin {
                saw_stdin = true;
                targets.push(InputTarget::Stdin);
            }
            continue;
        }

        let path = Path::new(raw);
        let meta = fs::metadata(path).map_err(|e| format!("Cannot access `{raw}`: {e}"))?;

        if meta.is_file() {
            if filter_files && !is_eligible_walked_file(path) {
                if explain {
                    explanations.push(format!(
                        "Skipped {}: not a supported Markdown documentation file",
                        path.display()
                    ));
                }
                continue;
            }
            let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if seen_canonical.insert(canonical.clone()) {
                targets.push(InputTarget::File {
                    display_path: path.to_path_buf(),
                    write_path: canonical,
                });
            } else if explain {
                explanations.push(format!(
                    "Skipped {}: duplicate canonical target",
                    path.display()
                ));
            }
        } else if meta.is_dir() {
            let mut walked_paths = Vec::new();
            let mut builder = WalkBuilder::new(path);
            builder
                .hidden(false)
                .require_git(false)
                .git_ignore(!no_ignore)
                .git_global(!no_ignore)
                .git_exclude(!no_ignore)
                .ignore(!no_ignore)
                .parents(!no_ignore)
                .follow_links(true)
                .filter_entry(|entry| {
                    let name = entry.file_name().to_string_lossy();
                    if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                        && VCS_DIRS.contains(&name.as_ref())
                    {
                        return false;
                    }
                    name != ".DS_Store"
                });

            for result in builder.build() {
                let entry = match result {
                    Ok(e) => e,
                    Err(err) => {
                        warnings.push(err.to_string());
                        continue;
                    }
                };
                if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    continue;
                }
                let entry_path = entry.path();
                if is_eligible_walked_file(entry_path) {
                    walked_paths.push(entry_path.to_path_buf());
                } else if explain {
                    // Only files the walk actually visited; ignored ones (such as build output)
                    // would bury the useful lines.
                    explanations.push(format!(
                        "Skipped {}: not a supported Markdown documentation file",
                        entry_path.display()
                    ));
                }
            }

            if explain {
                let selected: HashSet<_> = walked_paths.iter().cloned().collect();
                builder
                    .git_ignore(false)
                    .git_global(false)
                    .git_exclude(false)
                    .ignore(false)
                    .parents(false);
                // Markdown files that ignore rules hid; other ignored files aren't worth listing.
                let mut ignored: Vec<PathBuf> = builder
                    .build()
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
                    .map(|entry| entry.into_path())
                    .filter(|p| !selected.contains(p) && is_eligible_walked_file(p))
                    .collect();
                ignored.sort();
                explanations.extend(explain_ignored(path, &ignored));
            }

            walked_paths.sort();
            for p in walked_paths {
                let canonical = match fs::canonicalize(&p) {
                    Ok(c) => c,
                    Err(err) => {
                        warnings.push(format!("Cannot resolve `{}`: {err}", p.display()));
                        continue;
                    }
                };
                if seen_canonical.insert(canonical.clone()) {
                    targets.push(InputTarget::File {
                        display_path: p,
                        write_path: canonical,
                    });
                } else if explain {
                    explanations.push(format!(
                        "Skipped {}: duplicate canonical target",
                        p.display()
                    ));
                }
            }
        }
    }

    explanations.sort();
    Ok(Discovered {
        targets,
        warnings,
        explanations,
    })
}

/// Files hidden by ignore rules, summarized per top-level directory when there are many (such
/// as vendored dependencies), so `--explain` stays readable.
fn explain_ignored(root: &Path, ignored: &[PathBuf]) -> Vec<String> {
    const MAX_LISTED_PER_DIRECTORY: usize = 5;
    const REASON: &str = "matched ignore rules (use --no-ignore to include)";
    let mut by_directory: Vec<(Option<PathBuf>, Vec<&PathBuf>)> = Vec::new();
    for p in ignored {
        let rel = p.strip_prefix(root).unwrap_or(p);
        let mut components = rel.components();
        let first = components.next().map(|c| root.join(c));
        // Files directly in the root have no directory to summarize.
        let directory = first.filter(|_| components.next().is_some());
        match by_directory.iter_mut().find(|(dir, _)| *dir == directory) {
            Some((_, files)) => files.push(p),
            None => by_directory.push((directory, vec![p])),
        }
    }
    let mut out = Vec::new();
    for (directory, files) in by_directory {
        match directory {
            Some(dir) if files.len() > MAX_LISTED_PER_DIRECTORY => out.push(format!(
                "Skipped {} Markdown files in {}: {REASON}",
                files.len(),
                dir.display()
            )),
            _ => out.extend(
                files
                    .iter()
                    .map(|p| format!("Skipped {}: {REASON}", p.display())),
            ),
        }
    }
    out
}

pub fn is_markdeep_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_ascii_lowercase().ends_with(".md.html"))
        .unwrap_or(false)
}

/// Whether `path` is a known Git or Jujutsu commit/tag/note/description edit file.
pub fn is_commit_message_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if COMMIT_MESSAGE_NAMES
        .iter()
        .any(|name| name.eq_ignore_ascii_case(file_name))
    {
        return true;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jjdescription"))
}

fn is_eligible_walked_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let upper_name = file_name.to_ascii_uppercase();
    if SKIP_BASENAME_PREFIXES
        .iter()
        .any(|prefix| upper_name.starts_with(prefix))
    {
        return false;
    }

    let lower_name = file_name.to_ascii_lowercase();
    if lower_name.ends_with(".md.html") {
        return true;
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lower_ext = ext.to_ascii_lowercase();
        return MARKDOWN_EXTENSIONS.contains(&lower_ext.as_str());
    }

    if EXTENSIONLESS_DOC_NAMES.contains(&upper_name.as_str()) {
        if let Ok(content) = fs::read_to_string(path) {
            return sniff_is_markdown(&content);
        }
    }

    false
}

/// Conservatively sniffs whether an extensionless doc file (e.g., `README`, `CHANGELOG`)
/// is Markdown rather than plain text, reStructuredText, AsciiDoc, or roff.
pub fn sniff_is_markdown(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.contains('\0') {
        return false;
    }

    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return false;
    }

    // Immediate rejections for rST, AsciiDoc, and roff.
    for (idx, &raw_line) in lines.iter().enumerate() {
        let trimmed = raw_line.trim();
        let trimmed_start = raw_line.trim_start();

        // rST directive: `.. note::`, `.. code-block::`, `.. image::`
        if let Some(rest) = trimmed_start.strip_prefix(".. ") {
            if let Some(colon_pos) = rest.find("::") {
                let name = &rest[..colon_pos];
                if !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    let after = &rest[colon_pos + 2..];
                    if after.is_empty() || after.starts_with(char::is_whitespace) {
                        return false;
                    }
                }
            }
        }

        // rST roles / field lists
        if raw_line.contains(":ref:`")
            || raw_line.contains(":func:`")
            || raw_line.contains(":class:`")
            || trimmed_start.starts_with(":param ")
            || trimmed_start.starts_with(":returns:")
            || trimmed_start.starts_with(":rtype:")
        {
            return false;
        }

        // rST literal block (`::` at end of line followed by blank line + indented block)
        if trimmed.ends_with("::") && idx + 2 < lines.len() && lines[idx + 1].trim().is_empty() {
            let next_non_empty = lines[idx + 2];
            if next_non_empty.starts_with(' ') || next_non_empty.starts_with('\t') {
                return false;
            }
        }

        // AsciiDoc headings & source attributes
        if (idx == 0 && trimmed_start.starts_with("= "))
            || trimmed_start.starts_with("== ")
            || trimmed_start.starts_with("=== ")
            || trimmed_start.starts_with("[source,")
            || trimmed == "[source]"
        {
            return false;
        }

        // roff / man page macros
        if trimmed_start.starts_with(".TH ")
            || trimmed_start.starts_with(".SH ")
            || trimmed == ".PP"
            || trimmed_start.starts_with(".PP ")
        {
            return false;
        }
    }

    // Check for unambiguous Markdown signals.
    if text.contains("markdeep.min.js") || text.contains("<!-- Markdeep:") {
        return true;
    }

    let mut fence_count = 0usize;
    for (idx, &raw_line) in lines.iter().enumerate() {
        let trimmed_start = raw_line.trim_start();

        // Fenced code block markers
        if trimmed_start.starts_with("```") {
            fence_count += 1;
            if fence_count >= 2 {
                return true;
            }
        }

        // ATX headings (`# Heading` .. `###### Heading`)
        if is_atx_heading_line(trimmed_start) {
            return true;
        }

        // Inline Markdown link or image `[label](dest)`
        if contains_markdown_link(raw_line) {
            return true;
        }

        // GFM pipe table (`| a | b |` followed by `| --- | --- |`)
        if idx + 1 < lines.len() && raw_line.contains('|') && is_gfm_table_divider(lines[idx + 1]) {
            return true;
        }
    }

    false
}

fn is_atx_heading_line(line: &str) -> bool {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return false;
    }
    let rest = &line[hashes..];
    if !rest.starts_with(' ') {
        return false;
    }
    let after_space = rest.trim();
    if after_space.is_empty() {
        return false;
    }
    // Exclude C preprocessor / shell shebang lookalikes if hashes == 1 (though `#include` has no space)
    true
}

fn contains_markdown_link(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(close_bracket) = line[i + 1..].find(']') {
                let after_bracket = i + 1 + close_bracket + 1;
                if after_bracket < bytes.len() && bytes[after_bracket] == b'(' {
                    if let Some(close_paren) = line[after_bracket + 1..].find(')') {
                        let dest_raw =
                            line[after_bracket + 1..after_bracket + 1 + close_paren].trim();
                        let dest = dest_raw
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_start_matches('<')
                            .trim_end_matches('>');
                        let lower = dest.to_ascii_lowercase();
                        if lower.starts_with("http://")
                            || lower.starts_with("https://")
                            || lower.starts_with("mailto:")
                            || lower.starts_with("file://")
                            || lower.starts_with("./")
                            || lower.starts_with("../")
                            || lower.starts_with('#')
                            || lower.ends_with(".md")
                            || lower.ends_with(".png")
                            || lower.ends_with(".jpg")
                            || lower.ends_with(".gif")
                            || lower.ends_with(".svg")
                            || lower.ends_with(".html")
                        {
                            return true;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    false
}

fn is_gfm_table_divider(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('|') || !trimmed.contains('-') {
        return false;
    }
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let cells: Vec<&str> = inner.split('|').collect();
    if cells.is_empty() {
        return false;
    }
    cells.iter().all(|cell| {
        let c = cell.trim();
        if c.is_empty() {
            return false;
        }
        let c = c.strip_prefix(':').unwrap_or(c);
        let c = c.strip_suffix(':').unwrap_or(c);
        !c.is_empty() && c.bytes().all(|b| b == b'-')
    })
}
