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
use std::io::{self, Read, Write};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;

use markfmt::cli::{self, Cli};
use markfmt::config::{Config, ConfigResolver, ExcludeSet};
use markfmt::discovery::{self, InputTarget};
use markfmt::editorconfig::EditorConfigResolver;
use markfmt::formatter;
use markfmt::lint;
use markfmt::options::FormattingOptions;
use markfmt::range_filter::RangeFilter;

mod report;

/// Prints an error and exits with status 2.
fn fail(message: &str) -> ! {
    report::error(message);
    process::exit(2);
}

fn main() {
    let raw_args: Vec<String> = std::env::args().collect();
    let expanded = cli::expand_args(&raw_args).unwrap_or_else(|err| fail(&err));

    let cli = match Cli::try_parse_from(&expanded) {
        Ok(c) => c,
        Err(err) => err.exit(),
    };
    report::apply_color(cli.color);

    let generated = match cli.completions {
        Some(shell) => Some(cli::completions(shell)),
        None if cli.man => Some(cli::man_page()),
        None => None,
    };
    if let Some(bytes) = generated {
        match io::stdout().lock().write_all(&bytes) {
            // A closed pipe (`markfmt --man | head`) is not an error.
            Err(err) if err.kind() != io::ErrorKind::BrokenPipe => {
                fail(&format!("Failed to write to stdout: {err}"))
            }
            _ => return,
        }
    }

    if cli.files.is_empty() {
        eprintln!("Usage: markfmt [OPTIONS] <FILES>...");
        fail("No input files or directories specified.");
    }

    let target_files = if cli.commit_editor.is_some() {
        let allowlisted = run_commit_editor(&cli);
        if allowlisted.is_empty() {
            return;
        }
        allowlisted
    } else {
        cli.files.clone()
    };

    // Reaching here in editor mode means the interactive editor succeeded. Formatting is
    // best effort: a configuration or I/O failure must not abort the user's commit.
    let formatting_failure = |err: &str| -> ! {
        report_formatting_error(&cli, err);
        process::exit(if cli.commit_editor.is_some() { 0 } else { 2 })
    };

    let discovered = discovery::discover_targets_with_options(
        &target_files,
        discovery::DiscoveryOptions {
            no_ignore: cli.no_ignore,
            filter_files: cli.filter_files,
            explain: cli.explain,
        },
    )
    .unwrap_or_else(|err| formatting_failure(&err));
    for warning in &discovered.warnings {
        report::warning(warning);
    }
    for explanation in discovered.explanations {
        println!("{explanation}");
    }
    let targets = discovered.targets;

    if !cli.lines.is_empty() && targets.len() != 1 {
        fail("--lines can only be used with a single input file.");
    }

    let range_filter = if cli.print_config || cli.explain {
        RangeFilter::All
    } else {
        build_range_filter(&cli, &targets).unwrap_or_else(|err| formatting_failure(&err))
    };
    let only_stdin = !targets.is_empty() && targets.iter().all(|t| matches!(t, InputTarget::Stdin));
    let jobs = build_jobs(&cli, &targets).unwrap_or_else(|err| formatting_failure(&err));
    if cli.print_config || cli.explain {
        return;
    }

    // Reading and formatting is independent per file, so it runs in parallel; reporting and
    // writing happen afterwards in input order so output stays deterministic.
    let results = parallel_map(&jobs, |job| {
        let input = match &job.source {
            JobSource::Stdin(input) => input.clone(),
            JobSource::File { write_path } => fs::read_to_string(write_path)
                .map_err(|err| format!("Failed to read `{}`: {err}", job.display_name))?,
        };
        let formatted =
            formatter::format_file_content(&input, &job.filter_path, &range_filter, &job.options);
        let warnings = lint::lint_with_options(&input, &job.options);
        Ok::<_, String>((input, formatted, warnings))
    });

    let mut counts = report::Counts::default();
    for (job, result) in jobs.iter().zip(results) {
        let (input, formatted, warnings) = match result {
            Ok(outcome) => outcome,
            Err(err) => {
                report_formatting_error(&cli, &err);
                counts.failed += 1;
                continue;
            }
        };
        for warning in warnings {
            report::warning(&format!(
                "{}:{}: {}",
                job.display_name, warning.line, warning.message
            ));
        }
        let changed = formatted != input;
        match report_or_write(&cli, job, &input, &formatted, changed) {
            Ok(()) if changed => counts.changed += 1,
            Ok(()) => counts.unchanged += 1,
            Err(err) => {
                report_formatting_error(&cli, &err);
                counts.failed += 1;
            }
        }
    }

    // Formatting stdin (as editors do) or commit messages prints no summary unless `-v` is set.
    let quiet_commit = !jobs.is_empty()
        && jobs.iter().all(|j| j.options.commit_message)
        && !cli.verbose
        && !cli.check;
    if !only_stdin && !cli.quiet && !quiet_commit {
        if cli.check {
            counts.print_check_summary();
        } else if !cli.diff && !cli.dry_run {
            counts.print_formatted_summary();
        }
    }

    if counts.failed > 0 && cli.commit_editor.is_none() {
        process::exit(2);
    }
    if cli.check && counts.changed > 0 {
        process::exit(1);
    }
}

/// Reports a formatting failure after the underlying editor has already succeeded, if any.
fn report_formatting_error(cli: &Cli, message: &str) {
    if cli.commit_editor.is_some() {
        report::warning(&format!("Commit message left as edited: {message}"));
    } else {
        report::error(message);
    }
}

/// One input to format, with its resolved options.
struct Job {
    source: JobSource,
    display_name: String,
    /// Path used to look up line-range filters (canonical for files).
    filter_path: PathBuf,
    options: FormattingOptions,
}

enum JobSource {
    Stdin(String),
    File { write_path: PathBuf },
}

/// Resolves options for every target up front (sequentially, so the `.editorconfig` and
/// `.markfmt.toml` caches are shared), drops excluded files, and reads stdin if requested.
///
/// Option precedence: command-line flags, then `.markfmt.toml`, then `.editorconfig`.
fn build_jobs(cli: &Cli, targets: &[InputTarget]) -> Result<Vec<Job>, String> {
    let cwd =
        std::env::current_dir().map_err(|err| format!("Cannot read current directory: {err}"))?;
    let absolute = |p: &Path| markfmt::config::normalized_path_in(&cwd, p);

    let mut configs = match &cli.config {
        Some(path) => Some(ConfigResolver::with_explicit(&cwd, Path::new(path))?),
        None if cli.no_config => None,
        None => Some(ConfigResolver::new()),
    };
    let cli_excludes = ExcludeSet::new(&cwd, &cli.exclude)?;
    let mut ec_resolver = EditorConfigResolver::new();
    let mut git_commit_config_cache: HashMap<PathBuf, GitCommitConfig> = HashMap::new();
    let overrides = cli.cli_overrides();

    // `.editorconfig` only applies to stdin when `--stdin-filepath` gives it a location.
    let mut resolve = |path: &Path,
                       probe_paths: &[&Path],
                       use_editorconfig: bool,
                       apply_excludes: bool|
     -> Result<(FormattingOptions, bool), String> {
        let abs_path = absolute(path);
        let is_commit = cli.commit_msg
            || probe_paths
                .iter()
                .any(|p| discovery::is_commit_message_path(p));
        let cfg_path = if is_commit {
            commit_config_path(cli.git_path.as_deref(), &cwd, &abs_path)
        } else {
            abs_path.clone()
        };
        let config = match configs.as_mut() {
            Some(resolver) => resolver.for_file(&cfg_path)?,
            None => None,
        };
        let mut options = if use_editorconfig && !cli.no_editorconfig {
            ec_resolver.resolve(&cfg_path)
        } else {
            FormattingOptions::default()
        };
        if is_commit {
            options.commit_message = true;
            options.markdeep = Some(false);
            let commit_config = config
                .as_ref()
                .and_then(|c| c.settings.commit_message.as_ref());
            let need_comment = cli.comment_char.is_none()
                && commit_config.is_none_or(|cm| cm.comment_char.is_none());
            let need_separators = cli.trailer_separators.is_none()
                && commit_config.is_none_or(|cm| cm.trailer_separators.is_none());
            // Git's configured trailer keys are always needed; the cache keeps it to one
            // `git config` call per directory.
            let probe_dir = cfg_path
                .parent()
                .and_then(|d| d.ancestors().find(|a| a.is_dir()))
                .unwrap_or(&cwd)
                .to_path_buf();
            let detected = git_commit_config_cache
                .entry(probe_dir.clone())
                .or_insert_with(|| detect_git_commit_config(cli.git_path.as_deref(), &probe_dir))
                .clone();
            if let Some(prefix) = detected.comment_char.filter(|_| need_comment) {
                options.comment_char = prefix;
            }
            if let Some(separators) = detected.trailer_separators.filter(|_| need_separators) {
                markfmt::config::validate_trailer_separators(&separators)
                    .map_err(|err| format!("Unsupported Git trailer.separators setting: {err}"))?;
                options.trailer_separators = separators;
            }
            options.trailer_keys = detected.trailer_keys;
            if let Some(config) = &config {
                config.apply_commit_message(&mut options);
            }
        } else if let Some(config) = &config {
            config.apply(&mut options);
        }
        overrides.apply(&mut options);
        if !options.commit_message && probe_paths.iter().any(|p| discovery::is_markdeep_path(p)) {
            options.markdeep = Some(true);
        }
        let config_source = config.as_ref().map(|c| c.path.display().to_string());
        let exclusion = if !apply_excludes {
            None
        } else if cli_excludes.matches(&abs_path) {
            Some("--exclude".to_string())
        } else if is_commit {
            None
        } else {
            config
                .as_ref()
                .filter(|c| c.excludes(&abs_path))
                .map(|c| c.path.display().to_string())
        };
        let editorconfig_sources =
            if use_editorconfig && !cli.no_editorconfig && (cli.explain || cli.print_config) {
                ec_resolver
                    .source_paths(&cfg_path)
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                String::new()
            };
        if cli.explain {
            if let Some(source) = &exclusion {
                println!("Skipped {}: excluded by {source}", path.display());
            } else {
                println!(
                    "Included {} (config: {}; EditorConfig: {})",
                    path.display(),
                    config_source.as_deref().unwrap_or("defaults"),
                    if editorconfig_sources.is_empty() {
                        "none"
                    } else {
                        &editorconfig_sources
                    }
                );
            }
        }
        if cli.print_config {
            println!("# {}", path.display());
            println!("# config: {}", config_source.as_deref().unwrap_or("none"));
            println!(
                "# EditorConfig sources: {}",
                if editorconfig_sources.is_empty() {
                    "none"
                } else {
                    &editorconfig_sources
                }
            );
            let line_ending = options.line_ending.name();
            let commit_note = if options.commit_message {
                format!(
                    "; commit_message: true; comment_char: {}; trailer_separators: {}",
                    options.comment_char, options.trailer_separators
                )
            } else {
                String::new()
            };
            println!("# tab_width: {}; minimum indent: {}; line_ending: {line_ending}; final newline: {}; Markdeep: {}{commit_note}",
                options.tab_width, options.min_hanging_indent.map_or_else(|| "automatic".to_string(), |indent| indent.to_string()),
                options.insert_final_newline.map_or("preserve", |enabled| if enabled { "true" } else { "false" }),
                options.markdeep.map_or("auto", |enabled| if enabled { "true" } else { "false" }));
            if !cli.exclude.is_empty() {
                println!(
                    "# --exclude (relative to the current directory): {}",
                    cli.exclude.join(", ")
                );
            }
            let config_excludes = if options.commit_message {
                &[][..]
            } else {
                config.as_ref().map_or(&[][..], |c| &c.settings.exclude[..])
            };
            println!(
                "{}",
                toml::to_string_pretty(&Config::resolved_settings(&options, config_excludes))
                    .map_err(|err| err.to_string())?
            );
        }
        Ok((options, exclusion.is_some()))
    };

    let mut jobs = Vec::with_capacity(targets.len());
    for target in targets {
        match target {
            InputTarget::Stdin => {
                let mut input = String::new();
                if !cli.print_config && !cli.explain {
                    io::stdin()
                        .read_to_string(&mut input)
                        .map_err(|err| format!("Failed to read stdin: {err}"))?;
                }
                let virtual_path = cli.stdin_filepath.as_deref().map(Path::new);
                let markdeep_paths: Vec<&Path> = virtual_path.into_iter().collect();
                // Without a virtual path, use the config that applies to the current directory.
                let config_path =
                    virtual_path.map_or_else(|| cwd.join("<stdin>"), Path::to_path_buf);
                // Stdin is formatted even if its virtual path matches an exclude pattern.
                let (options, _) =
                    resolve(&config_path, &markdeep_paths, virtual_path.is_some(), false)?;
                jobs.push(Job {
                    options,
                    filter_path: virtual_path
                        .map(canonical_or_abs)
                        .unwrap_or_else(|| PathBuf::from("<stdin>")),
                    display_name: cli
                        .stdin_filepath
                        .clone()
                        .unwrap_or_else(|| "<stdin>".to_string()),
                    source: JobSource::Stdin(input),
                });
            }
            InputTarget::File {
                display_path,
                write_path,
            } => {
                let (options, excluded) =
                    resolve(display_path, &[display_path, write_path], true, true)?;
                if cli.print_config || cli.explain {
                    continue;
                }
                if excluded {
                    if cli.verbose && !cli.quiet {
                        anstream::eprintln!("Skipping excluded {}", display_path.display());
                    }
                    continue;
                }
                jobs.push(Job {
                    options,
                    filter_path: write_path.clone(),
                    display_name: display_path.display().to_string(),
                    source: JobSource::File {
                        write_path: write_path.clone(),
                    },
                });
            }
        }
    }
    Ok(jobs)
}

/// Prints the diff / dry-run / check output for one job, or writes the formatted result
/// (to stdout for stdin, otherwise back to the file).
fn report_or_write(
    cli: &Cli,
    job: &Job,
    input: &str,
    formatted: &str,
    changed: bool,
) -> Result<(), String> {
    let name = &job.display_name;
    match &job.source {
        JobSource::Stdin(_) => {
            if cli.diff {
                if changed {
                    report::print_diff(name, input, formatted)
                        .map_err(|err| format!("Failed to write stdout: {err}"))?;
                }
            } else if cli.dry_run {
                if changed {
                    println!("{name}");
                }
            } else if cli.check {
                if changed && !cli.quiet {
                    report::would_reformat(name);
                }
            } else {
                io::stdout()
                    .write_all(formatted.as_bytes())
                    .map_err(|err| format!("Failed to write stdout: {err}"))?;
            }
        }
        JobSource::File { write_path } => {
            if !changed {
                return Ok(());
            }
            if cli.diff {
                report::print_diff(name, input, formatted)
                    .map_err(|err| format!("Failed to write stdout: {err}"))?;
            }
            if cli.dry_run {
                println!("{name}");
            } else if cli.check {
                if !cli.diff && !cli.quiet {
                    report::would_reformat(name);
                }
            } else if !cli.diff {
                // `write_path` is the canonicalized target, so symlinks stay intact.
                write_atomically(write_path, formatted.as_bytes())
                    .map_err(|err| format!("Failed to write `{name}`: {err}"))?;
                if cli.verbose && !cli.quiet {
                    report::formatted(name);
                }
            }
        }
    }
    Ok(())
}

/// Replaces `path` with `contents` via a temporary file in the same directory and a rename, so
/// an interrupted run never leaves a truncated file. Keeps the original permissions. The
/// temporary file is created exclusively, so existing files and symlinks cannot be followed.
/// Rename replaces hard links with a new file, which is the usual trade-off for atomic writes.
/// When the directory isn't writable this fails rather than writing non-atomically in place.
fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let permissions = fs::metadata(path)?.permissions();
    let mut temp = tempfile::Builder::new()
        .prefix(".markfmt-")
        .tempfile_in(dir)?;
    temp.write_all(contents)?;
    temp.as_file().set_permissions(permissions)?;
    temp.persist(path).map_err(|err| err.error)?;
    Ok(())
}

/// Maps `f` over `items` on all available cores, returning results in input order.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(items.len());
    if threads <= 1 {
        return items.iter().map(f).collect();
    }

    let next = AtomicUsize::new(0);
    let mut indexed: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        out.push((i, f(item)));
                    }
                    out
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("formatter worker thread panicked"))
            .collect()
    });
    indexed.sort_unstable_by_key(|(i, _)| *i);
    indexed.into_iter().map(|(_, r)| r).collect()
}

fn canonical_or_abs(path: &Path) -> PathBuf {
    if let Ok(c) = fs::canonicalize(path) {
        return c;
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn build_range_filter(cli: &Cli, targets: &[InputTarget]) -> Result<RangeFilter, String> {
    let mut combined = RangeFilter::All;

    if !cli.lines.is_empty() {
        let target_path = match &targets[0] {
            InputTarget::File { write_path, .. } => write_path.clone(),
            InputTarget::Stdin => cli
                .stdin_filepath
                .as_deref()
                .map(|p| canonical_or_abs(Path::new(p)))
                .unwrap_or_else(|| PathBuf::from("<stdin>")),
        };
        let line_filter = RangeFilter::from_line_strings(&target_path, &cli.lines)?;
        combined = merge_range_filters(combined, line_filter);
    }

    if !cli.overlaps_git_changes.is_empty() {
        let anchor = targets
            .iter()
            .find_map(|t| match t {
                InputTarget::File { display_path, .. } => Some(display_path.clone()),
                InputTarget::Stdin => None,
            })
            .or_else(|| cli.stdin_filepath.as_deref().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));

        if targets.iter().any(|t| matches!(t, InputTarget::Stdin)) && cli.stdin_filepath.is_none() {
            return Err("--overlaps-git-changes with stdin requires --stdin-filepath".to_string());
        }

        let git_filter = RangeFilter::from_git_changes(
            cli.git_path.as_deref(),
            &anchor,
            &cli.overlaps_git_changes,
        )?;
        combined = merge_range_filters(combined, git_filter);
    }

    Ok(combined)
}

fn merge_range_filters(a: RangeFilter, b: RangeFilter) -> RangeFilter {
    match (a, b) {
        (RangeFilter::All, other) | (other, RangeFilter::All) => other,
        (RangeFilter::LineRanges(mut m1), RangeFilter::LineRanges(m2)) => {
            let mut merged: HashMap<PathBuf, Vec<RangeInclusive<usize>>> = std::mem::take(&mut m1);
            for (k, mut v) in m2 {
                merged.entry(k).or_default().append(&mut v);
            }
            RangeFilter::LineRanges(merged)
        }
    }
}

/// The Git settings that change how a commit message is read.
#[derive(Debug, Clone, Default)]
struct GitCommitConfig {
    /// `core.commentChar` / `core.commentString`: they set the same value, so whichever Git
    /// reads last wins.
    comment_char: Option<String>,
    /// `trailer.separators`.
    trailer_separators: Option<String>,
    /// The `<name>` and `key` of every `trailer.<name>.*` setting.
    trailer_keys: Vec<String>,
}

/// Resolves the checkout path whose `.markfmt.toml`, `.editorconfig`, and `git config` apply to a
/// commit-message file.
///
/// Linked worktrees keep `COMMIT_EDITMSG` under `<main>/.git/worktrees/<name>/` (with a `gitdir`
/// file pointing back to `<worktree>/.git`), submodules keep it under `<super>/.git/modules/<sub>/`
/// (with `core.worktree` pointing back to the submodule), and Jujutsu may write `*.jjdescription`
/// in the system temp directory outside any checkout.
fn commit_config_path(git_path: Option<&str>, cwd: &Path, abs_path: &Path) -> PathBuf {
    let file_name = abs_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("COMMIT_EDITMSG"));
    for ancestor in abs_path.parent().into_iter().flat_map(Path::ancestors) {
        if ancestor.join("commondir").is_file() && ancestor.join("gitdir").is_file() {
            if let Ok(raw) = fs::read_to_string(ancestor.join("gitdir")) {
                let dot_git = markfmt::config::normalized_path_in(ancestor, Path::new(raw.trim()));
                if let Some(worktree) = dot_git.parent() {
                    return worktree.join(file_name);
                }
            }
        }
        if ancestor.join("HEAD").is_file() && ancestor.join("config").is_file() {
            if ancestor
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(".git"))
            {
                return ancestor.parent().unwrap_or(ancestor).join(file_name);
            }
            if let Some(wt) = std::env::var_os("GIT_WORK_TREE").filter(|v| !v.is_empty()) {
                return markfmt::config::normalized_path_in(cwd, Path::new(&wt)).join(file_name);
            }
            if let Ok(out) = process::Command::new(git_path.unwrap_or("git"))
                .arg("--git-dir")
                .arg(ancestor)
                .args(["config", "--get", "core.worktree"])
                .output()
            {
                if out.status.success() {
                    let wt = String::from_utf8_lossy(&out.stdout);
                    let wt = wt.trim_end_matches(['\r', '\n']);
                    if !wt.is_empty() {
                        return markfmt::config::normalized_path_in(ancestor, Path::new(wt))
                            .join(file_name);
                    }
                }
            }
            return abs_path.to_path_buf();
        }
        if ancestor
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case(".jj"))
        {
            return ancestor.parent().unwrap_or(ancestor).join(file_name);
        }
    }

    let raw_tmp = markfmt::config::normalized_path(&std::env::temp_dir());
    let canon_tmp = fs::canonicalize(&raw_tmp).unwrap_or_else(|_| raw_tmp.clone());
    let canon_cwd = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let canon_dir = abs_path
        .parent()
        .and_then(|p| fs::canonicalize(p).ok())
        .unwrap_or_else(|| abs_path.parent().unwrap_or(cwd).to_path_buf());
    let in_tmp = canon_dir.starts_with(&canon_tmp) || abs_path.starts_with(&raw_tmp);
    if in_tmp && !canon_dir.starts_with(&canon_cwd) && !abs_path.starts_with(cwd) {
        let has_local_config = canon_dir
            .ancestors()
            .take_while(|a| a.starts_with(&canon_tmp) && *a != canon_tmp)
            .any(|a| a.join(markfmt::config::CONFIG_FILE_NAME).is_file());
        if !has_local_config {
            return cwd.join(file_name);
        }
    }

    abs_path.to_path_buf()
}

/// Queries Git's commit-message settings in `dir` with one `git config` call. Silently returns
/// nothing when outside a Git repository or if `git` is unavailable.
fn detect_git_commit_config(git_path: Option<&str>, dir: &Path) -> GitCommitConfig {
    let mut detected = GitCommitConfig::default();
    let Ok(output) = process::Command::new(git_path.unwrap_or("git"))
        .arg("-C")
        .arg(dir)
        .args([
            "config",
            "--null",
            "--get-regexp",
            r"^(core\.comment(char|string)|trailer\..+)$",
        ])
        .output()
    else {
        return detected;
    };
    if !output.status.success() {
        return detected;
    }
    // With `--null`, each entry is `<key>\n<value>\0` (keys lowercased, multiline values intact),
    // in the order Git reads them.
    // Each configured trailer, by name, with its `key` (the last one set wins, as in Git).
    let mut trailers: Vec<(String, Option<String>)> = Vec::new();
    for entry in String::from_utf8_lossy(&output.stdout).split('\0') {
        if entry.is_empty() {
            continue;
        }
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        if let Some(trailer) = key.strip_prefix("trailer.") {
            if trailer == "separators" {
                detected.trailer_separators = Some(value.to_string());
            } else if let Some((name, var)) = trailer.rsplit_once('.') {
                // Any `trailer.<name>.*` setting configures a trailer Git recognizes by name.
                let index = trailers
                    .iter()
                    .position(|(known, _)| known == name)
                    .unwrap_or_else(|| {
                        trailers.push((name.to_string(), None));
                        trailers.len() - 1
                    });
                if var == "key" {
                    trailers[index].1 = Some(value.to_string()).filter(|v| !v.is_empty());
                }
            }
        } else if matches!(key, "core.commentchar" | "core.commentstring") {
            detected.comment_char = markfmt::config::validate_comment_char(value)
                .is_ok()
                .then(|| value.to_string());
        }
    }
    for (name, key) in trailers {
        for value in std::iter::once(name).chain(key) {
            if !detected.trailer_keys.contains(&value) {
                detected.trailer_keys.push(value);
            }
        }
    }
    detected
}

const EDITOR_RECURSION_ENV: &str = "MARKFMT_IN_COMMIT_EDITOR";

/// Launches the user's underlying interactive editor on `cli.files` via `sh -c`, propagates any
/// non-zero exit status untouched, and returns the subset of files eligible for commit-message
/// formatting (using the `is_commit_message_path` allowlist unless `--commit-msg` was also passed).
fn run_commit_editor(cli: &Cli) -> Vec<String> {
    let Some(editor_arg) = &cli.commit_editor else {
        return cli.files.clone();
    };
    if launched_by_commit_editor(&cli.files) {
        fail("Recursive `--commit-editor` invocation detected; set `$MARKFMT_EDITOR`, `$VISUAL`, or `$EDITOR` to your underlying editor (not `markfmt`).");
    }
    for raw in &cli.files {
        if raw == "-" {
            fail("--commit-editor cannot be used with stdin (`-`).");
        }
        if Path::new(raw).is_dir() {
            fail(&format!(
                "--commit-editor requires file paths, got directory `{raw}`."
            ));
        }
    }

    let editor_cmd = resolve_editor_command(editor_arg.as_deref());
    if looks_like_self_commit_editor(&editor_cmd) {
        fail("Recursive `--commit-editor` invocation detected; set `$MARKFMT_EDITOR`, `$VISUAL`, or `$EDITOR` to your underlying editor (not `markfmt`).");
    }

    let status = spawn_editor_via_sh(&editor_cmd, cli.git_path.as_deref(), &cli.files)
        .unwrap_or_else(|err| fail(&err));
    if !status.success() {
        process::exit(status.code().unwrap_or(1));
    }

    cli.files
        .iter()
        .filter(|raw| {
            let path = Path::new(raw.as_str());
            (cli.commit_msg || discovery::is_commit_message_path(path)) && path.is_file()
        })
        .cloned()
        .collect()
}

/// Whether this process, editing `files`, runs inside an editor that another
/// `markfmt --commit-editor` launched on one of the same files.
///
/// [`EDITOR_RECURSION_ENV`] holds `<pid>:<tempfile>` (or `<pid>` if a tempfile could not be
/// created), and the tempfile lists the files that `markfmt` is editing. Editors that outlive the
/// commit (such as `code --wait` starting a GUI editor) pass the variable on to every terminal
/// they open; once the launching `markfmt` exits and deletes its tempfile, `!exists()` clears the
/// guard on every platform without walking the process tree. A commit started from inside the
/// editor (`:!git commit` in another repository, or a Git or Jujutsu command that edits its own
/// message) edits a different file, so it is not recursion either.
fn launched_by_commit_editor(files: &[String]) -> bool {
    let Some(value) = std::env::var_os(EDITOR_RECURSION_ENV) else {
        return false;
    };
    let Some(raw) = value.to_str() else {
        return true;
    };
    let pid_str = match raw.split_once(':') {
        Some((pid_part, marker_path)) if !marker_path.is_empty() => {
            let recorded = match fs::read_to_string(marker_path) {
                Ok(recorded) => recorded,
                // Gone: the launching `markfmt` has exited.
                Err(_) if !Path::new(marker_path).exists() => return false,
                // Unreadable: fall back to the process check below.
                Err(_) => String::new(),
            };
            let recorded: Vec<&str> = recorded.split('\0').filter(|p| !p.is_empty()).collect();
            let editing_same_file = files.iter().any(|file| {
                let path = canonical_or_abs(Path::new(file));
                recorded.contains(&path.to_string_lossy().as_ref())
            });
            if !recorded.is_empty() && !editing_same_file {
                return false;
            }
            pid_part
        }
        _ => raw,
    };
    let Ok(launcher) = pid_str.parse::<u32>() else {
        return true;
    };
    is_ancestor_process(launcher).unwrap_or(true)
}

/// Whether `pid` is an ancestor of this process, or `None` if that can't be determined.
#[cfg(unix)]
fn is_ancestor_process(pid: u32) -> Option<bool> {
    let mut current = std::os::unix::process::parent_id();
    for _ in 0..64 {
        if current == pid {
            return Some(true);
        }
        if current <= 1 {
            return Some(false);
        }
        current = parent_pid(current)?;
    }
    None
}

#[cfg(unix)]
fn parent_pid(pid: u32) -> Option<u32> {
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        if let Some((_, after_comm)) = stat.rsplit_once(')') {
            if let Some(ppid) = after_comm.split_whitespace().nth(1) {
                if let Ok(parsed) = ppid.parse::<u32>() {
                    return Some(parsed);
                }
            }
        }
    }
    let output = process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

#[cfg(not(unix))]
fn is_ancestor_process(_pid: u32) -> Option<bool> {
    None
}

/// Resolves the underlying editor command, deliberately ignoring `$GIT_EDITOR` and `$JJ_EDITOR`
/// since those point to `markfmt --commit-editor` itself when configured as the VCS editor.
fn resolve_editor_command(explicit: Option<&str>) -> String {
    if let Some(cmd) = explicit {
        return cmd.to_string();
    }
    for key in ["MARKFMT_EDITOR", "VISUAL", "EDITOR"] {
        if let Ok(val) = std::env::var(key) {
            if !val.trim().is_empty() {
                return val;
            }
        }
    }
    "vi".to_string()
}

fn looks_like_self_commit_editor(editor_cmd: &str) -> bool {
    if !editor_cmd.contains("--commit-editor") {
        return false;
    }
    let first_prog = first_shell_program(editor_cmd);
    Path::new(first_prog)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("markfmt"))
}

fn first_shell_program(cmd: &str) -> &str {
    let mut rest = cmd.trim_start();
    while let Some(ws) = rest.find(char::is_whitespace) {
        let tok = &rest[..ws];
        if tok.contains('=') && !tok.starts_with(['"', '\'', '/', '.']) {
            rest = rest[ws..].trim_start();
        } else {
            break;
        }
    }
    if let Some(quote) = rest.chars().next().filter(|&c| c == '"' || c == '\'') {
        let inner = &rest[1..];
        return inner.split_once(quote).map_or(inner, |(prog, _)| prog);
    }
    rest.split_whitespace().next().unwrap_or("")
}

fn spawn_editor_via_sh(
    editor_cmd: &str,
    git_path: Option<&str>,
    files: &[String],
) -> Result<process::ExitStatus, String> {
    // The marker lists the files being edited, so an editor's nested `--commit-editor` on another
    // file (a commit started from inside the editor) is not taken for recursion.
    let marker = tempfile::Builder::new()
        .prefix(".markfmt-editor-")
        .tempfile()
        .ok()
        .and_then(|mut marker| {
            let listed = files
                .iter()
                .map(|file| {
                    canonical_or_abs(Path::new(file))
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
                .join("\0");
            marker.write_all(listed.as_bytes()).ok()?;
            Some(marker)
        });
    let recursion_val = match &marker {
        Some(f) => format!("{}:{}", process::id(), f.path().display()),
        None => process::id().to_string(),
    };
    let script = format!("{editor_cmd} \"$@\"");
    let run_with = |sh_bin: &Path| {
        process::Command::new(sh_bin)
            .arg("-c")
            .arg(&script)
            .arg(editor_cmd)
            .args(files)
            .env(EDITOR_RECURSION_ENV, &recursion_val)
            .stdin(process::Stdio::inherit())
            .stdout(process::Stdio::inherit())
            .stderr(process::Stdio::inherit())
            .status()
    };

    match run_with(Path::new("sh")) {
        Ok(status) => Ok(status),
        #[cfg(windows)]
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if let Some(git_sh) = find_git_for_windows_sh(git_path) {
                return run_with(&git_sh).map_err(|e| {
                    format!(
                        "Failed to launch editor `{editor_cmd}` via `{}`: {e}",
                        git_sh.display()
                    )
                });
            }
            Err(format!(
                "Failed to launch editor `{editor_cmd}`: `sh` was not found in PATH or Git for Windows"
            ))
        }
        Err(err) => {
            let _ = git_path;
            Err(format!("Failed to launch editor `{editor_cmd}`: {err}"))
        }
    }
}

#[cfg(windows)]
fn find_git_for_windows_sh(git_path: Option<&str>) -> Option<PathBuf> {
    let git_bin = git_path.unwrap_or("git");
    let output = process::Command::new(git_bin)
        .arg("--exec-path")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let exec_path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    for ancestor in exec_path.ancestors() {
        for rel in ["usr/bin/sh.exe", "bin/sh.exe"] {
            let candidate = ancestor.join(rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
