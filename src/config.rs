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

//! Project configuration from `.markfmt.toml` and exclude patterns.
//!
//! For each file, the nearest `.markfmt.toml` in the file's directory or an ancestor applies
//! (files are not merged). Settings there override `.editorconfig`; command-line flags override
//! both.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::options::{FormattingOptions, OrderedListStyle, WrapMode};

/// Absolute path with `.` and `..` resolved before configuration ancestry is inspected.
/// Resolve existing prefixes before stepping through `..` so directory symlinks retain
/// filesystem semantics. Other symlinks keep their spelling for configuration lookup.
pub fn normalized_path(path: &Path) -> PathBuf {
    let cwd = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().unwrap_or_default()
    };
    normalized_path_in(&cwd, path)
}

/// [`normalized_path`] with relative paths resolved against `cwd` instead of the process's
/// current directory, so callers that already read it use one consistent base.
pub fn normalized_path_in(cwd: &Path, path: &Path) -> PathBuf {
    let absolute = cwd.join(path);
    let mut result = PathBuf::new();
    let mut remaining_hops = 40usize;
    for component in absolute.components() {
        push_normalized_component(&mut result, component, &mut remaining_hops);
    }
    result
}

fn push_normalized_component(
    result: &mut PathBuf,
    component: Component<'_>,
    remaining_hops: &mut usize,
) {
    match component {
        Component::CurDir => {}
        Component::ParentDir => {
            while *remaining_hops > 0 {
                let Ok(target) = fs::read_link(&*result) else {
                    break;
                };
                *remaining_hops -= 1;
                if target.is_absolute() {
                    result.clear();
                } else {
                    result.pop();
                }
                for sub in target.components() {
                    push_normalized_component(result, sub, remaining_hops);
                }
            }
            result.pop();
        }
        Component::Prefix(_) => {
            result.clear();
            result.push(component.as_os_str());
        }
        Component::RootDir | Component::Normal(_) => {
            result.push(component.as_os_str());
        }
    }
}

/// File name searched for in each file's directory and its ancestors.
pub const CONFIG_FILE_NAME: &str = ".markfmt.toml";

/// Settings under `[commit_message]` in `.markfmt.toml`, applied only when formatting commit
/// messages so documentation widths and excludes do not leak into `git commit`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommitMessageSettings {
    pub max_line_width: Option<usize>,
    pub hanging_indent: Option<usize>,
    pub align_table_columns: Option<bool>,
    pub greedy: Option<bool>,
    #[serde(default, with = "serde_wrap")]
    pub wrap: Option<WrapMode>,
    #[serde(default, with = "serde_ordered_lists")]
    pub ordered_lists: Option<OrderedListStyle>,
    pub comment_char: Option<String>,
    pub trailer_separators: Option<String>,
}

/// Validates trailer separator characters (`--trailer-separators` or
/// `[commit_message] trailer_separators`).
pub fn validate_trailer_separators(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("trailer_separators must not be empty".to_string());
    }
    // Git matches separators byte by byte, so only ASCII punctuation means the same thing here.
    if !s.chars().all(|c| c.is_ascii_punctuation()) {
        return Err(
            "trailer_separators must be ASCII punctuation (such as `:`, `-`, or `=`)".to_string(),
        );
    }
    Ok(())
}

/// Validates a commit-message comment prefix (`--comment-char` or `[commit_message] comment_char`).
pub fn validate_comment_char(s: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err("comment_char must not be empty".to_string());
    }
    if s.contains(['\n', '\r']) {
        return Err("comment_char must not contain newlines".to_string());
    }
    if s.starts_with(char::is_whitespace) {
        return Err("comment_char must not start with whitespace".to_string());
    }
    Ok(())
}

/// The settings a config file may contain. Unknown keys are rejected to catch typos.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub max_line_width: Option<usize>,
    pub hanging_indent: Option<usize>,
    pub align_table_columns: Option<bool>,
    pub greedy: Option<bool>,
    #[serde(default, with = "serde_wrap")]
    pub wrap: Option<WrapMode>,
    #[serde(default, with = "serde_ordered_lists")]
    pub ordered_lists: Option<OrderedListStyle>,
    /// Patterns (relative to the config file's directory) for files to skip.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Commit-message-specific settings (`[commit_message]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<CommitMessageSettings>,
}

/// Serde adapters that read and write the public option enums through their `*Name` spellings.
macro_rules! serde_via_name {
    ($module:ident, $ty:ident, $name:ident) => {
        mod $module {
            use crate::options::{$name, $ty};
            use serde::{Deserialize, Deserializer, Serialize, Serializer};

            pub fn serialize<S: Serializer>(
                val: &Option<$ty>,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                val.map($name::from).serialize(serializer)
            }

            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Option<$ty>, D::Error> {
                Ok(Option::<$name>::deserialize(deserializer)?.map($ty::from))
            }
        }
    };
}

serde_via_name!(serde_wrap, WrapMode, WrapModeName);
serde_via_name!(serde_ordered_lists, OrderedListStyle, OrderedListStyleName);

/// A loaded config file.
#[derive(Debug)]
pub struct Config {
    pub path: PathBuf,
    pub settings: Settings,
    excludes: ExcludeSet,
}

impl Config {
    /// Loads and validates the config file at `path`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let cwd = if path.is_absolute() {
            PathBuf::new()
        } else {
            std::env::current_dir().unwrap_or_default()
        };
        Self::load_in(&cwd, path)
    }

    /// Like [`Config::load`], resolving a relative `path` against `cwd`. Messages keep the
    /// caller's spelling of `path`; the stored path and exclude base are normalized.
    pub fn load_in(cwd: &Path, path: &Path) -> Result<Self, String> {
        let normalized = normalized_path_in(cwd, path);
        let text = fs::read_to_string(&normalized)
            .map_err(|err| format!("Failed to read `{}`: {err}", path.display()))?;
        let settings: Settings = toml::from_str(&text)
            .map_err(|err| format!("Invalid config `{}`: {err}", path.display()))?;
        if settings.max_line_width == Some(0) {
            return Err(format!(
                "Invalid config `{}`: max_line_width must be at least 1",
                path.display()
            ));
        }
        if let Some(cm) = &settings.commit_message {
            if cm.max_line_width == Some(0) {
                return Err(format!(
                    "Invalid config `{}`: commit_message.max_line_width must be at least 1",
                    path.display()
                ));
            }
            if let Some(prefix) = &cm.comment_char {
                validate_comment_char(prefix)
                    .map_err(|err| format!("Invalid config `{}`: {err}", path.display()))?;
            }
            if let Some(separators) = &cm.trailer_separators {
                validate_trailer_separators(separators)
                    .map_err(|err| format!("Invalid config `{}`: {err}", path.display()))?;
            }
        }
        let base = normalized.parent().unwrap_or_else(|| Path::new("."));
        let excludes = ExcludeSet::new(base, &settings.exclude)
            .map_err(|err| format!("Invalid config `{}`: {err}", path.display()))?;
        Ok(Self {
            path: normalized,
            settings,
            excludes,
        })
    }

    /// Applies this config's formatting settings on top of `options`.
    pub fn apply(&self, options: &mut FormattingOptions) {
        let s = &self.settings;
        if let Some(width) = s.max_line_width {
            options.max_line_width = width;
        }
        if let Some(indent) = s.hanging_indent {
            options.hanging_indent = Some(indent);
        }
        if let Some(align) = s.align_table_columns {
            options.align_table_columns = align;
        }
        if let Some(greedy) = s.greedy {
            options.optimal = !greedy;
        }
        if let Some(wrap) = s.wrap {
            options.wrap = wrap;
        }
        if let Some(style) = s.ordered_lists {
            options.ordered_lists = style;
        }
    }

    /// Applies this config's `[commit_message]` table (if any) on top of `options`, leaving
    /// top-level documentation settings untouched.
    pub fn apply_commit_message(&self, options: &mut FormattingOptions) {
        let Some(cm) = &self.settings.commit_message else {
            return;
        };
        if let Some(width) = cm.max_line_width {
            options.max_line_width = width;
        }
        if let Some(indent) = cm.hanging_indent {
            options.hanging_indent = Some(indent);
        }
        if let Some(align) = cm.align_table_columns {
            options.align_table_columns = align;
        }
        if let Some(greedy) = cm.greedy {
            options.optimal = !greedy;
        }
        if let Some(wrap) = cm.wrap {
            options.wrap = wrap;
        }
        if let Some(style) = cm.ordered_lists {
            options.ordered_lists = style;
        }
        if let Some(prefix) = &cm.comment_char {
            options.comment_char.clone_from(prefix);
        }
        if let Some(separators) = &cm.trailer_separators {
            options.trailer_separators.clone_from(separators);
        }
    }

    /// Serializes effective settings using the same keys accepted by project configuration.
    /// `exclude` is the applicable config file's patterns (relative to that file).
    pub fn resolved_settings(options: &FormattingOptions, exclude: &[String]) -> Settings {
        Settings {
            max_line_width: Some(options.max_line_width),
            hanging_indent: options.hanging_indent,
            align_table_columns: Some(options.align_table_columns),
            greedy: Some(!options.optimal),
            wrap: Some(options.wrap),
            ordered_lists: Some(options.ordered_lists),
            exclude: exclude.to_vec(),
            commit_message: options.commit_message.then(|| CommitMessageSettings {
                max_line_width: Some(options.max_line_width),
                hanging_indent: options.hanging_indent,
                align_table_columns: Some(options.align_table_columns),
                greedy: Some(!options.optimal),
                wrap: Some(options.wrap),
                ordered_lists: Some(options.ordered_lists),
                comment_char: Some(options.comment_char.clone()),
                trailer_separators: Some(options.trailer_separators.clone()),
            }),
        }
    }

    /// Whether `abs_path` matches one of this config's `exclude` patterns.
    pub fn excludes(&self, abs_path: &Path) -> bool {
        self.excludes.matches(abs_path)
    }
}

/// Finds and caches the config file that applies to each path.
#[derive(Debug, Default)]
pub struct ConfigResolver {
    /// A `--config` file used for every path instead of searching.
    explicit: Option<Rc<Config>>,
    dir_cache: HashMap<PathBuf, Option<Rc<Config>>>,
}

impl ConfigResolver {
    /// Searches for `.markfmt.toml` next to each file and in its ancestors.
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses the given config file (relative paths resolve against `cwd`) for every path.
    pub fn with_explicit(cwd: &Path, path: &Path) -> Result<Self, String> {
        Ok(Self {
            explicit: Some(Rc::new(Config::load_in(cwd, path)?)),
            dir_cache: HashMap::new(),
        })
    }

    /// The config that applies to the file at `abs_path` (an absolute path), if any.
    pub fn for_file(&mut self, abs_path: &Path) -> Result<Option<Rc<Config>>, String> {
        if let Some(explicit) = &self.explicit {
            return Ok(Some(explicit.clone()));
        }
        let abs_path = normalized_path(abs_path);
        let mut dir = abs_path.parent();
        let mut visited = Vec::new();
        let mut found = None;
        while let Some(d) = dir {
            if let Some(cached) = self.dir_cache.get(d) {
                found = cached.clone();
                break;
            }
            visited.push(d.to_path_buf());
            let candidate = d.join(CONFIG_FILE_NAME);
            if candidate.is_file() {
                found = Some(Rc::new(Config::load(&candidate)?));
                break;
            }
            dir = d.parent();
        }
        for d in visited {
            self.dir_cache.insert(d, found.clone());
        }
        Ok(found)
    }
}

/// Exclude patterns relative to a base directory, with `.gitignore`-like matching:
///
/// - A pattern without a `/` matches a file or directory name at any depth (`CHANGELOG.md`,
///   `*.generated.md`, `vendor`).
/// - A pattern containing a `/` matches the path relative to the base directory
///   (`docs/api/**`, `/README.md`).
/// - Matching a directory excludes everything inside it; a trailing `/` is allowed.
/// - `*` does not cross `/`; `**` does.
#[derive(Debug)]
pub struct ExcludeSet {
    base: PathBuf,
    /// `base` with symlinks resolved, computed once (`None` if it cannot be resolved).
    canonical_base: Option<PathBuf>,
    by_name: GlobSet,
    by_path: GlobSet,
}

impl ExcludeSet {
    pub fn new(base: &Path, patterns: &[String]) -> Result<Self, String> {
        let mut by_name = GlobSetBuilder::new();
        let mut by_path = GlobSetBuilder::new();
        for raw in patterns {
            let pattern = raw.trim().trim_end_matches('/');
            if pattern.is_empty() {
                continue;
            }
            let build = |p: &str| {
                GlobBuilder::new(p)
                    .literal_separator(true)
                    .build()
                    .map_err(|err| format!("invalid exclude pattern `{raw}`: {err}"))
            };
            if pattern.contains('/') {
                by_path.add(build(pattern.trim_start_matches('/'))?);
            } else {
                by_name.add(build(pattern)?);
            }
        }
        let base = normalized_path(base);
        Ok(Self {
            canonical_base: fs::canonicalize(&base).ok(),
            base,
            by_name: by_name.build().map_err(|err| err.to_string())?,
            by_path: by_path.build().map_err(|err| err.to_string())?,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty() && self.by_path.is_empty()
    }

    /// Whether `abs_path`, or any of its parent directories below the base, is excluded.
    pub fn matches(&self, abs_path: &Path) -> bool {
        if self.is_empty() {
            return false;
        }
        let abs_path = normalized_path(abs_path);
        if abs_path
            .strip_prefix(&self.base)
            .is_ok_and(|rel| self.matches_relative(rel))
        {
            return true;
        }
        // Formatting writes through symlinks, so also check where the file really is: a link
        // inside the base can point into an excluded directory, and an absolute link can
        // resolve out of a symlinked base spelling into the real tree.
        let (Some(canon_base), Ok(canon_abs)) = (&self.canonical_base, fs::canonicalize(&abs_path))
        else {
            return false;
        };
        canon_abs
            .strip_prefix(canon_base)
            .is_ok_and(|rel| self.matches_relative(rel))
    }

    /// Whether `rel` (relative to the base), or any of its parent directories, is excluded.
    fn matches_relative(&self, rel: &Path) -> bool {
        let mut prefix = String::new();
        for component in rel.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            let name = name.to_string_lossy();
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(&name);
            if self.by_name.is_match(name.as_ref()) || self.by_path.is_match(&prefix) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(patterns: &[&str]) -> ExcludeSet {
        let patterns: Vec<String> = patterns.iter().map(|p| p.to_string()).collect();
        ExcludeSet::new(Path::new("/repo"), &patterns).unwrap()
    }

    #[test]
    fn exclude_patterns_match_like_gitignore() {
        let s = set(&[
            "CHANGELOG.md",
            "vendor/",
            "docs/api/**",
            "*.gen.md",
            "/TOP.md",
        ]);
        assert!(s.matches(Path::new("/repo/CHANGELOG.md")));
        assert!(s.matches(Path::new("/repo/sub/CHANGELOG.md")));
        assert!(s.matches(Path::new("/repo/vendor/x/README.md")));
        assert!(s.matches(Path::new("/repo/docs/api/ref.md")));
        assert!(s.matches(Path::new("/repo/a/b.gen.md")));
        assert!(s.matches(Path::new("/repo/TOP.md")));
        assert!(!s.matches(Path::new("/repo/sub/TOP.md")));
        assert!(!s.matches(Path::new("/repo/docs/guide.md")));
        assert!(!s.matches(Path::new("/elsewhere/CHANGELOG.md")));
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        let err = toml::from_str::<Settings>("max_line_widht = 80").unwrap_err();
        assert!(err.to_string().contains("max_line_widht"), "{err}");
        assert!(toml::from_str::<Settings>("wrap = \"sometimes\"").is_err());
        let ok: Settings =
            toml::from_str("wrap = \"sentence\"\nordered_lists = \"ascending\"").unwrap();
        assert_eq!(ok.wrap, Some(WrapMode::Sentence));
        assert_eq!(ok.ordered_lists, Some(OrderedListStyle::Ascending));
    }
}
