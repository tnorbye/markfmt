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

# Changelog

All notable changes to `markfmt` are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/) for the command line
and the documented library API.

## [Unreleased]

## [1.1.0] - 2026-11-19

### Added

- Commit-message formatting: `--commit-msg` (alias `--commit-message`),
  or automatically for `COMMIT_EDITMSG`, `MERGE_MSG`, `SQUASH_MSG`,
  `TAG_EDITMSG`, `NOTES_EDITMSG`, `EDIT_DESCRIPTION`, and
  `*.jjdescription`. The subject line, trailers, comment lines,
  scissors diffs, and signatures are kept as written, including
  blank lines in detached tails. Nested heading underlines and table
  syntax are kept when formatting would change more than whitespace,
  and Git's trailer interpretation is preserved. Only the body is
  reflowed. `--comment-char` and `--trailer-separators` override Git's
  `core.commentChar`/`core.commentString` and `trailer.separators`,
  and a `[commit_message]` table in `.markfmt.toml` holds settings that
  apply only to commit messages.
- Commit messages are checked after formatting: if formatting would
  change more than whitespace, change the trailers Git parses or the
  Markdown structure, or give a different result on a second pass, the
  message is left as written. Quotes in commit messages are kept as
  written.
- `--commit-editor[=<CMD>]`, for use as `GIT_EDITOR` or `JJ_EDITOR`:
  runs your editor, then formats the message. Formatting problems are
  reported as warnings and never abort the commit.
- `git-clang-format` and `clang-format-diff.py` compatibility:
  `--list-ignored`, `--stdout`, `-i`/`--in-place`, `--assume-filename`,
  the ignored `--style`, `--fallback-style`, and `--sort-includes`
  flags, and single-dash spellings such as `-lines=1:3`.
- Git change filters: `--staged` (alias `--cached`), `--since <REV>`,
  and commits and `<base>..<head>`/`<base>...<head>` ranges in
  `--overlaps-git-changes`. Changed lines from the index, commits, and
  ranges are mapped onto the working tree, and `--lines` combines with
  them.
- `--completions <SHELL>` and `--man`. Release archives now include the
  `markfmt(1)` man page and bash, zsh, fish, PowerShell, and Elvish
  completions, and `make install` installs the man page.

### Changed

- Git change filters with no `<FILES>` format the current directory.
- `--stdin-filepath` (or a clang-format style flag) with no `<FILES>`
  reads standard input.
- The summary counts only files that `--lines` or a git change filter
  selected, and `--diff` headers no longer include `./`, so `git apply`
  accepts them.
- If formatting a file hits an internal error, only that file fails and
  is left unchanged; other files are still formatted. Release binaries
  are built with `panic = "unwind"` for this.

## [1.0.0] - 2026-11-14

Initial release.

[Unreleased]: https://github.com/tnorbye/markfmt/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/tnorbye/markfmt/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/tnorbye/markfmt/releases/tag/v1.0.0
