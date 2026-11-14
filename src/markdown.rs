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

//! Parser settings shared by formatting and preprocessing. Indentation in Markdown syntax
//! always uses four-column tab stops, independently of the editor's display preferences.

use pulldown_cmark::{Event, Options, Parser};

pub(crate) const TAB_STOP: usize = 4;

pub(crate) fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
        | Options::ENABLE_MATH
}

/// Original soft-break offsets distinguish lazy paragraph continuations from reference
/// definitions, alert headers, and literal newlines inside inline code/math.
pub(crate) fn soft_breaks(text: &str) -> Vec<std::ops::Range<usize>> {
    Parser::new_ext(text, parser_options())
        .into_offset_iter()
        .filter_map(|(event, range)| matches!(event, Event::SoftBreak).then_some(range))
        .collect()
}

/// Whether the line starting at `offset` follows a soft break, given [`soft_breaks`] output
/// (sorted and disjoint). Binary search keeps per-line checks `O(log n)`.
pub(crate) fn follows_soft_break(breaks: &[std::ops::Range<usize>], offset: usize) -> bool {
    let index = breaks.partition_point(|range| range.end < offset);
    breaks.get(index).is_some_and(|range| range.start < offset)
}

/// Sorted, merged copy of `ranges` for [`strictly_inside`] queries.
pub(crate) fn merged(mut ranges: Vec<std::ops::Range<usize>>) -> Vec<std::ops::Range<usize>> {
    ranges.sort_by_key(|range| range.start);
    let mut out: Vec<std::ops::Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match out.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => out.push(range),
        }
    }
    out
}

/// Whether `offset` lies strictly inside one of the `merged` ranges.
pub(crate) fn strictly_inside(merged: &[std::ops::Range<usize>], offset: usize) -> bool {
    let index = merged.partition_point(|range| range.end <= offset);
    merged.get(index).is_some_and(|range| range.start < offset)
}
