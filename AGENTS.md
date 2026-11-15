<!--
  Copyright 2026 Tor Norbye

  Licensed under the Apache License, Version 2.0 (the "License");
  you may not use this file except in compliance with the License.
  You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

  Unless required by applicable law or agreed to in writing, software
  distributed under the License is distributed on an "AS IS" BASIS,
  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
  See the License for the specific language governing permissions and
  limitations under the License.
-->

# Agent Instructions

`markfmt` is a Rust CLI and library that reflows and formats Markdown,
Markdeep, and AI agent instruction files. See [README.md](README.md) for
user-facing behavior.

## Before Finishing Any Change

Run `make check` and make sure it passes. It runs `cargo doc`
(warnings are errors), `cargo fmt --check`, `markfmt --check .` on this
repository's own Markdown, `cargo clippy --all-targets -D warnings`,
and `cargo test`. If you edited Markdown, run `make fmt` (or
`cargo run -- .`) to format it with the tool itself.

## Invariants

A formatter must never change what a document means. Every formatting
change must keep both of these true:

- **Idempotent**: formatting already-formatted output changes nothing.
- **Rendering-preserving**: the rendered HTML is unchanged, apart from
  whitespace inside text.
- **Line-ending-preserving**: CRLF input gives exactly the
  LF output with CRLF endings, a byte order mark is kept,
  and whether the file ends with a newline doesn't change
  (unless `.editorconfig` sets `insert_final_newline`). See
  [tests/line_endings_test.rs](tests/line_endings_test.rs); add new
  block types to its `CASES`.

[tests/roundtrip_test.rs](tests/roundtrip_test.rs) checks both over
generated paragraphs full of "hazard" words. When you fix a case where
reflow changed a document's structure, add the minimal reproduction
to `line_start_and_end_hazards_round_trip` there, and if the word can
appear in prose, add it to `HAZARDS` too. Words that must not start a
line belong in `can_start_line` in [src/inline.rs](src/inline.rs); the
tokenizer glues them to the previous word so no wrapping strategy can
break before them.

## Code Map

- [src/main.rs](src/main.rs): CLI driver (job setup, parallel
  formatting, atomic writes).
- [src/report.rs](src/report.rs): terminal output (summaries, styled
  errors and warnings, colored word-highlighted diffs). Messages go
  through `anstream`; formatted document content never does, so it is
  written byte-for-byte.
- [src/cli.rs](src/cli.rs): `clap` definitions and `@file` argument
  expansion.
- [src/discovery.rs](src/discovery.rs): directory walking and Markdown
  file detection.
- [src/editorconfig.rs](src/editorconfig.rs): `.editorconfig`
  resolution.
- [src/config.rs](src/config.rs): `.markfmt.toml` loading and `exclude`
  pattern matching.
- [src/lint.rs](src/lint.rs): warnings that never change output (such as
  `|` inside a code span in a table).
- [src/range_filter.rs](src/range_filter.rs): `--lines` and
  `--overlaps-git-changes` filtering.
- [src/preprocess.rs](src/preprocess.rs): splits a document into
  segments the CommonMark parser should not see directly (frontmatter,
  ignore directives, display math, Markdeep constructs, prompt tags).
- [src/formatter.rs](src/formatter.rs): formats each top-level block
  from the `pulldown-cmark` AST, recursing into lists, blockquotes, and
  footnotes.
- [src/markdown.rs](src/markdown.rs): shared parser settings, syntax tab
  stops, and original soft-break boundaries.
- [src/inline.rs](src/inline.rs) and [src/reflow.rs](src/reflow.rs):
  paragraph tokenization and line breaking (greedy, Knuth-Plass, and the
  `--wrap` modes).
- [src/table.rs](src/table.rs): GFM table alignment.

## Conventions

- Every new source, config, workflow, and Markdown file starts with the
  Apache 2.0 license header used by the existing files.
- Tests are integration tests in [tests/](tests/). Formatting tests
  call `format_markdown` with explicit `FormattingOptions`, compare
  against the expected output, and assert that a second pass leaves
  the output unchanged. Use `tests/common` helpers (`assert_format`,
  `assert_round_trip`, and `Project`) for shared assertions and isolated
  CLI fixtures. Rendering comparisons normalize only ordinary text
  whitespace; code, math, raw text, and HTML attributes remain exact.
- New formatting options go in `FormattingOptions`, `CliOverrides`, the
  `.markfmt.toml` `Settings`, the README, and the round-trip test (run
  it under the new option's values).
- Keep the README's options table and exit codes in sync with
  [src/cli.rs](src/cli.rs) and [src/main.rs](src/main.rs).
- `rust-version` in [Cargo.toml](Cargo.toml) and the `msrv` job in
  [.github/workflows/ci.yml](.github/workflows/ci.yml) must match. Raise
  both if a dependency update requires a newer Rust.
- Every dependency must be available under a license accepted in
  [about.toml](about.toml) (MIT, Apache-2.0, Unlicense, or Unicode-3.0);
  the CI `licenses` job enforces this. Don't add a license there without
  checking it is compatible with Apache-2.0 and with distributing
  binaries.
- GitHub Actions are pinned to commit SHAs with a version comment;
  Dependabot updates them. Don't replace a pin with a movable tag.
- Releases come from pushing a `vX.Y.Z` tag that matches `version`
  in [Cargo.toml](Cargo.toml) (see "Cutting a Release" in
  [README.md](README.md)); the release workflow then publishes to
  GitHub Releases and crates.io. Never push a release tag or run
  `cargo publish` unless the maintainer asks: a crates.io version can be
  yanked but never deleted or replaced.
