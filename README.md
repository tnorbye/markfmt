# markfmt

A fast, standalone formatter for Markdown, Markdeep, and AI agent
instruction and skill files (`AGENTS.md`, `GEMINI.md`, `CLAUDE.md`,
`SKILL.md`, ...).

`markfmt` reflows paragraphs to a consistent width, aligns
tables, and tidies lists and headings, while keeping your Markdown
dialect and never changing how a document renders. Point it at
a file or a folder and it formats everything in it, recursively.
It is a Rust reimagining of the `--include-md-files` mode of
[kdoc-formatter](https://github.com/tnorbye/kdoc-formatter), and
compiles to a single native binary with no runtime dependencies.

## Quick Start

Install the latest release on macOS or Linux (see
[Installation](#installation) for Windows, Cargo, and source builds):

```sh
curl -LsSf https://github.com/tnorbye/markfmt/releases/latest/download/install.sh | sh
```

Then format every Markdown file in the current folder and all folders
below it:

```sh
markfmt .
```

```text
3 files reformatted, 41 files unchanged
```

Preview what would change without touching any files, or check
formatting in CI:

```sh
markfmt --diff .
markfmt --check .
```

## Example

Given this `SKILL.md`:

```markdown
---
name: release-notes
description: Drafts release notes from merged pull requests.
---

# Release   notes

When the user asks for release notes, collect the merged pull requests since the last tag and group them by area.
Keep each entry to one line,
and link the pull request.

* Run `git log --oneline v1.2.0..HEAD` to list the commits since the last release, then read each pull request description.
* Group entries under **Features**, **Fixes**, and **Docs**.

| Area | Label |
|-|:-:|
| CLI | `area:cli` |
| Documentation | `area:docs` |

<instructions>
Never invent pull request numbers.   If a commit has no pull request, list its hash instead.
</instructions>
```

`markfmt SKILL.md` produces:

```markdown
---
name: release-notes
description: Drafts release notes from merged pull requests.
---

# Release notes

When the user asks for release notes, collect the merged pull requests
since the last tag and group them by area. Keep each entry to one line,
and link the pull request.

* Run `git log --oneline v1.2.0..HEAD` to list the commits since the
  last release, then read each pull request description.
* Group entries under **Features**, **Fixes**, and **Docs**.

| Area          | Label       |
| ------------- | :---------: |
| CLI           | `area:cli`  |
| Documentation | `area:docs` |

<instructions>
Never invent pull request numbers. If a commit has no pull request, list
its hash instead.
</instructions>
```

The front matter is untouched, paragraphs are reflowed to 72 columns,
the wrapped list item is indented under its text, the table is aligned
(keeping its centered column), and the prose inside `<instructions>`
is reflowed while the tags stay on their own lines. The `*` bullets are
kept rather than converted to `-`. With `--wrap sentence`, the first
paragraph instead gets one sentence per line:

```markdown
When the user asks for release notes, collect the merged pull requests
since the last tag and group them by area.
Keep each entry to one line, and link the pull request.
```

## Features

### Safe, Syntax-Preserving Formatting

- **Never changes how a document renders**: Every change keeps the
  rendered HTML the same; this is checked by round-trip tests over
  generated documents.
- **Preserves your Markdown dialect**: Keeps your choice of bullet
  marker (`-`, `*`, `+`), ATX (`#`) vs. Setext (`===`/`---`) headings,
  ordered list numbers, emphasis delimiters (`*` vs. `_`), tight vs.
  loose lists, and hard line breaks (a trailing backslash `\` or two
  trailing spaces).
- **Line endings and the end of the file stay as they were**: CRLF
  (Windows) files stay CRLF and LF files stay LF (a file with mixed
  endings is normalized to its predominant style), a UTF-8 byte order
  mark is kept, and a file that doesn't end with a newline still won't
  afterwards. Extra blank lines at the end of a file are removed, like
  other runs of blank lines.
- **Code and Mermaid blocks stay untouched**: Fenced (```` ``` ````,
  `~~~`) and 4-space indented code blocks—including ```` ```mermaid ````
  diagrams—HTML blocks, display math (`$$...$$`), and `[id]: url` link
  reference definitions are preserved byte-for-byte.
- **GFM table alignment**: Aligns table columns with uniform 1-space
  cell padding (`| col |`), Unicode display-width measurement, and
  escaped `\|` pipes (disable with `--no-align-table-columns`). Cells
  are split exactly as GFM does, so a `|` inside a code span starts a
  new cell unless written as `\|`; `markfmt` warns about such pipes and
  leaves tables whose rows have more cells than the header unchanged.
- **Optional list renumbering**: `--ordered-lists ascending` numbers
  items consecutively (`1. 2. 3.`) and `--ordered-lists one` gives every
  item the first item's number (`1. 1. 1.`). The first number, which
  sets where the list starts, is always kept, and continuation lines are
  re-aligned when a marker changes width.

### Smart Paragraph Reflowing

The default line width is **72** columns (`--max-line-width`):

- **Short paragraphs (1–3 lines)** use greedy line wrapping so brief
  notes fill early lines naturally instead of introducing awkward
  splits.
- **Longer paragraphs (4+ lines)** use Knuth-Plass optimal line
  balancing to minimize raggedness across lines `1..N-1` (with zero
  slack penalty on the final line). Pass `--greedy` to always use greedy
  wrapping.
- **Wrap modes** (`--wrap`):
  - `always` (default): reflow paragraphs to fill lines.
  - `never`: join each paragraph onto one line.
  - `preserve`: keep existing line breaks; only normalize spacing and
    indentation.
  - `sentence`: start each sentence on its own line ("semantic line
    breaks"), wrapping sentences longer than the line width. Common
    abbreviations (`e.g.`, `i.e.`, `Dr.`) and initials don't end a
    sentence.
- **Overflow isolation**: An unbreakable long URL is placed on its own
  line without preventing optimal balancing on surrounding lines.
- **Line-start hazard avoidance**: Never moves a word like `1.`, `#`,
  `-`, `=`, `>`, `---`, `|-|`, `<div>`, or `[^1]:` to the start of a
  line, where CommonMark would re-parse it as a list item, heading,
  blockquote, table, HTML block, or footnote, and never ends a line with
  a literal `\` that would become a hard line break.
- **Smart inline handling**:
  - Inline `` `code spans` ``, `$math$` (distinguished from currency
    like `$100 to $200`), `<autolinks>`, `@[label](path)`, and
    `[[wikilinks]]` stay intact.
  - Short multi-word links (`[link text](url)`) are kept unbroken across
    lines unless wrapping inside `[...]` is needed to avoid overflow or
    an excessively short line, keeping `lastWord](destination)` atomic.
  - Unencoded space-containing paths in links or images
    (such as `[screenshot](file://My Files/foo.png)` or
    `![diagram](./My Docs/arch v2.png)`) are kept unbroken **as
    written** without rewriting spaces to `%20`.

### First-Class AI Agent & `SKILL.md` Support

Agent instruction files and skills combine standard Markdown with
metadata and XML structure that naive formatters often break:

- **Front matter preservation**: YAML (`---`) and TOML (`+++`) front
  matter at the top of `SKILL.md` or documentation files is kept
  verbatim, followed by a single normalized blank line.
- **Prompt XML tags & imports**: Standalone custom XML tags commonly
  used in agent prompts (`<IMPORTANT>`, `<instructions>`, `<example>`,
  `<rules>`, `<details>`, `<summary>...</summary>`) and standalone
  `@path/to/file.md` imports are preserved on their own lines while
  inner Markdown prose reflows cleanly instead of being swallowed by
  CommonMark HTML Block Rule 7.
- **Hidden agent directory discovery**: Directory walks automatically
  enter non-VCS hidden directories such as `.agents/`, `.agent/`,
  `.claude/`, `.gemini/`, `.codex/`, and `.github/` while still skipping
  `.git/`, `.hg/`, `.svn/` and respecting `.gitignore` (override with
  `--no-ignore`).
- **Symlink-safe writes**: Common setups like `CLAUDE.md -> AGENTS.md`
  follow symlinks, deduplicate by canonical path, and write directly
  to the canonical target so symlinks are never replaced with regular
  files.

### Markdeep (`.md.html`) Support

Files ending in `.md.html` or containing `markdeep.min.js` /
`<!-- Markdeep:` markers automatically enable Markdeep mode:

- Preserves top `**Title**` and indented author header blocks verbatim.
- Preserves `****+` ASCII box diagrams (and adjacent `[Figure ...]`
  captions), `$$...$$` display math, and `<script>` Markdeep footers
  verbatim.
- Recursively reflows `!!!` admonition bodies and `:   ` definition
  lists while maintaining their 4-space indentation.

### Workflow Features

- **Recursive folder formatting** with `.gitignore` support and
  [exclude patterns](#configuration-file).
- **Partial formatting**: only the blocks touched by uncommitted,
  staged, or last-commit git changes, or by given line ranges.
- **`--check`, `--diff`, and `--dry-run`** for CI and previews, with
  colored, word-highlighted diffs on terminals.
- **Configuration** from `.editorconfig` and `.markfmt.toml`, plus
  [inline ignore directives](#inline-ignore-directives).
- **[pre-commit](#pre-commit) hooks**, stdin/stdout mode for editors,
  and parallel formatting of large repositories.

## Usage

```text
markfmt [OPTIONS] <FILES>...
```

`<FILES>` can be any mix of files and folders; folders are searched
recursively. Use `-` to read from stdin and write to stdout.

### Formatting Files and Folders

Format every Markdown, agent/skill, and Markdeep file in the current
folder and everything below it:

```sh
markfmt .
```

Format specific files or folders:

```sh
markfmt README.md docs/
```

It prints a one-line summary such as
`2 files reformatted, 46 files unchanged`; add `-v` to also list each
reformatted file, or `-q` for no output.

When given folders, `markfmt` formats:

- `.md`, `.markdown`, `.mdown`, `.mkd`, `.mkdn`, `.mdwn`, and `.md.html`
  files (skipping `.mdx`).
- Extensionless `README`, `CHANGELOG`, `CONTRIBUTING`, `HACKING`,
  `SECURITY`, `PULL_REQUEST_TEMPLATE`, `RELEASE_NOTES`, and `NEWS` files
  when conservative content sniffing confirms Markdown (rejecting binary
  files, plain text, reStructuredText, AsciiDoc, and roff/man pages).
- License and notice files (`LICENSE*`, `COPYING*`, `NOTICE*`,
  `PATENTS*`) are skipped during folder walks, but formatted if passed
  explicitly on the command line.

Files matched by `.gitignore` are skipped (`--no-ignore` includes them),
and so are files matching [exclude patterns](#configuration-file).

### Checking and Previewing

Check formatting in CI (exits with status `1` if any files need
changes):

```sh
markfmt --check .
```

This lists each file that would change
(`Would reformat <path>`) followed by a summary such as
`2 files would be reformatted, 46 files already formatted`, all on
stderr.

Preview changes as a unified diff, or list the files that would change,
without modifying anything:

```sh
markfmt --diff docs/
markfmt --dry-run .
```

On a color terminal, `--diff` colors removed and added lines and
highlights the words whose text changed; a paragraph that was only
rewrapped shows no highlighted words. When output is redirected it is
a plain unified diff that `patch` and `git apply` accept. Errors and
warnings are colored too. Color follows `--color auto|always|never`
(default `auto`) and honors the `NO_COLOR`, `CLICOLOR`, and
`CLICOLOR_FORCE` environment variables. To page a colored diff, use
`markfmt --diff --color always . | less -R`.

### Formatting Only What Changed

Format only the blocks touched by git changes:

- `uncommitted`: everything not yet committed, staged or not
  (`git diff HEAD`), plus new untracked files.
- `staged`: staged changes only (`git diff --cached`).
- `HEAD`: the most recent commit (`git show HEAD`).

```sh
markfmt --overlaps-git-changes=uncommitted .
markfmt --overlaps-git-changes=staged .
markfmt --overlaps-git-changes=HEAD .
```

Format only blocks overlapping lines `10..=25` of a single file:

```sh
markfmt --lines 10:25 README.md
```

### Editors and Scripts (stdin)

Format via stdin/stdout, with an optional virtual path for
`.editorconfig`, `.markfmt.toml`, and `.md.html` detection. Only the
formatted document is printed, never a summary:

```sh
cat guide.md | markfmt --stdin-filepath docs/guide.md -
```

You can also pass `@<filename>` to read newline-separated arguments and
paths from a file.

### pre-commit

To run `markfmt` from [pre-commit](https://pre-commit.com), add this to
`.pre-commit-config.yaml`, replacing `rev` with the release tag you use:

```yaml
repos:
  - repo: https://github.com/tnorbye/markfmt
    rev: v1.0.0
    hooks:
      - id: markfmt # or markfmt-check to only verify
```

pre-commit builds the hook with `cargo install`, so Rust must be
installed.

## Configuration

Settings are applied in this order, with later ones winning: defaults,
`.editorconfig`, `.markfmt.toml`, then command-line flags.

### `.editorconfig`

`markfmt` reads `.editorconfig` files walking upward from each target
file to `root = true`:

- `max_line_length` (`off`/`unset` resets to `72`) and `indent_size`
  are applied **only** from Markdown/documentation-specific sections
  (such as `[*.md]` or `[*.{md,markdown}]`), ignoring sections that also
  match code files, so code widths don't leak into prose. That includes
  catch-all `[*]` and directory-wide sections like `[docs/**]`; to set a
  width for Markdown in one directory, use `[docs/**.md]`.
- `indent_size` is a **minimum** continuation indent for list items:
  wrapped lines are never indented less than the list marker, so with
  `indent_size = 2` a `1.` item still wraps with 3 spaces. Indents are
  also capped at 3 columns past the item text, since more would turn
  nested blocks into code blocks.
- `end_of_line` (`lf` / `crlf`) and `insert_final_newline` (`true` adds
  a missing final newline, `false` removes it; unset keeps whatever the
  file has) also apply from `[*]` sections.

Markdown syntax uses CommonMark's fixed four-column tab stops. The
editor's `tab_width` preference is recorded for configuration inspection
and does not change how tabs affect Markdown structure.

Use `--no-editorconfig` to ignore `.editorconfig` files.

### Configuration File

Settings that `.editorconfig` can't express go in a `.markfmt.toml`.
For each file, the nearest one in the file's directory or an ancestor
applies (files are not merged). Every key is optional, and unknown keys
are an error:

```toml
max_line_width = 80
hanging_indent = 2          # exact continuation indent (as --hanging-indent)
align_table_columns = true
greedy = false
wrap = "sentence"           # always | never | preserve | sentence
ordered_lists = "ascending" # preserve | ascending | one

# Files to skip, relative to this file's directory:
exclude = ["CHANGELOG.md", "vendor/", "docs/generated/**"]
```

Exclude patterns work like `.gitignore`: a pattern without `/` matches a
file or directory name at any depth, a pattern with `/` matches the path
relative to the config file, matching a directory excludes everything
in it, and `*` doesn't cross `/` while `**` does. Excludes also apply
to files named explicitly on the command line (as pre-commit does).
`--exclude <PATTERN>` adds patterns relative to the current directory,
`--config <PATH>` uses one config file for everything, and `--no-config`
ignores config files.

### Inspecting Configuration and Discovery

Print the effective formatting settings for a file, including
command-line overrides, without changing it:

```sh
markfmt --print-config README.md
markfmt --print-config --stdin-filepath docs/guide.md -
```

Each file gets its own TOML settings block with comments identifying the
configuration source and effective EditorConfig preferences. Stdin is
not read in this mode. Exclude patterns are inspected with `--explain`.

Explain which files are included or skipped, and which configuration
file applies, without formatting them:

```sh
markfmt --explain docs/
```

This reports ignore rules, unsupported file types, duplicate targets,
and project or command-line excludes. To identify ignored files,
diagnostic discovery also visits ignored directories. It continues to
skip VCS metadata directories.

Use `--optimal` to override `greedy = true` in configuration. When both
`--greedy` and `--optimal` are supplied, the last one wins.

### Inline Ignore Directives

Disable formatting around specific blocks using HTML comments:

```markdown
<!-- markfmt:off -->
Custom   spacing   and   long   unwrapped   lines   kept   as-is.
<!-- markfmt:on -->

<!-- prettier-ignore -->
Single   following   block   kept   as-is.

<!-- markfmt-ignore-next -->
Same as prettier-ignore: the next block is kept as-is.
```

A `markfmt:off` comment without a matching `markfmt:on` leaves the rest
of the file unformatted.

## Command-Line Reference

### Options

| Flag                            | Description                                                                  |
| :------------------------------ | :--------------------------------------------------------------------------- |
| `--max-line-width <N>`          | Maximum line width (`72`; aliases `--line-width`, `--right-margin`)          |
| `--hanging-indent <N>`          | Override continuation and nested list indent spaces                          |
| `--no-align-table-columns`      | Leave GFM table column spacing unchanged (`--align-table-columns` forces on) |
| `--greedy`                      | Use greedy line breaking instead of optimal Knuth-Plass balancing            |
| `--optimal`                     | Force optimal line breaking, overriding `greedy = true` in configuration     |
| `--print-config`                | Print effective settings and configuration sources without modifying files   |
| `--explain`                     | Explain included and skipped inputs without modifying files                  |
| `--wrap <MODE>`                 | `always` (default), `never`, `preserve`, or `sentence`                       |
| `--ordered-lists <STYLE>`       | `preserve` (default), `ascending`, or `one`                                  |
| `--config <PATH>`               | Use this config file instead of searching for `.markfmt.toml`                |
| `--no-config`                   | Do not read `.markfmt.toml` files                                            |
| `--exclude <PATTERN>`           | Skip matching files (relative to the current directory; repeatable)          |
| `--no-editorconfig`             | Do not read `.editorconfig` files                                            |
| `--no-ignore`                   | Do not respect `.gitignore` when traversing directories                      |
| `--filter-files`                | Apply Markdown discovery rules to explicitly named files                     |
| `--overlaps-git-changes <MODE>` | Only format blocks overlapping `uncommitted`, `staged`, or `HEAD` changes    |
| `--git-path <PATH>`             | Explicit path to the `git` executable                                        |
| `--lines <START:END>`           | Only format blocks overlapping 1-based line range(s) on a single file        |
| `--stdin-filepath <PATH>`       | Virtual path for configuration and Markdeep detection when reading `-`       |
| `-n`, `--dry-run`               | Print paths of files that would change without modifying them                |
| `--check`                       | Verify formatting without modifying files; exit `1` if changes are needed    |
| `--diff`                        | Print a unified diff of formatting changes without modifying files           |
| `--color <WHEN>`                | Color output and diffs: `auto` (default, terminals only), `always`, `never`  |
| `-q`, `--quiet`                 | Suppress summary output                                                      |
| `-v`, `--verbose`               | Also list each reformatted file                                              |
| `-h`, `--help`                  | Print help information (including the embedded version)                      |
| `-V`, `--version`               | Print version information                                                    |
| `@<filename>`                   | Read newline-separated arguments/paths from `<filename>`                     |

### Exit Codes

| Code | Meaning                                                                        |
| :--- | :----------------------------------------------------------------------------- |
| `0`  | Success (with `--check`: no files need formatting)                             |
| `1`  | `--check` found files that need formatting                                     |
| `2`  | Error, such as an invalid argument or a file that could not be read or written |

Files that cannot be read or written are reported and skipped; the
remaining files are still processed, and the run exits with `2`.

## Installation

### Prebuilt Binaries

**macOS and Linux** (downloads the matching release archive into
`~/.local/bin/markfmt` and verifies its SHA-256 checksum):

```sh
curl -LsSf https://github.com/tnorbye/markfmt/releases/latest/download/install.sh | sh
```

To install into a different directory or pin a specific version:

```sh
curl -LsSf https://github.com/tnorbye/markfmt/releases/download/v1.0.0/install.sh | sh -s -- --dir /usr/local/bin
```

You can also pass `-v` / `MARKFMT_VERSION` or set `MARKFMT_INSTALL_DIR`.

**Windows (PowerShell):**

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/tnorbye/markfmt/releases/latest/download/install.ps1 | iex"
```

Prebuilt archives for Linux (static, `x86_64` and `aarch64`), macOS
(`aarch64` and `x86_64`), and Windows (`x86_64`) are also attached to
each [GitHub Release](https://github.com/tnorbye/markfmt/releases).

### Cargo

Install from [crates.io](https://crates.io/crates/markfmt) (requires
Rust 1.88 or newer):

```sh
cargo install markfmt
```

Or download the prebuilt GitHub Release binary via
[cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```sh
cargo binstall markfmt
```

### From Source

Build and install into `~/.local/bin/markfmt` from a local checkout
(requires Rust 1.88 or newer):

```sh
make install
```

Override `PREFIX` to install elsewhere (for example, `/usr/local/bin`):

```sh
sudo make install PREFIX=/usr/local
```

Remove an installed binary with `make uninstall` (passing the
same `PREFIX` if overridden). You can also install from a checkout
with `cargo install --path .`, or build the standalone binary at
`target/release/markfmt` with `cargo build --release`.

### Using the Library

The formatter is also a Rust library
([`markfmt` on crates.io](https://crates.io/crates/markfmt)); see the
[API documentation on docs.rs](https://docs.rs/markfmt) (or build it
locally with `make doc`) for an example of calling `format_markdown`
with `FormattingOptions`.

## Building & Development

`markfmt` is built with **Cargo** (Rust's standard build tool
and package manager). A convenience [Makefile](Makefile) and
[.cargo/config.toml](.cargo/config.toml) are also included to provide
Gradle-style targets:

| Task                 | `make` target    | Underlying `cargo` command                             |
| :------------------- | :--------------- | :----------------------------------------------------- |
| Format Rust & docs   | `make fmt`       | `cargo fmt --all && cargo run -- .`                    |
| Verify formatting    | `make fmt-check` | `cargo fmt --all -- --check && cargo run -- --check .` |
| Lint                 | `make lint`      | `cargo clippy --all-targets -- -D warnings`            |
| Run tests            | `make test`      | `cargo test`                                           |
| Build API docs       | `make doc`       | `cargo doc --no-deps` (warnings are errors)            |
| Full check (`check`) | `make check`     | Runs `fmt-check`, `lint`, `test`, and `doc`            |
| Build release binary | `make release`   | `cargo build --release`                                |
| License notices      | `make licenses`  | `cargo about generate` (see [Licenses](#license))      |
| Package distribution | `make dist`      | Builds release archive in `target/dist/`               |
| Install locally      | `make install`   | Installs release binary to `~/.local/bin` (`PREFIX`)   |
| Uninstall            | `make uninstall` | Removes binary from `$(PREFIX)/bin`                    |

See [AGENTS.md](AGENTS.md) for the project's invariants, code map, and
conventions.

## Versioning & Releases

Within a major version, existing command-line flags and configuration
settings remain supported, and documented exit-code meanings remain
stable. Formatting output may change as bugs are fixed and formatting
improves. Pin an exact version when reproducible formatting is required,
and use the same version locally and in CI.

- **Where the version is defined**: The single source of truth is
  `version = "1.0.0"` under `[package]` in [Cargo.toml](Cargo.toml).
- **Compile-time version embedding**: During compilation,
  Cargo passes `CARGO_PKG_VERSION` to `rustc`. `markfmt` embeds
  `env!("CARGO_PKG_VERSION")` into the binary ([src/lib.rs](src/lib.rs)
  and [src/cli.rs](src/cli.rs)), so both `markfmt --version`
  and `markfmt --help` (and `-h`) display the baked-in version
  (`markfmt 1.0.0`).

### Building Distributions

- **Standalone release binary (`make release` or
  `cargo build --release`)**: Writes the stripped executable to
  `target/release/markfmt` (`target\release\markfmt.exe` on Windows).
- **Local distribution archive (`make dist`)**: Builds the release
  binary for the host platform (or a specific Rust target via
  `make dist TARGET=<rust-triple>`) and packages `markfmt`, `README.md`,
  `LICENSE`, and `THIRD-PARTY-LICENSES.txt` into `target/dist/` (Cargo's
  equivalent of Gradle's `build/distributions/`). Requires `cargo-about`
  (`cargo install --locked cargo-about --features cli`):
  - `target/dist/markfmt-v<version>-<target>.tar.gz`
  - `target/dist/markfmt-v<version>-<target>.tar.gz.sha256`
- **Cross-platform CI distributions (`.github/workflows/release.yml`)**:
  - Pushing a `vX.Y.Z` tag builds archives for Linux (`x86_64`,
    `aarch64`; statically linked against musl, so they run on any
    distribution), macOS (`aarch64`, `x86_64`), and Windows (`x86_64`)
    along with `install.sh`, `install.ps1`, and `SHA256SUMS.txt` and
    attaches them to the **GitHub Releases** page for that tag. Tags
    with a pre-release suffix (such as `v1.1.0-rc1`) are published as
    pre-releases.
  - Triggering **Actions → Release → Run workflow**
    (`workflow_dispatch`) builds the same archives (staged under `dist/`
    on each runner) and uploads them as downloadable **Workflow Run
    Artifacts** (`binary-<target>`) without creating a public release.

### Cutting a Release

To bump the version and publish a new release to GitHub and crates.io:

1. Update `version = "X.Y.Z"` under `[package]` in
   [Cargo.toml](Cargo.toml).
2. Run `make check` to update `Cargo.lock` and verify formatting, lints,
   and tests.
3. Commit the changes (`git commit -am "Bump version to X.Y.Z"`).
4. Create and push a matching `v`-prefixed tag:

```sh
git tag vX.Y.Z
git push origin main --tags
```

Pushing a `v*` tag triggers `.github/workflows/release.yml`,
which verifies that the tag matches `Cargo.toml`, runs the full
verification suite (including a `cargo publish --dry-run`),
and publishes the archives and `SHA256SUMS.txt` to the GitHub
Release. Only after that succeeds does it publish the crate
to [crates.io](https://crates.io/crates/markfmt), using
[trusted publishing](https://crates.io/docs/trusted-publishing) instead
of a stored token; [docs.rs](https://docs.rs/markfmt) then builds the
API documentation automatically.

A crates.io version can be yanked but never deleted or replaced, so
check the tag before pushing it. If the crates.io job fails after the
GitHub Release was created (for example, a network error), re-run just
that job from the workflow run page.

Trusted publishing is configured once on crates.io, under the crate's
**Settings → Trusted Publishing**: add a GitHub publisher for repository
`tnorbye/markfmt` with workflow `release.yml`.

## License

Copyright 2026 Tor Norbye. Licensed under the
[Apache License, Version 2.0](LICENSE).

All dependencies are available under permissive licenses
compatible with Apache-2.0 (MIT, Apache-2.0, Unlicense, and
Unicode-3.0). Release archives include `THIRD-PARTY-LICENSES.txt`
with the license text and copyright notices of every crate
compiled into the binary, generated from `Cargo.lock` by
[cargo-about](https://github.com/EmbarkStudios/cargo-about)
(`make licenses`). The accepted licenses are listed in
[about.toml](about.toml), and CI fails if a dependency isn't available
under one of them.

See [kdoc-formatter](https://github.com/tnorbye/kdoc-formatter) for the
original Kotlin/KDoc formatter that inspired this project.
