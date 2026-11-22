//! Blacklist of exact values: the variants of every known secret, searched with `memchr::memmem` on borrowed needles.

use std::ops::Range;

use memchr::memmem;

use super::{MARKERS, SECRET_MARKER};

/// Byte ranges of every marker already present in `text`.
pub(crate) fn marker_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = MARKERS
        .iter()
        .flat_map(|m| memmem::find_iter(text.as_bytes(), m.as_bytes()).map(move |s| s..s + m.len()))
        .collect();
    ranges.sort_by_key(|r| r.start);
    ranges
}

fn inside(range: &Range<usize>, outer: &Range<usize>) -> bool {
    outer.start <= range.start && range.end <= outer.end
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Occurrences of any needle, except those lying entirely inside a marker (a marker is never masked again).
fn hits(text: &str, needles: &[&[u8]], markers: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut hits = Vec::new();
    for needle in needles {
        for start in memmem::find_iter(text.as_bytes(), needle) {
            let hit = start..start + needle.len();
            if !markers.iter().any(|m| inside(&hit, m)) {
                hits.push(hit);
            }
        }
    }
    hits
}

pub(crate) fn contains(text: &str, needles: &[&[u8]]) -> bool {
    !needles.is_empty() && !hits(text, needles, &marker_ranges(text)).is_empty()
}

/// Replaces every occurrence of a needle with `SECRET_MARKER`. Overlapping or adjacent occurrences become one marker; an
/// occurrence that straddles a marker absorbs it. Returns the masked text and the number of markers written, or `None`
/// when nothing matched.
pub(crate) fn mask(text: &str, needles: &[&[u8]]) -> Option<(String, u32)> {
    if needles.is_empty() {
        return None;
    }
    let markers = marker_ranges(text);
    let mut ranges = hits(text, needles, &markers);
    if ranges.is_empty() {
        return None;
    }
    // A needle may be a non-UTF-8 byte string (percent-decoded cookie): widen to character boundaries.
    for r in &mut ranges {
        while !text.is_char_boundary(r.start) {
            r.start -= 1;
        }
        while !text.is_char_boundary(r.end) {
            r.end += 1;
        }
    }
    let absorbed: Vec<Range<usize>> = markers
        .into_iter()
        .filter(|m| ranges.iter().any(|r| overlaps(r, m)))
        .collect();
    ranges.extend(absorbed);
    ranges.sort_by_key(|r| r.start);

    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for r in &merged {
        out.push_str(text.get(pos..r.start).unwrap_or_default());
        out.push_str(SECRET_MARKER);
        pos = r.end;
    }
    out.push_str(text.get(pos..).unwrap_or_default());
    Some((out, u32::try_from(merged.len()).unwrap_or(u32::MAX)))
}
