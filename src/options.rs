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

//! Formatting options.

/// The line ending used in formatted output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LineEnding {
    /// Use the input's predominant line ending (`\n` if it has none, or on a tie).
    #[default]
    Auto,
    Lf,
    CrLf,
}

impl LineEnding {
    /// The name used for this setting in `.editorconfig` (`end_of_line`) and `--print-config`.
    #[doc(hidden)]
    pub fn name(self) -> &'static str {
        match self {
            LineEnding::Auto => "auto",
            LineEnding::Lf => "lf",
            LineEnding::CrLf => "crlf",
        }
    }

    /// The concrete line ending to use when formatting `input`.
    pub(crate) fn resolve(self, input: &str) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Auto => {
                // Use the file's predominant style, so a mostly-CRLF file with a stray LF line
                // (or vice versa) is normalized toward what it mostly is.
                let crlf = input.matches("\r\n").count();
                let lf = input.matches('\n').count() - crlf;
                if crlf > lf {
                    "\r\n"
                } else {
                    "\n"
                }
            }
        }
    }
}

/// How paragraph text is broken into lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WrapMode {
    /// Reflow paragraphs to fill lines up to the maximum width.
    #[default]
    Always,
    /// Join each paragraph onto a single line (hard breaks are kept).
    Never,
    /// Keep the existing line breaks; only normalize whitespace and indentation.
    Preserve,
    /// Start each sentence on a new line (semantic line breaks), wrapping sentences that are
    /// longer than the maximum width.
    Sentence,
}

/// How ordered list items are numbered. Only the first item's number affects rendering (it sets
/// the list's start), so every style keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OrderedListStyle {
    /// Keep each item's number as written.
    #[default]
    Preserve,
    /// Number items consecutively from the first item's number (`1. 2. 3.`).
    Ascending,
    /// Give every item the first item's number (`1. 1. 1.`).
    One,
}

/// `--wrap` / `wrap =` spelling of [`WrapMode`]. Kept separate so the public enum carries no
/// `clap` or `serde` traits; the exhaustive conversions both ways catch a variant added to
/// either side.
#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum WrapModeName {
    /// Reflow paragraphs to fill lines up to the maximum width.
    Always,
    /// Join each paragraph onto a single line (hard breaks are kept).
    Never,
    /// Keep the existing line breaks; only normalize whitespace and indentation.
    Preserve,
    /// Start each sentence on a new line (semantic line breaks), wrapping sentences that are
    /// longer than the maximum width.
    Sentence,
}

impl From<WrapModeName> for WrapMode {
    fn from(name: WrapModeName) -> Self {
        match name {
            WrapModeName::Always => WrapMode::Always,
            WrapModeName::Never => WrapMode::Never,
            WrapModeName::Preserve => WrapMode::Preserve,
            WrapModeName::Sentence => WrapMode::Sentence,
        }
    }
}

impl From<WrapMode> for WrapModeName {
    fn from(mode: WrapMode) -> Self {
        match mode {
            WrapMode::Always => WrapModeName::Always,
            WrapMode::Never => WrapModeName::Never,
            WrapMode::Preserve => WrapModeName::Preserve,
            WrapMode::Sentence => WrapModeName::Sentence,
        }
    }
}

/// `--ordered-lists` / `ordered_lists =` spelling of [`OrderedListStyle`] (see
/// [`WrapModeName`]).
#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OrderedListStyleName {
    /// Keep each item's number as written.
    Preserve,
    /// Number items consecutively from the first item's number (`1. 2. 3.`).
    Ascending,
    /// Give every item the first item's number (`1. 1. 1.`).
    One,
}

impl From<OrderedListStyleName> for OrderedListStyle {
    fn from(name: OrderedListStyleName) -> Self {
        match name {
            OrderedListStyleName::Preserve => OrderedListStyle::Preserve,
            OrderedListStyleName::Ascending => OrderedListStyle::Ascending,
            OrderedListStyleName::One => OrderedListStyle::One,
        }
    }
}

impl From<OrderedListStyle> for OrderedListStyleName {
    fn from(style: OrderedListStyle) -> Self {
        match style {
            OrderedListStyle::Preserve => OrderedListStyleName::Preserve,
            OrderedListStyle::Ascending => OrderedListStyleName::Ascending,
            OrderedListStyle::One => OrderedListStyleName::One,
        }
    }
}

/// Options controlling how a document is formatted. Start from [`Default::default`] and
/// mutate individual fields.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FormattingOptions {
    /// Maximum line width in display columns (default 72).
    pub max_line_width: usize,
    /// Exact indent for continuation lines inside list items and definitions (`--hanging-indent`).
    /// `None` aligns continuation lines with the item text.
    pub hanging_indent: Option<usize>,
    /// Minimum continuation indent (from `.editorconfig` `indent_size`); never narrower than the
    /// list marker.
    pub min_hanging_indent: Option<usize>,
    /// Align GFM table columns (default `true`).
    pub align_table_columns: bool,
    /// Use Knuth-Plass optimal line breaking for paragraphs of 4+ lines (default `true`);
    /// `false` always uses greedy wrapping.
    pub optimal: bool,
    /// How paragraph text is broken into lines.
    pub wrap: WrapMode,
    /// How ordered list items are numbered.
    pub ordered_lists: OrderedListStyle,
    /// Preferred editor tab width (default 8), retained for configuration inspection.
    /// Markdown syntax always uses four-column tab stops, independently of this preference.
    pub tab_width: usize,
    /// Line ending of the output.
    pub line_ending: LineEnding,
    /// Whether the output ends with a line ending (`.editorconfig` `insert_final_newline`).
    /// `None` keeps whatever the input does.
    pub insert_final_newline: Option<bool>,
    /// Force Markdeep mode on or off; `None` detects it from the content.
    pub markdeep: Option<bool>,
}

impl Default for FormattingOptions {
    fn default() -> Self {
        Self {
            max_line_width: 72,
            hanging_indent: None,
            min_hanging_indent: None,
            align_table_columns: true,
            optimal: true,
            wrap: WrapMode::Always,
            ordered_lists: OrderedListStyle::Preserve,
            tab_width: 8,
            line_ending: LineEnding::Auto,
            insert_final_newline: None,
            markdeep: None,
        }
    }
}

/// Options given explicitly on the command line, applied after `.editorconfig` and
/// `.markfmt.toml` settings.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub max_line_width: Option<usize>,
    pub hanging_indent: Option<usize>,
    pub align_table_columns: Option<bool>,
    pub optimal: Option<bool>,
    pub wrap: Option<WrapMode>,
    pub ordered_lists: Option<OrderedListStyle>,
}

impl CliOverrides {
    pub fn apply(&self, options: &mut FormattingOptions) {
        if let Some(w) = self.max_line_width {
            options.max_line_width = w.max(1);
        }
        if let Some(h) = self.hanging_indent {
            options.hanging_indent = Some(h);
        }
        if let Some(align) = self.align_table_columns {
            options.align_table_columns = align;
        }
        if let Some(opt) = self.optimal {
            options.optimal = opt;
        }
        if let Some(wrap) = self.wrap {
            options.wrap = wrap;
        }
        if let Some(style) = self.ordered_lists {
            options.ordered_lists = style;
        }
    }
}
