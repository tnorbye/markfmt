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
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::options::{FormattingOptions, LineEnding};

const CODE_PROBE_FILENAMES: &[&str] =
    &["probe.kt", "probe.rs", "probe.java", "probe.py", "probe.js"];

#[derive(Debug, Clone)]
struct Section {
    pattern: String,
    entries: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct EditorConfigFile {
    dir: PathBuf,
    is_root: bool,
    sections: Vec<Section>,
}

#[derive(Debug, Default)]
pub struct EditorConfigResolver {
    dir_cache: HashMap<PathBuf, Option<Rc<EditorConfigFile>>>,
}

impl EditorConfigResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves `.editorconfig` settings applicable to `file_path`.
    ///
    /// `max_line_length` and `indent_size` are applied ONLY from Markdown/doc-specific sections
    /// (sections whose glob does not also match code probe filenames like `probe.rs`/`probe.kt`),
    /// whereas `tab_width`, `end_of_line`, and `insert_final_newline` also apply from catch-all
    /// `[*]` sections.
    pub fn resolve(&mut self, file_path: &Path) -> FormattingOptions {
        let mut options = FormattingOptions::default();

        let abs_path = crate::config::normalized_path(file_path);

        let Some(start_dir) = abs_path.parent() else {
            return options;
        };

        let chain = self.config_chain(start_dir);
        for cfg in &chain {
            let rel_path = abs_path.strip_prefix(&cfg.dir).unwrap_or(&abs_path);
            let rel_str = path_to_slash_string(rel_path);
            let file_name = abs_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            for section in &cfg.sections {
                if !glob_matches_target(&section.pattern, &rel_str, &file_name) {
                    continue;
                }

                let is_catchall_for_code = CODE_PROBE_FILENAMES.iter().any(|probe| {
                    let probe_path = rel_str
                        .rsplit_once('/')
                        .map_or_else(|| probe.to_string(), |(dir, _)| format!("{dir}/{probe}"));
                    glob_matches_target(&section.pattern, &probe_path, probe)
                });

                for (key, value) in &section.entries {
                    let val_lower = value.to_ascii_lowercase();
                    match key.as_str() {
                        "tab_width" => {
                            if val_lower == "unset" {
                                options.tab_width = FormattingOptions::default().tab_width;
                            } else if let Ok(v) = val_lower.parse::<usize>() {
                                if v > 0 {
                                    options.tab_width = v;
                                }
                            }
                        }
                        "insert_final_newline" => match val_lower.as_str() {
                            "true" => options.insert_final_newline = Some(true),
                            "false" => options.insert_final_newline = Some(false),
                            "unset" => options.insert_final_newline = None,
                            _ => {}
                        },
                        "end_of_line" => match val_lower.as_str() {
                            "lf" => options.line_ending = LineEnding::Lf,
                            "crlf" => options.line_ending = LineEnding::CrLf,
                            "unset" => options.line_ending = LineEnding::Auto,
                            _ => {}
                        },
                        "max_line_length" if !is_catchall_for_code => {
                            if val_lower == "unset" || val_lower == "off" {
                                options.max_line_width =
                                    FormattingOptions::default().max_line_width;
                            } else if let Ok(v) = val_lower.parse::<usize>() {
                                if v > 0 {
                                    options.max_line_width = v;
                                }
                            }
                        }
                        "indent_size" if !is_catchall_for_code => {
                            if val_lower == "unset" {
                                options.min_hanging_indent = None;
                            } else if let Ok(v) = val_lower.parse::<usize>() {
                                options.min_hanging_indent = Some(v);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        options
    }

    /// EditorConfig files consulted for an absolute file path, in precedence order.
    pub fn source_paths(&mut self, absolute_file: &Path) -> Vec<PathBuf> {
        crate::config::normalized_path(absolute_file)
            .parent()
            .map(|dir| {
                self.config_chain(dir)
                    .iter()
                    .map(|config| config.dir.join(".editorconfig"))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn config_chain(&mut self, start_dir: &Path) -> Vec<Rc<EditorConfigFile>> {
        let mut chain = Vec::new();
        let mut current = Some(start_dir.to_path_buf());
        while let Some(dir) = current {
            if let Some(config) = self.load_config_in_dir(&dir) {
                let stop = config.is_root;
                chain.push(config);
                if stop {
                    break;
                }
            }
            current = dir.parent().map(Path::to_path_buf);
        }
        chain.reverse();
        chain
    }

    fn load_config_in_dir(&mut self, dir: &Path) -> Option<Rc<EditorConfigFile>> {
        if let Some(cached) = self.dir_cache.get(dir) {
            return cached.clone();
        }
        let candidate = dir.join(".editorconfig");
        let parsed = if candidate.is_file() {
            fs::read_to_string(&candidate)
                .ok()
                .map(|content| Rc::new(parse_editorconfig(dir, &content)))
        } else {
            None
        };
        self.dir_cache.insert(dir.to_path_buf(), parsed.clone());
        parsed
    }
}

fn path_to_slash_string(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn parse_editorconfig(dir: &Path, content: &str) -> EditorConfigFile {
    let mut is_root = false;
    let mut sections: Vec<Section> = Vec::new();
    let mut in_preamble = true;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            in_preamble = false;
            let pattern = line[1..line.len() - 1].to_string();
            sections.push(Section {
                pattern,
                entries: Vec::new(),
            });
            continue;
        }
        if let Some((raw_key, raw_val)) = line.split_once('=') {
            let key = raw_key.trim().to_ascii_lowercase();
            let val = raw_val.trim().to_string();
            if in_preamble {
                if key == "root" && val.eq_ignore_ascii_case("true") {
                    is_root = true;
                }
            } else if let Some(sec) = sections.last_mut() {
                sec.entries.push((key, val));
            }
        }
    }

    EditorConfigFile {
        dir: dir.to_path_buf(),
        is_root,
        sections,
    }
}

fn glob_matches_target(pattern: &str, rel_path: &str, file_name: &str) -> bool {
    let expanded = expand_braces(pattern);
    for pat in expanded {
        let has_slash = pat.contains('/');
        let clean_pat = pat.strip_prefix('/').unwrap_or(&pat);
        let target = if has_slash { rel_path } else { file_name };
        if match_glob(clean_pat, target) {
            return true;
        }
    }
    false
}

/// Expands EditorConfig `{a,b,c}` brace groups (including nested braces).
fn expand_braces(pattern: &str) -> Vec<String> {
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'{' {
            if let Some((close_idx, Some(parts))) = find_matching_brace(pattern, i) {
                let prefix = &pattern[..i];
                let suffix_variants = expand_braces(&pattern[close_idx + 1..]);
                let mut result = Vec::new();
                for part in parts {
                    for mid in expand_braces(&part) {
                        for suf in &suffix_variants {
                            result.push(format!("{prefix}{mid}{suf}"));
                        }
                    }
                }
                return result;
            }
        }
        i += 1;
    }
    vec![pattern.to_string()]
}

fn find_matching_brace(s: &str, open_idx: usize) -> Option<(usize, Option<Vec<String>>)> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut i = open_idx;
    let mut current_start = open_idx + 1;
    let mut parts = Vec::new();
    let mut saw_comma_at_depth_1 = false;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'{' {
            depth += 1;
        } else if bytes[i] == b'}' {
            depth -= 1;
            if depth == 0 {
                parts.push(s[current_start..i].to_string());
                if saw_comma_at_depth_1 {
                    return Some((i, Some(parts)));
                } else {
                    // Numeric range `{1..5}` check or literal `{foo}`
                    let inner = &s[open_idx + 1..i];
                    if let Some((a, b)) = inner.split_once("..") {
                        if let (Ok(start_n), Ok(end_n)) = (a.parse::<i64>(), b.parse::<i64>()) {
                            if start_n.abs_diff(end_n) <= 1000 {
                                let range_parts: Vec<String> = if start_n <= end_n {
                                    (start_n..=end_n).map(|n| n.to_string()).collect()
                                } else {
                                    (end_n..=start_n).rev().map(|n| n.to_string()).collect()
                                };
                                return Some((i, Some(range_parts)));
                            }
                        }
                    }
                    return Some((i, None));
                }
            }
        } else if bytes[i] == b',' && depth == 1 {
            saw_comma_at_depth_1 = true;
            parts.push(s[current_start..i].to_string());
            current_start = i + 1;
        }
        i += 1;
    }
    None
}

fn match_glob(pat: &str, text: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    match_glob_chars(&p, &t)
}

fn match_glob_chars(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }

    // Handle `**`
    if p.len() >= 2 && p[0] == '*' && p[1] == '*' {
        // Consume any extra consecutive `*`
        let mut p_next = 2;
        while p_next < p.len() && p[p_next] == '*' {
            p_next += 1;
        }
        // `**/` can also match zero path segments
        if p_next < p.len() && p[p_next] == '/' && match_glob_chars(&p[p_next + 1..], t) {
            return true;
        }
        for split in 0..=t.len() {
            if match_glob_chars(&p[p_next..], &t[split..]) {
                return true;
            }
        }
        return false;
    }

    // Handle single `*` (matches anything except `/`)
    if p[0] == '*' {
        for split in 0..=t.len() {
            if match_glob_chars(&p[1..], &t[split..]) {
                return true;
            }
            if split < t.len() && t[split] == '/' {
                break;
            }
        }
        return false;
    }

    if t.is_empty() {
        return false;
    }

    // Handle `?` (matches any single char except `/`)
    if p[0] == '?' {
        if t[0] == '/' {
            return false;
        }
        return match_glob_chars(&p[1..], &t[1..]);
    }

    // Handle character class `[...]`
    if p[0] == '[' {
        if let Some((matched, class_end)) = match_char_class(&p[1..], t[0]) {
            if t[0] == '/' {
                return false;
            }
            return matched && match_glob_chars(&p[1 + class_end + 1..], &t[1..]);
        }
    }

    // Handle escaped char `\`
    if p[0] == '\\' && p.len() >= 2 {
        return p[1] == t[0] && match_glob_chars(&p[2..], &t[1..]);
    }

    p[0] == t[0] && match_glob_chars(&p[1..], &t[1..])
}

fn match_char_class(p: &[char], ch: char) -> Option<(bool, usize)> {
    if p.is_empty() {
        return None;
    }
    let mut idx = 0;
    let negate = if p[0] == '!' || p[0] == '^' {
        idx += 1;
        true
    } else {
        false
    };

    let mut matched = false;
    let mut i = idx;
    while i < p.len() {
        if p[i] == ']' && i > idx {
            let ok = if negate { !matched } else { matched };
            return Some((ok, i));
        }
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            if ch >= p[i] && ch <= p[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if ch == p[i] {
                matched = true;
            }
            i += 1;
        }
    }
    None
}
