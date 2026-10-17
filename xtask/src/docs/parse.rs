//! Block-level parsing of one markdown document into a [`Doc`]: it separates headings,
//! fenced code, HTML comments and paragraphs, and hands each paragraph to the inline
//! scanner as one buffer, so links and code spans may wrap across lines.

use std::num::NonZeroU32;

use super::{
    inline::{self, Found, HTML_COMMENT_CLOSE, HTML_COMMENT_OPEN, Inline},
    model::{Doc, Heading, Link},
    slug::Anchors,
};

/// An ATX heading may be indented by at most this many spaces.
const MAX_HEADING_INDENT: usize = 3;
const MIN_FENCE_LEN: usize = 3;
const MAX_HEADING_LEVEL: usize = 6;
const BYTE_ORDER_MARK: char = '\u{feff}';
const QUOTE_MARKER: char = '>';

/// Parses one markdown document. Fenced code blocks (``` or ~~~) and HTML comments are
/// skipped entirely, and inline code spans are not scanned for links.
///
/// The parser is deliberately line-based, not a full CommonMark implementation, and
/// leaves out constructs the design library does not use:
/// - Headings are ATX only (`# Title`); setext headings (an underline of `===` or `---`)
///   are not recognised, so their anchors would be missing.
/// - A heading is recognised at the start of a line (up to three spaces of indentation)
///   after any `>` quote markers, but not inside a list item.
/// - A fence is recognised at any indentation and after any `>` quote markers, so
///   fences nested in list items work. Indented code blocks are not recognised.
/// - A paragraph is the run of non-blank lines up to the next heading, fence, comment,
///   list item or table row. Links and code spans may wrap across its lines.
/// - Only inline links and images (`[text](target)`) are collected. Reference-style links
///   are not recognised, so their targets are not checked; the `inline` module lists the
///   other limits on links.
///
/// A leading byte order mark is ignored.
pub fn parse_doc(text: &str) -> Doc {
    let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(text);
    let mut parser = Parser::default();
    for (index, line) in text.lines().enumerate() {
        parser.accept(line_number(index), line);
    }
    parser.finish()
}

/// 1-based line number for a 0-based line index; saturates for absurdly long files.
fn line_number(index: usize) -> NonZeroU32 {
    let index = u32::try_from(index).unwrap_or(u32::MAX);
    NonZeroU32::MIN.saturating_add(index)
}

#[derive(Default)]
struct Parser {
    doc: Doc,
    anchors: Anchors,
    paragraph: Paragraph,
    block: Block,
}

/// What the parser is inside of. Only the text outside fences and comments is scanned.
#[derive(Default)]
enum Block {
    #[default]
    Text,
    Fenced(Fence),
    Comment,
}

impl Parser {
    fn accept(&mut self, number: NonZeroU32, raw: &str) {
        let line = strip_quote(raw);
        match &self.block {
            Block::Fenced(fence) => {
                if fence.is_closed_by(line) {
                    self.block = Block::Text;
                }
            }
            Block::Comment => {
                if line.contains(HTML_COMMENT_CLOSE) {
                    self.block = Block::Text;
                }
            }
            Block::Text => self.text_line(number, line),
        }
    }

    fn text_line(&mut self, number: NonZeroU32, line: &str) {
        if line.trim().is_empty() {
            self.flush();
        } else if let Some(fence) = Fence::opening(line) {
            self.flush();
            self.block = Block::Fenced(fence);
        } else if let Some(text) = atx_heading(line) {
            self.flush();
            self.heading(number, text);
        } else if let Some(after_open) = line.trim_start().strip_prefix(HTML_COMMENT_OPEN) {
            self.flush();
            if !after_open.contains(HTML_COMMENT_CLOSE) {
                self.block = Block::Comment;
            }
        } else if is_table_row(line) {
            // A table cell never wraps, so a row is a paragraph of its own.
            self.flush();
            self.paragraph.push(number, line);
            self.flush();
        } else {
            if is_list_item(line) {
                self.flush();
            }
            self.paragraph.push(number, line);
        }
    }

    fn heading(&mut self, number: NonZeroU32, text: &str) {
        self.doc.headings.push(Heading {
            text: text.to_owned(),
            anchor: self.anchors.allocate(text),
            line: number,
        });
        self.record(inline::scan(text), |_| number);
    }

    /// Scans the buffered paragraph, if any.
    fn flush(&mut self) {
        let paragraph = std::mem::take(&mut self.paragraph);
        if !paragraph.text.is_empty() {
            self.record(inline::scan(&paragraph.text), |offset| {
                paragraph.line_of(offset)
            });
        }
    }

    fn record(&mut self, found: Vec<Found>, line_of: impl Fn(usize) -> NonZeroU32) {
        for Found { offset, item } in found {
            let line = line_of(offset);
            match item {
                Inline::Link(target) => self.doc.links.push(Link { target, line }),
                Inline::Marker(marker) => self.doc.markers.push((marker, line)),
                Inline::UnknownMarker(word) => self.doc.unknown_markers.push((word, line)),
            }
        }
    }

    fn finish(mut self) -> Doc {
        self.flush();
        self.doc
    }
}

/// The lines of one paragraph joined by `\n`, and where each line starts, so a byte
/// offset in the joined text maps back to its source line.
#[derive(Default)]
struct Paragraph {
    text: String,
    line_starts: Vec<(usize, NonZeroU32)>,
}

impl Paragraph {
    fn push(&mut self, number: NonZeroU32, line: &str) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.line_starts.push((self.text.len(), number));
        self.text.push_str(line);
    }

    fn line_of(&self, offset: usize) -> NonZeroU32 {
        let started = self
            .line_starts
            .partition_point(|&(start, _)| start <= offset);
        started
            .checked_sub(1)
            .and_then(|last| self.line_starts.get(last))
            .map_or(NonZeroU32::MIN, |&(_, number)| number)
    }
}

/// The opening fence of a code block: its character and length.
struct Fence {
    marker: char,
    len: usize,
}

impl Fence {
    fn opening(line: &str) -> Option<Self> {
        let body = line.trim_start();
        let marker = body.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let len = body.chars().take_while(|c| *c == marker).count();
        // A backtick fence's info string cannot hold a backtick; that is an inline span.
        let info_has_backtick = marker == '`' && body[len..].contains('`');
        (len >= MIN_FENCE_LEN && !info_has_backtick).then_some(Self { marker, len })
    }

    /// A closing fence uses the same character, is at least as long, and has nothing
    /// else on its line.
    fn is_closed_by(&self, line: &str) -> bool {
        let body = line.trim_start();
        let len = body.chars().take_while(|c| *c == self.marker).count();
        len >= self.len && body[len..].trim().is_empty()
    }
}

/// The line without any leading `>` block quote markers (each with its optional space).
fn strip_quote(line: &str) -> &str {
    let mut rest = line;
    while let Some(after_marker) =
        strip_indent(rest).and_then(|body| body.strip_prefix(QUOTE_MARKER))
    {
        rest = after_marker.strip_prefix(' ').unwrap_or(after_marker);
    }
    rest
}

/// The line without its indentation, or `None` when indented too far to be a heading or
/// a quote marker.
fn strip_indent(line: &str) -> Option<&str> {
    let body = line.trim_start_matches(' ');
    (line.len() - body.len() <= MAX_HEADING_INDENT).then_some(body)
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

fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// A bullet (`-`, `*` or `+`) or an ordered marker (`1.` or `1)`), followed by a space.
fn is_list_item(line: &str) -> bool {
    let body = line.trim_start();
    let after_marker = match body.strip_prefix(['-', '*', '+']) {
        Some(after_bullet) => after_bullet,
        None => {
            let after_digits = body.trim_start_matches(|c: char| c.is_ascii_digit());
            if after_digits.len() == body.len() {
                return false;
            }
            match after_digits.strip_prefix(['.', ')']) {
                Some(after_dot) => after_dot,
                None => return false,
            }
        }
    };
    after_marker.starts_with(' ')
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

    /// `(target, line)` of every link, to compare whole documents at once.
    fn links(doc: &Doc) -> Vec<(LinkTarget, u32)> {
        doc.links
            .iter()
            .map(|l| (l.target.clone(), l.line.get()))
            .collect()
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
    fn heading_anchors_ignore_links_and_emphasis_but_keep_snake_case() {
        let doc = parse_doc(
            "## See [intro](a.md) now\n\
             ## _Why_ **now**\n\
             ## the snake_case rule\n",
        );
        let anchors: Vec<_> = doc.headings.iter().map(|h| h.anchor.as_str()).collect();
        assert_eq!(anchors, ["see-intro-now", "why-now", "the-snake_case-rule"]);
        assert_eq!(links(&doc), [(relative("a.md"), 1)]);
    }

    #[test]
    fn fenced_code_is_skipped_by_character_length_and_indent() {
        let doc = parse_doc(
            "```rust\n\
             # not a heading [x](skipped.md) `[verfiy]` [verify]\n\
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
        assert_eq!(
            links(&doc),
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
    fn fences_nested_in_list_items_are_skipped() {
        let doc = parse_doc(
            "1. Step one:\n\
             \n\
             \x20   ```yaml\n\
             \x20   [x](skipped.md): `[verfiy]`\n\
             \x20   # not a heading\n\
             \x20   ```\n\
             \n\
             - [after](a.md)\n",
        );
        assert_eq!(links(&doc), [(relative("a.md"), 8)]);
        assert!(doc.headings.is_empty());
        assert!(doc.unknown_markers.is_empty());
    }

    #[test]
    fn fences_and_headings_in_block_quotes_are_recognised() {
        let doc = parse_doc(
            "> ## Quoted heading\n\
             > ```\n\
             > [x](skipped.md)\n\
             > ```\n\
             > > [nested](a.md)\n\
             >[tight](b.md)\n",
        );
        assert_eq!(
            doc.headings
                .iter()
                .map(|h| (h.text.as_str(), h.line.get()))
                .collect::<Vec<_>>(),
            [("Quoted heading", 1)]
        );
        assert_eq!(links(&doc), [(relative("a.md"), 5), (relative("b.md"), 6)]);
    }

    #[test]
    fn links_in_headings_and_prose_are_recorded_with_their_lines() {
        let doc = parse_doc("# See [intro](a.md)\n\ntext `[no](b.md)` and [yes](c.md#frag)\n");
        assert_eq!(
            links(&doc),
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
    fn wrapped_links_and_code_spans_are_found_at_the_line_of_their_bracket() {
        let doc = parse_doc(
            "Intro text that\n\
             see [the long\n\
             link text](gone.md) and `a span that\n\
             wraps [x](hidden.md)` then\n\
             [last](z.md)\n\
             \n\
             - item with [one\n\
             wrapped](item.md)\n\
             - next [two](two.md)\n\
             | table [a](t.md) | `stray |\n\
             | table [b](u.md) | other ` |\n",
        );
        assert_eq!(
            links(&doc),
            [
                (relative("gone.md"), 2),
                (relative("z.md"), 5),
                (relative("item.md"), 7),
                (relative("two.md"), 9),
                (relative("t.md"), 10),
                (relative("u.md"), 11),
            ]
        );
    }

    #[test]
    fn html_comments_are_skipped_across_lines_and_inline() {
        let doc = parse_doc(
            "<!-- [one](skipped.md) -->\n\
             [a](a.md) <!-- [two](skipped.md) --> [b](b.md)\n\
             <!--\n\
             # Hidden heading\n\
             ```\n\
             [three](skipped.md) `[verfiy]`\n\
             -->\n\
             ## Shown\n\
             text <!-- spans\n\
             lines [four](skipped.md) --> [c](c.md)\n",
        );
        assert_eq!(
            links(&doc),
            [
                (relative("a.md"), 2),
                (relative("b.md"), 2),
                (relative("c.md"), 10)
            ]
        );
        let headings: Vec<_> = doc.headings.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(headings, ["Shown"]);
        assert!(doc.unknown_markers.is_empty());
    }

    #[test]
    fn a_leading_byte_order_mark_does_not_hide_the_first_heading() {
        let doc = parse_doc("\u{feff}# Title\n[a](a.md)\n");

        assert_eq!(doc.headings.len(), 1);
        assert_eq!(doc.headings[0].anchor, "title");
        assert_eq!(links(&doc), [(relative("a.md"), 2)]);
    }

    #[test]
    fn markers_in_every_form_are_counted_and_typos_are_kept() {
        let doc = parse_doc(
            "- `[open]` and **[verify]**\n\
             - `[later, with observability]`\n\
             - the typo `[verfiy]` is reported, `[bans]` and `[u8]` are not\n",
        );
        assert_eq!(
            doc.markers,
            [
                (Marker::Open, line(1)),
                (Marker::Verify, line(1)),
                (Marker::Later, line(2)),
            ]
        );
        assert_eq!(doc.unknown_markers, [("verfiy".to_owned(), line(3))]);
    }
}
