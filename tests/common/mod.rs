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

//! Shared assertions keep formatting tests focused on their input and expected output.
#![allow(dead_code)] // Each integration-test binary uses a different subset of these helpers.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use markfmt::{format_markdown, FormattingOptions};
use pulldown_cmark::{html, Options, Parser};
use tempfile::TempDir;

pub fn options(width: usize) -> FormattingOptions {
    let mut opts = FormattingOptions::default();
    opts.max_line_width = width;
    opts
}

/// Check the expected formatting and its stability, with useful diffs on failure.
pub fn assert_format(input: &str, expected: &str, options: &FormattingOptions) {
    let actual = format_markdown(input, options);
    pretty_assertions::assert_eq!(actual, expected, "input: {input:?}, options: {options:?}");
    pretty_assertions::assert_eq!(
        format_markdown(&actual, options),
        actual,
        "not idempotent: {input:?}"
    );
}

pub fn render(md: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
        | Options::ENABLE_MATH;
    let mut html = String::new();
    html::push_html(&mut html, Parser::new_ext(md, options));
    normalize_html(&html)
}

/// Render with comrak, a port of GitHub's cmark-gfm that follows the CommonMark spec. pulldown-cmark
/// deviates from the spec in places (for example, it keeps leading whitespace on continuation
/// lines inside code spans), so output must render the same under both.
pub fn render_gfm(md: &str) -> String {
    let mut options = comrak::Options::default();
    options.extension.table = true;
    options.extension.tasklist = true;
    options.extension.strikethrough = true;
    options.extension.footnotes = true;
    normalize_html(&comrak::markdown_to_html(md, &options))
}

/// Collapse only ordinary text whitespace. Tags (including quoted attributes), code, raw text,
/// and math retain their exact contents; a global split_whitespace would hide data corruption.
fn normalize_html(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut protected: Vec<String> = Vec::new();
    let mut was_space = false;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            let mut quote = None;
            let end = if rest.starts_with("<!--") {
                rest.find("-->").map_or(rest.len(), |i| i + 3)
            } else {
                rest.char_indices()
                    .find_map(|(i, c)| {
                        if let Some(q) = quote {
                            if c == q {
                                quote = None;
                            }
                        } else if c == '\'' || c == '"' {
                            quote = Some(c);
                        } else if c == '>' {
                            return Some(i + 1);
                        }
                        None
                    })
                    .unwrap_or(rest.len())
            };
            let tag = &rest[..end];
            let closing = tag.starts_with("</");
            let name = tag
                .trim_start_matches(['<', '/'])
                .split(|c: char| !c.is_ascii_alphanumeric())
                .next()
                .unwrap_or("");
            if closing && protected.last().is_some_and(|last| last == name) {
                protected.pop();
            } else if !closing
                && !tag.ends_with("/>")
                && (matches!(name, "pre" | "code" | "script" | "style" | "textarea")
                    || (name == "span" && tag.contains("class=\"math")))
            {
                protected.push(name.to_string());
            }
            out.push_str(tag);
            was_space = false;
            rest = &rest[end..];
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..end];
            if !protected.is_empty() {
                out.push_str(text);
            } else {
                for c in text.chars() {
                    if c.is_whitespace() {
                        if !was_space {
                            out.push(' ');
                        }
                        was_space = true;
                    } else {
                        out.push(c);
                        was_space = false;
                    }
                }
            }
            rest = &rest[end..];
        }
    }
    out.trim().to_string()
}

pub fn check_round_trip(input: &str, options: &FormattingOptions) -> Option<String> {
    check_round_trip_with(input, options, true)
}

/// For inputs that pulldown-cmark and cmark-gfm already parse differently (so no output could
/// preserve both readings): check idempotence and pulldown-cmark rendering only.
pub fn assert_round_trip_renderers_disagree(input: &str, options: &FormattingOptions) {
    if let Some(failure) = check_round_trip_with(input, options, false) {
        panic!("{failure}");
    }
}

fn check_round_trip_with(input: &str, options: &FormattingOptions, gfm: bool) -> Option<String> {
    let once = format_markdown(input, options);
    let twice = format_markdown(&once, options);
    let reason = if once != twice {
        "not idempotent"
    } else if render(input) != render(&once) {
        "rendering changed (pulldown-cmark)"
    } else if gfm && render_gfm(input) != render_gfm(&once) {
        return Some(format!("rendering changed (GFM) with {options:?}\ninput: {input:?}\nonce: {once:?}\nHTML before: {}\nHTML after: {}", render_gfm(input), render_gfm(&once)));
    } else {
        return None;
    };
    Some(format!("{reason} with {options:?}\ninput: {input:?}\nonce: {once:?}\ntwice: {twice:?}\nHTML before: {}\nHTML after: {}", render(input), render(&once)))
}

pub fn assert_round_trip(input: &str, options: &FormattingOptions) {
    if let Some(failure) = check_round_trip(input, options) {
        panic!("{failure}");
    }
}

/// An isolated project for CLI tests. All commands run in it, and EditorConfig lookup stops here.
pub struct Project(pub TempDir);
impl Project {
    pub fn new() -> Self {
        let project = Self(TempDir::new().unwrap());
        project.write(".editorconfig", "root = true\n");
        project
    }
    pub fn path(&self) -> &Path {
        self.0.path()
    }
    pub fn write(&self, path: &str, content: &str) {
        let path = self.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    pub fn read(&self, path: &str) -> String {
        fs::read_to_string(self.path().join(path)).unwrap()
    }
    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_markfmt"));
        command.current_dir(self.path());
        command
    }
    pub fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    pub fn stdin(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    pub fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(self.path())
            .args(args)
            .output()
            .unwrap();
        assert_success(&output);
    }
    pub fn init_git(&self) {
        self.git(&["init", "-q"]);
        self.git(&["config", "user.name", "Formatter Test"]);
        self.git(&["config", "user.email", "test@example.invalid"]);
    }
    pub fn commit(&self) {
        self.git(&["add", "."]);
        self.git(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Test fixture",
        ]);
    }
}

pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {}, stdout: {}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
