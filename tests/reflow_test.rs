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

mod common;

use pretty_assertions::assert_eq;

use markfmt::{format_markdown, FormattingOptions, OrderedListStyle, WrapMode};

#[test]
fn greedy_used_for_short_three_line_paragraphs() {
    // Fits within 3 lines cleanly with greedy wrapping; first line should be filled as full as possible
    // rather than shrunk artificially for balance.
    let input = "This paragraph has enough words to fill two full lines and leave just a couple words on the third line for testing.\n";
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 50;
    opts.optimal = true;

    let mut greedy_opts = opts.clone();
    greedy_opts.optimal = false;
    let expected_greedy = format_markdown(input, &greedy_opts);
    let actual_default = format_markdown(input, &opts);
    assert_eq!(actual_default.lines().count(), 3);
    assert_eq!(actual_default, expected_greedy);
}

#[test]
fn optimal_knuth_plass_balances_four_plus_line_paragraphs_with_zero_last_line_slack() {
    // Carefully constructed 4+ line paragraph where greedy produces a tiny orphan or skewed lines
    // whereas optimal balancing evens out lines 1..N-1 with 0 slack penalty on line N.
    let input = "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega extra words end.\n";

    let mut optimal_opts = FormattingOptions::default();
    optimal_opts.max_line_width = 38;
    optimal_opts.optimal = true;
    let optimal_out = format_markdown(input, &optimal_opts);

    let mut greedy_opts = optimal_opts.clone();
    greedy_opts.optimal = false;
    let greedy_out = format_markdown(input, &greedy_opts);

    let opt_lines: Vec<&str> = optimal_out.lines().collect();
    assert!(opt_lines.len() >= 4);

    // Every optimal non-final line must fit width <= 38
    for line in &opt_lines {
        assert!(line.len() <= 38, "line too long: {line:?}");
    }

    // Optimal non-final squared slack sum should be <= greedy non-final squared slack sum
    let non_final_slack_sq = |lines: &[&str]| -> usize {
        lines[..lines.len() - 1]
            .iter()
            .map(|l| {
                let s = 38usize.saturating_sub(l.len());
                s * s
            })
            .sum()
    };
    assert!(
        non_final_slack_sq(&opt_lines)
            <= non_final_slack_sq(&greedy_out.lines().collect::<Vec<_>>())
    );

    // Verify last line is NOT artificially padded/split just to match width (zero last-line slack penalty)
    assert!(
        opt_lines.last().unwrap().len() <= opt_lines[opt_lines.len() - 2].len()
            || opt_lines.len() == 4
    );
    assert_eq!(format_markdown(&optimal_out, &optimal_opts), optimal_out);
}

#[test]
fn overlong_url_overflows_its_own_line_without_disabling_optimal_balancing() {
    let input = "This introductory sentence sets up a long explanation containing a very long URL https://example.com/some/very/deep/path/that/exceeds/seventy/two/characters/by/far followed by plenty more words so the surrounding paragraph wraps cleanly across five lines with even margins.\n";
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 60;
    opts.optimal = true;

    let formatted = format_markdown(input, &opts);
    let lines: Vec<&str> = formatted.lines().collect();
    assert!(lines.len() >= 5);
    assert!(lines
        .iter()
        .any(|l| l.starts_with("https://") && !l.contains(' ')));
    for l in &lines {
        if !l.starts_with("https://") {
            assert!(l.len() <= 60, "surrounding line exceeded width: {l:?}");
        }
    }
    assert_eq!(format_markdown(&formatted, &opts), formatted);
}

#[test]
fn line_start_hazards_never_start_wrapped_line_when_avoidable() {
    let with_width = |width| {
        let mut opts = FormattingOptions::default();
        opts.max_line_width = width;
        opts
    };
    let opts = with_width(24);

    // "5." must not be stranded at column 0 where it would become an ordered list item!
    let ordered_hazard = "The total item count is 5. Please check it.\n";
    let out1 = format_markdown(ordered_hazard, &opts);
    for line in out1.lines().skip(1) {
        assert!(!line.starts_with("5."), "stranded ordered marker: {out1:?}");
    }
    assert_eq!(format_markdown(&out1, &opts), out1);

    // "#" must not be stranded at column 0 where it would become a heading!
    let hash_hazard = "Use C# or symbol # for tags here.\n";
    let out2 = format_markdown(hash_hazard, &with_width(19));
    for line in out2.lines().skip(1) {
        assert!(!line.starts_with("# "), "stranded heading marker: {out2:?}");
    }

    // "-" must not be stranded at column 0
    let dash_hazard = "First phrase here - second phrase follows.\n";
    let out3 = format_markdown(dash_hazard, &with_width(19));
    for line in out3.lines().skip(1) {
        assert!(!line.starts_with("- "), "stranded bullet marker: {out3:?}");
    }

    // ">" must not be stranded at column 0
    let quote_hazard = "Ensure x matches > 10 when checking.\n";
    let out4 = format_markdown(quote_hazard, &with_width(18));
    for line in out4.lines().skip(1) {
        assert!(
            !line.starts_with("> "),
            "stranded blockquote marker: {out4:?}"
        );
    }
}

#[test]
fn hard_breaks_preserved_and_stray_trailing_spaces_stripped() {
    let input = concat!(
        "Line one has a single trailing space \n",
        "and merges cleanly here.\n",
        "\n",
        "Two-space break line one    \n",
        "and line two continues here.\n",
        "\n",
        "Explicit break line one   \\\n",
        "and line two continues here.\n",
    );
    let opts = FormattingOptions::default();
    let expected = concat!(
        "Line one has a single trailing space and merges cleanly here.\n",
        "\n",
        "Two-space break line one  \n",
        "and line two continues here.\n",
        "\n",
        "Explicit break line one\\\n",
        "and line two continues here.\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn multiple_blank_lines_collapse_to_single_and_empty_stays_empty() {
    let opts = FormattingOptions::default();
    assert_eq!(format_markdown("", &opts), "");
    // Whitespace-only content is removed, but the final newline is kept.
    assert_eq!(format_markdown("   \n\n  \n", &opts), "\n");

    let input = "First paragraph.\n\n\n\nSecond paragraph.   \n";
    assert_eq!(
        format_markdown(input, &opts),
        "First paragraph.\n\nSecond paragraph.\n"
    );
}

#[test]
fn wrap_modes() {
    let input = "First sentence here. Second one, e.g. with an\nabbreviation, runs long enough to wrap. Third?\nYes.\n";
    let with = |wrap| {
        let mut opts = FormattingOptions::default();
        opts.max_line_width = 30;
        opts.wrap = wrap;
        format_markdown(input, &opts)
    };
    assert_eq!(
        with(WrapMode::Never),
        "First sentence here. Second one, e.g. with an abbreviation, runs long enough to wrap. Third? Yes.\n"
    );
    assert_eq!(
        with(WrapMode::Preserve),
        "First sentence here. Second one, e.g. with an\nabbreviation, runs long enough to wrap. Third?\nYes.\n"
    );
    assert_eq!(
        with(WrapMode::Sentence),
        concat!(
            "First sentence here.\n",
            "Second one, e.g. with an\n",
            "abbreviation, runs long enough\n",
            "to wrap.\n",
            "Third?\n",
            "Yes.\n",
        )
    );
}

#[test]
fn ordered_list_renumbering_keeps_start_and_realigns_continuations() {
    let input = "8. eight\n8. nine has words that wrap\n8. ten has words that wrap\n";
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 20;
    opts.ordered_lists = OrderedListStyle::Ascending;
    assert_eq!(
        format_markdown(input, &opts),
        "8. eight\n9. nine has words\n   that wrap\n10. ten has words\n    that wrap\n"
    );

    let mut one = FormattingOptions::default();
    one.ordered_lists = OrderedListStyle::One;
    assert_eq!(
        format_markdown("3. a\n4. b\n5. c\n", &one),
        "3. a\n3. b\n3. c\n"
    );
}
