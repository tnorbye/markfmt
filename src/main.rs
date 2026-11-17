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

    let discovered = discovery::discover_targets_with_options(
        &cli.files,
        discovery::DiscoveryOptions {
            no_ignore: cli.no_ignore,
            filter_files: cli.filter_files,
            explain: cli.explain,
        },
    )
    .unwrap_or_else(|err| fail(&err));
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
        build_range_filter(&cli, &targets).unwrap_or_else(|err| fail(&err))
    };
    let only_stdin = !targets.is_empty() && targets.iter().all(|t| matches!(t, InputTarget::Stdin));
    let jobs = build_jobs(&cli, &targets).unwrap_or_else(|err| fail(&err));
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
        let warnings = lint::lint(&input);
        Ok::<_, String>((input, formatted, warnings))
    });

    let mut counts = report::Counts::default();
    for (job, result) in jobs.iter().zip(results) {
        let (input, formatted, warnings) = match result {
            Ok(outcome) => outcome,
            Err(err) => {
                report::error(&err);
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
                report::error(&err);
                counts.failed += 1;
            }
        }
    }

    // Formatting stdin (as editors do) prints only the result, never a summary.
    if !only_stdin && !cli.quiet {
        if cli.check {
            counts.print_check_summary();
        } else if !cli.diff && !cli.dry_run {
            counts.print_formatted_summary();
        }
    }

    if counts.failed > 0 {
        process::exit(2);
    }
    if cli.check && counts.changed > 0 {
        process::exit(1);
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
    let overrides = cli.cli_overrides();

    // `.editorconfig` only applies to stdin when `--stdin-filepath` gives it a location.
    let mut resolve = |path: &Path,
                       markdeep_paths: &[&Path],
                       use_editorconfig: bool,
                       apply_excludes: bool|
     -> Result<(FormattingOptions, bool), String> {
        let abs_path = absolute(path);
        let config = match configs.as_mut() {
            Some(resolver) => resolver.for_file(&abs_path)?,
            None => None,
        };
        let mut options = if use_editorconfig && !cli.no_editorconfig {
            ec_resolver.resolve(&abs_path)
        } else {
            FormattingOptions::default()
        };
        if let Some(config) = &config {
            config.apply(&mut options);
        }
        overrides.apply(&mut options);
        if markdeep_paths
            .iter()
            .any(|p| discovery::is_markdeep_path(p))
        {
            options.markdeep = Some(true);
        }
        let config_source = config.as_ref().map(|c| c.path.display().to_string());
        let exclusion = if !apply_excludes {
            None
        } else if cli_excludes.matches(&abs_path) {
            Some("--exclude".to_string())
        } else {
            config
                .as_ref()
                .filter(|c| c.excludes(&abs_path))
                .map(|c| c.path.display().to_string())
        };
        let editorconfig_sources =
            if use_editorconfig && !cli.no_editorconfig && (cli.explain || cli.print_config) {
                ec_resolver
                    .source_paths(&abs_path)
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
            println!("# tab_width: {}; minimum indent: {}; line_ending: {line_ending}; final newline: {}; Markdeep: {}",
                options.tab_width, options.min_hanging_indent.map_or_else(|| "automatic".to_string(), |indent| indent.to_string()),
                options.insert_final_newline.map_or("preserve", |enabled| if enabled { "true" } else { "false" }),
                options.markdeep.map_or("auto", |enabled| if enabled { "true" } else { "false" }));
            if !cli.exclude.is_empty() {
                println!(
                    "# --exclude (relative to the current directory): {}",
                    cli.exclude.join(", ")
                );
            }
            let config_excludes = config.as_ref().map_or(&[][..], |c| &c.settings.exclude[..]);
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
