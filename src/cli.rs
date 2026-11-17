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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitChangeMode {
    /// Changes in the most recent commit (`git show HEAD`).
    Head,
    /// Staged changes (`git diff --cached`).
    Staged,
    /// Staged and unstaged changes to tracked files (`git diff HEAD`) plus untracked files.
    Uncommitted,
}

fn parse_git_mode(s: &str) -> Result<GitChangeMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "head" => Ok(GitChangeMode::Head),
        "staged" => Ok(GitChangeMode::Staged),
        "uncommitted" => Ok(GitChangeMode::Uncommitted),
        _ => Err(format!(
            "--overlaps-git-changes supports `HEAD`, `staged`, and `uncommitted`, got `{s}`"
        )),
    }
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
            "overlaps_git_changes"
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

    /// Only format blocks overlapping git changes (`HEAD`, `staged`, or `uncommitted`; repeatable)
    #[arg(
        long = "overlaps-git-changes",
        value_name = "MODE",
        value_hint = clap::ValueHint::Other,
        value_parser = parse_git_mode
    )]
    pub overlaps_git_changes: Vec<GitChangeMode>,

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

    /// Virtual file path when formatting stdin (`-`) for .editorconfig/Markdeep/commit detection
    #[arg(long = "stdin-filepath", value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
    pub stdin_filepath: Option<String>,

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

/// Expands `@<filename>` response files and normalizes kdoc-formatter compatibility flags.
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
        if after_double_dash {
            out.push(arg.to_string());
            continue;
        }
        if arg == "--" {
            after_double_dash = true;
            out.push(arg.to_string());
            continue;
        }
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
                    out.push(trimmed.to_string());
                }
            }
            continue;
        }

        match arg {
            "-help" => out.push("--help".to_string()),
            "--align-table-columns=false" | "--align-table-columns=off" => {
                out.push("--no-align-table-columns".to_string())
            }
            "--align-table-columns=true" | "--align-table-columns=on" => {
                out.push("--align-table-columns".to_string())
            }
            _ => out.push(arg.to_string()),
        }
    }

    Ok(out)
}
