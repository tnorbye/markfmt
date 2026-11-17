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

use std::fs;
use std::path::Path;

use clap::builder::{EnumValueParser, TypedValueParser};
use clap::{CommandFactory, Parser};
use clap_complete::Shell;

use crate::options::{
    CliOverrides, OrderedListStyle, OrderedListStyleName, WrapMode, WrapModeName,
};

/// When to use colored output (`--color`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ColorWhen {
    /// Color when writing to a terminal that supports it, unless `NO_COLOR` is set
    #[default]
    Auto,
    /// Always color, even when output is redirected
    Always,
    /// Never color
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitChangeMode {
    /// Staged changes (`git diff --cached`).
    Staged,
    /// Staged and unstaged changes to tracked files (`git diff HEAD`) plus untracked files.
    Uncommitted,
    /// Working-tree changes relative to `<rev>` (`git diff <rev>`) plus untracked files (`--since`).
    Since(String),
    /// Changes introduced by a single commit (`git show <rev>`, including `HEAD`).
    Commit(String),
    /// Committed changes in `<base>..<head>` or `<base>...<head>`.
    Range {
        base: String,
        head: String,
        merge_base: bool,
    },
}

fn validate_git_rev(rev: &str) -> Result<String, String> {
    if rev.is_empty() || rev.contains('\0') {
        return Err("Git revision must not be empty".to_string());
    }
    if rev.starts_with('-') {
        return Err(format!(
            "Invalid git revision `{rev}`: revisions cannot start with `-`"
        ));
    }
    Ok(rev.to_string())
}

fn parse_git_mode(s: &str) -> Result<GitChangeMode, String> {
    if s.is_empty() {
        return Err("--overlaps-git-changes requires a mode, commit, or range".to_string());
    }
    if !s.contains('/') {
        match s.to_ascii_lowercase().as_str() {
            "head" => return Ok(GitChangeMode::Commit("HEAD".to_string())),
            "staged" | "cached" => return Ok(GitChangeMode::Staged),
            "uncommitted" => return Ok(GitChangeMode::Uncommitted),
            _ => {}
        }
    }
    // As in Git, an omitted side of a range is `HEAD` (`main..` is `main..HEAD`).
    let side = |rev: &str| -> Result<String, String> {
        if rev.is_empty() {
            Ok("HEAD".to_string())
        } else {
            validate_git_rev(rev)
        }
    };
    for (dots, merge_base) in [("...", true), ("..", false)] {
        if let Some((base, head)) = s.split_once(dots) {
            if (base.is_empty() && head.is_empty()) || head.starts_with('.') {
                return Err(format!(
                    "Invalid git range `{s}`: give `<base>{dots}<head>`, `<base>{dots}`, or `{dots}<head>` (use `--since <rev>` to diff the working tree against a commit)"
                ));
            }
            return Ok(GitChangeMode::Range {
                base: side(base)?,
                head: side(head)?,
                merge_base,
            });
        }
    }
    Ok(GitChangeMode::Commit(validate_git_rev(s)?))
}

fn parse_positive_usize(s: &str) -> Result<usize, String> {
    let val: usize = s
        .parse()
        .map_err(|_| format!("`{s}` is not a valid positive integer"))?;
    if val == 0 {
        return Err("value must be at least 1".to_string());
    }
    Ok(val)
}

fn parse_comment_char(s: &str) -> Result<String, String> {
    crate::config::validate_comment_char(s)?;
    Ok(s.to_string())
}

fn parse_trailer_separators(s: &str) -> Result<String, String> {
    crate::config::validate_trailer_separators(s)?;
    Ok(s.to_string())
}

fn parse_editor_cmd(s: &str) -> Result<String, String> {
    if s.trim().is_empty() {
        return Err("--commit-editor command must not be empty".to_string());
    }
    Ok(s.to_string())
}

#[derive(Debug, Parser)]
#[command(
    name = "markfmt",
    version,
    before_help = concat!("markfmt ", env!("CARGO_PKG_VERSION")),
    about = "Reflow and format Markdown, Markdeep, AI agent instruction/skill files, and commit messages.",
    long_about = "Reflow and format Markdown, Markdeep, AI agent instruction/skill files, and commit messages.\n\n\
        Inspired by kdoc-formatter (https://github.com/tnorbye/kdoc-formatter). Uses optimal\n\
        Knuth-Plass paragraph balancing for longer paragraphs (4+ lines) and preserves\n\
        existing Markdown syntax choices, Mermaid/code blocks, YAML frontmatter, and symlinks.\n\n\
        Use `@<filename>` to read newline-separated file paths from a file, or `-` to read\n\
        from stdin and write to stdout."
)]
pub struct Cli {
    /// Maximum line width [default: 72]
    #[arg(
        long = "max-line-width",
        visible_aliases = ["line-width", "right-margin"],
        value_name = "N",
        value_hint = clap::ValueHint::Other,
        value_parser = parse_positive_usize
    )]
    pub max_line_width: Option<usize>,

    /// Override continuation and nested list indent spaces
    #[arg(long = "hanging-indent", value_name = "N", value_hint = clap::ValueHint::Other)]
    pub hanging_indent: Option<usize>,

    /// Align GFM table columns [default: on]
    #[arg(
        long = "align-table-columns",
        overrides_with = "no_align_table_columns"
    )]
    pub align_table_columns: bool,

    /// Leave table column spacing unchanged
    #[arg(
        long = "no-align-table-columns",
        overrides_with = "align_table_columns"
    )]
    pub no_align_table_columns: bool,

    /// Use greedy line breaking instead of optimal Knuth-Plass
    #[arg(long = "greedy", overrides_with = "optimal")]
    pub greedy: bool,

    /// Use optimal line breaking, overriding greedy mode in configuration
    #[arg(long = "optimal", overrides_with = "greedy")]
    pub optimal: bool,

    /// Print resolved formatting settings for each input without modifying files
    #[arg(long = "print-config", conflicts_with_all = ["check", "diff", "dry_run"])]
    pub print_config: bool,

    /// Explain included/skipped inputs and configuration sources without modifying files
    #[arg(long = "explain", conflicts_with_all = ["check", "diff", "dry_run"])]
    pub explain: bool,

    /// How to break paragraph text into lines [default: always]
    #[arg(
        long = "wrap",
        value_name = "MODE",
        value_parser = EnumValueParser::<WrapModeName>::new().map(WrapMode::from)
    )]
    pub wrap: Option<WrapMode>,

    /// How to number ordered list items [default: preserve]
    #[arg(
        long = "ordered-lists",
        value_name = "STYLE",
        value_parser = EnumValueParser::<OrderedListStyleName>::new().map(OrderedListStyle::from)
    )]
    pub ordered_lists: Option<OrderedListStyle>,

    /// Format as a Git or Jujutsu commit message (preserving subject, trailers, and comments)
    #[arg(long = "commit-msg", visible_alias = "commit-message")]
    pub commit_msg: bool,

    /// Launch CMD (or $MARKFMT_EDITOR / $VISUAL / $EDITOR) to edit files, then format commit messages
    #[arg(
        long = "commit-editor",
        value_name = "CMD",
        value_hint = clap::ValueHint::CommandString,
        num_args = 0..=1,
        require_equals = true,
        value_parser = parse_editor_cmd,
        conflicts_with_all = [
            "check",
            "diff",
            "dry_run",
            "print_config",
            "explain",
            "lines",
            "overlaps_git_changes",
            "staged",
            "since",
            "stdout",
            "in_place",
            "style",
            "fallback_style",
            "sort_includes"
        ]
    )]
    pub commit_editor: Option<Option<String>>,

    /// Comment line prefix in commit-message mode (e.g. `#`, `;`, `//`, or `auto`)
    #[arg(
        long = "comment-char",
        value_name = "CHAR",
        value_hint = clap::ValueHint::Other,
        value_parser = parse_comment_char
    )]
    pub comment_char: Option<String>,

    /// Trailer key/value separator characters in commit-message mode (default from Git's
    /// `trailer.separators`, else `:`)
    #[arg(
        long = "trailer-separators",
        value_name = "CHARS",
        value_hint = clap::ValueHint::Other,
        value_parser = parse_trailer_separators
    )]
    pub trailer_separators: Option<String>,

    /// Do not read .editorconfig files
    #[arg(long = "no-editorconfig")]
    pub no_editorconfig: bool,

    /// Use this config file instead of searching for .markfmt.toml
    #[arg(
        long = "config",
        value_name = "PATH",
        value_hint = clap::ValueHint::FilePath,
        conflicts_with = "no_config"
    )]
    pub config: Option<String>,

    /// Do not read .markfmt.toml files
    #[arg(long = "no-config")]
    pub no_config: bool,

    /// Skip files matching this pattern (relative to the current directory; repeatable)
    #[arg(long = "exclude", value_name = "PATTERN", value_hint = clap::ValueHint::Other)]
    pub exclude: Vec<String>,

    /// Do not respect .gitignore when traversing directories
    #[arg(long = "no-ignore")]
    pub no_ignore: bool,

    /// Apply Markdown file discovery rules to explicitly named files
    #[arg(long = "filter-files")]
    pub filter_files: bool,

    /// Only format blocks overlapping git changes (`uncommitted`, `staged`/`cached`, `<COMMIT>`, or `<BASE>..<HEAD>`/`<BASE>...<HEAD>`; repeatable)
    #[arg(
        long = "overlaps-git-changes",
        value_name = "MODE",
        value_hint = clap::ValueHint::Other,
        value_parser = parse_git_mode
    )]
    pub overlaps_git_changes: Vec<GitChangeMode>,

    /// Only format blocks overlapping staged git changes (shorthand for `--overlaps-git-changes=staged`)
    #[arg(long = "staged", visible_alias = "cached")]
    pub staged: bool,

    /// Only format blocks overlapping working-tree or untracked changes since `<REV>` (repeatable)
    #[arg(
        long = "since",
        value_name = "REV",
        value_hint = clap::ValueHint::Other,
        value_parser = validate_git_rev
    )]
    pub since: Vec<String>,

    /// Explicit path to git executable
    #[arg(long = "git-path", value_name = "PATH", value_hint = clap::ValueHint::ExecutablePath)]
    pub git_path: Option<String>,

    /// Only format blocks overlapping 1-based line range(s), e.g. `5:10` or `1:3,8`
    #[arg(
        long = "lines",
        alias = "line",
        value_name = "RANGE",
        value_hint = clap::ValueHint::Other
    )]
    pub lines: Vec<String>,

    /// Virtual file path when formatting stdin (`-`) for .editorconfig/Markdeep/commit detection (implies `-` without `<FILES>`)
    #[arg(
        long = "stdin-filepath",
        visible_alias = "assume-filename",
        value_name = "PATH",
        value_hint = clap::ValueHint::FilePath
    )]
    pub stdin_filepath: Option<String>,

    /// Write formatted output to stdout without modifying files (automatic under git-clang-format)
    #[arg(
        long = "stdout",
        overrides_with = "in_place",
        conflicts_with_all = ["check", "diff", "dry_run", "print_config", "explain"]
    )]
    pub stdout: bool,

    /// Modify files in place (default)
    #[arg(
        short = 'i',
        long = "in-place",
        overrides_with = "stdout",
        conflicts_with_all = ["check", "diff", "dry_run", "print_config", "explain"]
    )]
    pub in_place: bool,

    /// Ignored; accepted for compatibility with git-clang-format
    #[arg(long = "style", value_name = "STYLE", hide = true)]
    pub style: Option<String>,

    /// Ignored; accepted for compatibility with clang-format-diff.py
    #[arg(long = "fallback-style", value_name = "STYLE", hide = true)]
    pub fallback_style: Option<String>,

    /// Ignored; accepted for compatibility with clang-format-diff.py
    #[arg(
        long = "sort-includes",
        value_name = "BOOL",
        hide = true,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        value_parser = clap::value_parser!(bool)
    )]
    pub sort_includes: Option<bool>,

    /// Print the given files that `--exclude` or `.markfmt.toml` excludes, then exit
    // Clang-format style flags, `--config`, `--exclude`, and `--commit-msg` stay allowed: they
    // are what `git-clang-format` passes, or change what is excluded.
    #[arg(
        long = "list-ignored",
        conflicts_with_all = [
            "check",
            "diff",
            "dry_run",
            "print_config",
            "explain",
            "commit_editor",
            "lines",
            "overlaps_git_changes",
            "staged",
            "since",
            "stdout",
            "in_place",
            "stdin_filepath"
        ]
    )]
    pub list_ignored: bool,

    /// Print paths of files that would change without modifying them (exits 0)
    #[arg(short = 'n', long = "dry-run")]
    pub dry_run: bool,

    /// Verify formatting without modifying files; exit 1 if any files would change
    #[arg(long = "check")]
    pub check: bool,

    /// Print unified diff of formatting changes without modifying files
    #[arg(long = "diff")]
    pub diff: bool,

    /// When to color output and diffs
    #[arg(long = "color", value_name = "WHEN", default_value = "auto")]
    pub color: ColorWhen,

    /// Suppress summary output
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,

    /// Also list each reformatted file
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Print a completion script for SHELL to stdout and exit
    #[arg(long = "completions", value_name = "SHELL", exclusive = true)]
    pub completions: Option<Shell>,

    /// Print a man page in roff format to stdout and exit
    #[arg(long = "man", exclusive = true)]
    pub man: bool,

    /// Input files or directories (use `-` for stdin)
    #[arg(value_name = "FILES", value_hint = clap::ValueHint::AnyPath)]
    pub files: Vec<String>,
}

impl Cli {
    pub fn cli_overrides(&self) -> CliOverrides {
        let align_table_columns = if self.no_align_table_columns {
            Some(false)
        } else if self.align_table_columns {
            Some(true)
        } else {
            None
        };

        let optimal = if self.greedy {
            Some(false)
        } else if self.optimal {
            Some(true)
        } else {
            None
        };

        CliOverrides {
            max_line_width: self.max_line_width,
            hanging_indent: self.hanging_indent,
            align_table_columns,
            optimal,
            wrap: self.wrap,
            ordered_lists: self.ordered_lists,
            comment_char: self.comment_char.clone(),
            trailer_separators: self.trailer_separators.clone(),
        }
    }

    /// Combines `--overlaps-git-changes`, `--staged`, and `--since` in order without duplicates.
    pub fn git_change_modes(&self) -> Vec<GitChangeMode> {
        let mut modes = Vec::new();
        let mut push_unique = |mode: GitChangeMode| {
            if !modes.contains(&mode) {
                modes.push(mode);
            }
        };
        for mode in &self.overlaps_git_changes {
            push_unique(mode.clone());
        }
        if self.staged {
            push_unique(GitChangeMode::Staged);
        }
        for rev in &self.since {
            push_unique(GitChangeMode::Since(rev.clone()));
        }
        modes
    }

    /// Whether file outputs should be written to stdout rather than back to disk.
    ///
    /// Also enabled automatically when `git-clang-format` invokes `markfmt` in working-tree
    /// mode (`markfmt [--style=...] [--lines=a:b] path/to/file.md` with `GIT_INDEX_FILE` set
    /// to its temporary `clang-format-index*` file; `--whole-file` omits `--lines`), or when a
    /// clang-format style flag (`--style`, `--fallback-style`, `--sort-includes`) is given.
    /// A direct `markfmt --lines=a:b file.md` still edits the file in place.
    pub fn write_to_stdout(&self) -> bool {
        if self.stdout {
            return true;
        }
        if self.in_place || self.check || self.diff || self.dry_run {
            return false;
        }
        // Style flags only make sense from clang-format tooling, which reads stdout.
        self.has_clang_format_style_flag() || is_git_clang_format_index()
    }

    /// Whether a clang-format style flag was given (`--sort-includes=false` included: only its
    /// presence matters).
    pub fn has_clang_format_style_flag(&self) -> bool {
        self.style.is_some() || self.fallback_style.is_some() || self.sort_includes.is_some()
    }
}

/// Whether `git-clang-format` is running `markfmt`: it points `GIT_INDEX_FILE` at its temporary
/// `clang-format-index*` file while formatting, in every mode.
pub fn is_git_clang_format_index() -> bool {
    std::env::var_os("GIT_INDEX_FILE").is_some_and(|val| {
        Path::new(&val)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("clang-format-index"))
    })
}

/// Renders a completion script for `shell` (`--completions`).
pub fn completions(shell: Shell) -> Vec<u8> {
    let mut out = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "markfmt", &mut out);
    out
}

/// Renders the `markfmt(1)` man page in roff format (`--man`).
pub fn man_page() -> Vec<u8> {
    let mut out = Vec::new();
    clap_mangen::Man::new(Cli::command())
        .render(&mut out)
        .expect("writing to a Vec cannot fail");
    out
}

/// `clang-format` flags that tools may spell with a single dash (`-lines=1:2`).
const CLANG_FORMAT_LONG_FLAGS: [&str; 7] = [
    "list-ignored",
    "lines",
    "line",
    "assume-filename",
    "style",
    "fallback-style",
    "sort-includes",
];

fn normalize_arg(arg: &str) -> String {
    match arg {
        "-help" => "--help".to_string(),
        "--align-table-columns=false" | "--align-table-columns=off" => {
            "--no-align-table-columns".to_string()
        }
        "--align-table-columns=true" | "--align-table-columns=on" => {
            "--align-table-columns".to_string()
        }
        _ if !arg.starts_with("--")
            && CLANG_FORMAT_LONG_FLAGS.iter().any(|flag| {
                arg.strip_prefix('-')
                    .and_then(|rest| rest.strip_prefix(flag))
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('='))
            }) =>
        {
            format!("-{arg}")
        }
        _ => arg.to_string(),
    }
}

fn push_arg(token: &str, after_double_dash: &mut bool, out: &mut Vec<String>) {
    if *after_double_dash {
        out.push(token.to_string());
    } else if token == "--" {
        *after_double_dash = true;
        out.push(token.to_string());
    } else {
        out.push(normalize_arg(token));
    }
}

/// Expands `@<filename>` response files and normalizes compatibility flags.
pub fn expand_args<I, S>(raw_args: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut iter = raw_args.into_iter();
    let mut out = Vec::new();
    if let Some(prog) = iter.next() {
        out.push(prog.as_ref().to_string());
    }

    let mut after_double_dash = false;
    for arg_ref in iter {
        let arg = arg_ref.as_ref();
        if !after_double_dash {
            if let Some(path_str) = arg.strip_prefix('@') {
                if path_str.is_empty() {
                    return Err("Empty `@` argument file path".to_string());
                }
                let path = Path::new(path_str);
                let content = fs::read_to_string(path)
                    .map_err(|e| format!("Failed to read argument file `{path_str}`: {e}"))?;
                for line in content.split('\n') {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        push_arg(trimmed, &mut after_double_dash, &mut out);
                    }
                }
                continue;
            }
        }
        push_arg(arg, &mut after_double_dash, &mut out);
    }

    Ok(out)
}
