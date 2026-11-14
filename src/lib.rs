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

//! A formatter for Markdown, Markdeep, and AI agent instruction files.
//!
//! `markfmt` reflows paragraphs and normalizes block layout while preserving the document's
//! Markdown dialect (bullet and emphasis markers, heading style, list numbering) and never
//! changing how it renders. It is primarily a command-line tool; this library exposes the
//! formatter so it can be embedded in other tools.
//!
//! # Example
//!
//! ```
//! use markfmt::{format_markdown, FormattingOptions, WrapMode};
//!
//! let input = "A   paragraph that is long enough to be wrapped onto several lines.\n";
//! let mut options = FormattingOptions::default();
//! options.max_line_width = 30;
//! assert_eq!(
//!     format_markdown(input, &options),
//!     "A paragraph that is long\nenough to be wrapped onto\nseveral lines.\n"
//! );
//!
//! // One sentence per line ("semantic line breaks").
//! let mut options = FormattingOptions::default();
//! options.wrap = WrapMode::Sentence;
//! assert_eq!(
//!     format_markdown("First sentence. Second one.\n", &options),
//!     "First sentence.\nSecond one.\n"
//! );
//! ```
//!
//! [`format_markdown`] formats a whole document using the given [`FormattingOptions`].
//!
//! # API stability
//!
//! The items documented here are the supported, semver-stable API. The crate also has
//! hidden public modules that the `markfmt` command-line tool and its tests use (argument
//! parsing, file discovery, configuration loading, and so on); they are implementation
//! details and can change in any release.

// Internal modules shared with the `markfmt` binary and the integration tests. They are
// `pub` only because the binary is a separate crate; they are not part of the stable API.
#[doc(hidden)]
pub mod cli;
#[doc(hidden)]
pub mod config;
#[doc(hidden)]
pub mod discovery;
#[doc(hidden)]
pub mod editorconfig;
#[doc(hidden)]
pub mod formatter;
pub(crate) mod inline;
#[doc(hidden)]
pub mod lint;
mod markdown;
#[doc(hidden)]
pub mod options;
pub(crate) mod preprocess;
#[doc(hidden)]
pub mod range_filter;
pub(crate) mod reflow;
pub(crate) mod table;

pub use formatter::format_markdown;
pub use options::{FormattingOptions, LineEnding, OrderedListStyle, WrapMode};

/// The version of this crate (for example `1.0.0`).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
