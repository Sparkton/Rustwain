//! Line-based parsing of one markdown document into a [`Doc`].

use std::num::NonZeroU32;

use super::{
    inline::{self, Inline},
    model::{Doc, Heading, Link},
    slug::Anchors,
};

/// A fence may be indented by at most this many spaces, as may an ATX heading.
const MAX_INDENT: usize = 3;
const MIN_FENCE_LEN: usize = 3;
const MAX_HEADING_LEVEL: usize = 6;

/// Parses one markdown document. Fenced code blocks (``` or ~~~) are skipped entirely,
/// and inline code spans are not scanned for links.
///
/// The parser is deliberately line-based, not a full CommonMark implementation, and
/// leaves out constructs the design library does not use:
/// - Headings are ATX only (`# Title`); setext headings (an underline of `===` or `---`)
///   are not recognised, so their anchors would be missing.
/// - Headings and fences are recognised only at the start of a line (up to three spaces
///   of indentation), not inside block quotes or list items.
/// - Indented code blocks are not recognised.
/// - Only inline links and images (`[text](target)`) are collected. Reference-style links
///   are not recognised, so their targets are not checked; the `inline` module lists the
///   other limits on links.
pub fn parse_doc(text: &str) -> Doc {
    let mut doc = Doc::default();
    let mut anchors = Anchors::default();
    let mut open_fence: Option<Fence> = None;

    for (index, line) in text.lines().enumerate() {
        if let Some(fence) = &open_fence {
            if fence.is_closed_by(line) {
                open_fence = None;
            }
            continue;
        }
        if let Some(fence) = Fence::opening(line) {
            open_fence = Some(fence);
            continue;
        }

        let line_number = line_number(index);
        let content = match atx_heading(line) {
            Some(text) => {
                doc.headings.push(Heading {
                    text: text.to_owned(),
                    anchor: anchors.allocate(text),
                    line: line_number,
                });
                text
            }
            None => line,
        };
        for found in inline::scan(content) {
            match found {
                Inline::Link(target) => doc.links.push(Link {
                    target,
                    line: line_number,
                }),
                Inline::Marker(marker) => doc.markers.push((marker, line_number)),
                Inline::UnknownMarker(word) => doc.unknown_markers.push((word, line_number)),
            }
        }
    }
    doc
}

/// 1-based line number for a 0-based line index; saturates for absurdly long files.
fn line_number(index: usize) -> NonZeroU32 {
    let index = u32::try_from(index).unwrap_or(u32::MAX);
    NonZeroU32::MIN.saturating_add(index)
}

/// The opening fence of a code block: its character and length.
struct Fence {
    marker: char,
    len: usize,
}

impl Fence {
    fn opening(line: &str) -> Option<Self> {
        let body = strip_indent(line)?;
        let marker = body.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let len = body.chars().take_while(|c| *c == marker).count();
        // A backtick fence's info string cannot hold a backtick; that is an inline span.
        let info_has_backtick = marker == '`' && body[len..].contains('`');
        (len >= MIN_FENCE_LEN && !info_has_backtick).then_some(Self { marker, len })
    }

    /// A closing fence uses the same character, is at least as long, and has nothing
    /// else on its line.
    fn is_closed_by(&self, line: &str) -> bool {
        let Some(body) = strip_indent(line) else {
            return false;
        };
        let len = body.chars().take_while(|c| *c == self.marker).count();
        len >= self.len && body[len..].trim().is_empty()
    }
}

/// The line without its indentation, or `None` when indented too far to be a fence or
/// heading.
fn strip_indent(line: &str) -> Option<&str> {
    let body = line.trim_start_matches(' ');
    (line.len() - body.len() <= MAX_INDENT).then_some(body)
}

/// The text of an ATX heading line, without the leading `#`s and any closing `#`s.
fn atx_heading(line: &str) -> Option<&str> {
    let body = strip_indent(line)?;
    let level = body.chars().take_while(|c| *c == '#').count();
    if !(1..=MAX_HEADING_LEVEL).contains(&level) {
        return None;
    }
    let after_hashes = &body[level..];
    let text = if after_hashes.is_empty() {
        after_hashes
    } else {
        after_hashes.strip_prefix([' ', '\t'])?
    };
    Some(strip_closing_hashes(text.trim()))
}

/// Drops a trailing run of `#` that is separated from the text by whitespace, so `C#`
/// keeps its `#`.
fn strip_closing_hashes(text: &str) -> &str {
    let without_hashes = text.trim_end_matches('#');
    if without_hashes.is_empty() || without_hashes.ends_with([' ', '\t']) {
        without_hashes.trim_end()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::model::{LinkTarget, Marker};

    fn line(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).unwrap()
    }

    fn relative(path: &str) -> LinkTarget {
        LinkTarget::Relative {
            path: path.to_owned(),
            anchor: None,
        }
    }

    #[test]
    fn headings_get_github_anchors_with_duplicate_suffixes() {
        let doc = parse_doc(
            "# Title\n\
             ## M0 – Foundations ##\n\
             ### WireGuard `[proposed default when enabled]`\n\
             ## Notes\n\
             ## Notes\n\
             #NoSpace\n\
             ####### seven\n\
             ## C#\n\
             ## \n",
        );
        let anchors: Vec<_> = doc
            .headings
            .iter()
            .map(|h| (h.text.as_str(), h.anchor.as_str(), h.line.get()))
            .collect();
        assert_eq!(
            anchors,
            [
                ("Title", "title", 1),
                ("M0 – Foundations", "m0--foundations", 2),
                (
                    "WireGuard `[proposed default when enabled]`",
                    "wireguard-proposed-default-when-enabled",
                    3
                ),
                ("Notes", "notes", 4),
                ("Notes", "notes-1", 5),
                ("C#", "c", 8),
                ("", "", 9),
            ]
        );
        assert_eq!(doc.markers, [(Marker::Proposed, line(3))]);
    }

    #[test]
    fn fenced_code_is_skipped_by_character_length_and_indent() {
        let doc = parse_doc(
            "```rust\n\
             # not a heading [x](skipped.md) `[bogus]` [verify]\n\
             ~~~\n\
             still code\n\
             ```\n\
             [after](a.md)\n\
             ~~~~\n\
             ```\n\
             [inside](b.md)\n\
             ~~~\n\
             ~~~~\n\
             [last](c.md)\n\
             \x20 ```\n\
             [indented](d.md)\n\
             \x20 ```\n\
             [end](e.md)\n",
        );
        let linked: Vec<_> = doc
            .links
            .iter()
            .map(|l| (l.target.clone(), l.line.get()))
            .collect();
        assert_eq!(
            linked,
            [
                (relative("a.md"), 6),
                (relative("c.md"), 12),
                (relative("e.md"), 16)
            ]
        );
        assert!(doc.headings.is_empty());
        assert!(doc.markers.is_empty());
        assert!(doc.unknown_markers.is_empty());
    }

    #[test]
    fn links_in_headings_and_prose_are_recorded_with_their_lines() {
        let doc = parse_doc("# See [intro](a.md)\n\ntext `[no](b.md)` and [yes](c.md#frag)\n");
        let linked: Vec<_> = doc
            .links
            .iter()
            .map(|l| (l.target.clone(), l.line.get()))
            .collect();
        assert_eq!(
            linked,
            [
                (relative("a.md"), 1),
                (
                    LinkTarget::Relative {
                        path: "c.md".to_owned(),
                        anchor: Some("frag".to_owned())
                    },
                    3
                ),
            ]
        );
    }

    #[test]
    fn markers_in_every_form_are_counted_and_unknown_code_spans_are_kept() {
        let doc = parse_doc(
            "- `[open]` and **[verify]**\n\
             - `[later, with observability]`\n\
             - the mode `[none]` is unknown, `[u8]` is not shaped like a marker\n",
        );
        assert_eq!(
            doc.markers,
            [
                (Marker::Open, line(1)),
                (Marker::Verify, line(1)),
                (Marker::Later, line(2)),
            ]
        );
        assert_eq!(doc.unknown_markers, [("none".to_owned(), line(3))]);
    }
}
