//! Inline scanning of one line of markdown: links, images and decision markers.
//!
//! Deliberate limitations, each because the design library does not use the construct:
//! - Reference-style links (`[text][ref]`, `[ref]: url`) are not recognised, so their
//!   targets are not checked.
//! - Link destinations in angle brackets, backslash escapes inside destinations,
//!   percent-encoding, and titles containing `"` are not handled. A destination ends at
//!   the first whitespace or unbalanced `)`.
//! - Code spans are matched within one line only. A span that wraps onto the next line
//!   is treated as plain text.
//! - Raw HTML (`<a href>`, `<a name>`) is ignored.

use super::model::{LinkTarget, Marker};

const BACKTICK: char = '`';
const ESCAPE: char = '\\';

/// What [`scan`] finds on a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Inline {
    Link(LinkTarget),
    Marker(Marker),
    /// The word of a marker-shaped code span whose word is not a known marker.
    UnknownMarker(String),
}

/// Scans a line (or heading text) outside any fenced code block.
///
/// A marker is a known `[word]` in plain text that is not followed by `(` or `[`, or an
/// inline code span shaped like `[word]`, `[word rest]` or `[word, rest]`.
pub(super) fn scan(text: &str) -> Vec<Inline> {
    let mut found = Vec::new();
    scan_into(text, &mut found);
    found
}

fn scan_into(text: &str, found: &mut Vec<Inline>) {
    let mut rest = text;
    while let Some(offset) = rest.find([ESCAPE, BACKTICK, '[']) {
        let at = &rest[offset..];
        rest = match at.chars().next() {
            Some(ESCAPE) => skip_escape(at),
            Some(BACKTICK) => match backticks(at) {
                Backticks::Span { content, rest } => {
                    found.extend(code_marker(content));
                    rest
                }
                Backticks::Unmatched { rest } => rest,
            },
            _ => bracket(at, found),
        };
    }
}

/// `at` starts with `[`. Records a link or a marker and returns the text after it; for
/// plain brackets returns the text just after the `[`, so the contents are scanned too.
fn bracket<'a>(at: &'a str, found: &mut Vec<Inline>) -> &'a str {
    let inside_and_after = &at[1..];
    let Some(close) = closing_bracket(inside_and_after) else {
        return inside_and_after;
    };
    let inside = &inside_and_after[..close];
    let after = &inside_and_after[close + 1..];

    if let Some((destination, rest)) = link_destination(after) {
        found.push(Inline::Link(classify(destination)));
        scan_into(inside, found);
        return rest;
    }
    let followed_by_link_syntax = after.starts_with(['(', '[']);
    if let Some(marker) = Marker::from_word(inside).filter(|_| !followed_by_link_syntax) {
        found.push(Inline::Marker(marker));
        return after;
    }
    inside_and_after
}

/// The byte offset in `text` (which starts just after a `[`) of the matching `]`,
/// skipping nested brackets, escapes and code spans.
fn closing_bracket(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut rest = text;
    loop {
        let at = &rest[rest.find([ESCAPE, BACKTICK, '[', ']'])?..];
        rest = match at.chars().next() {
            Some(ESCAPE) => skip_escape(at),
            Some(BACKTICK) => match backticks(at) {
                Backticks::Span { rest, .. } | Backticks::Unmatched { rest } => rest,
            },
            Some('[') => {
                depth += 1;
                &at[1..]
            }
            Some(']') if depth == 0 => return Some(text.len() - at.len()),
            // A `]` that closes a nested `[`.
            _ => {
                depth = depth.saturating_sub(1);
                &at[1..]
            }
        };
    }
}

/// `at` starts with `\`: drops it and the character it escapes.
fn skip_escape(at: &str) -> &str {
    let mut chars = at.chars();
    chars.next();
    chars.next();
    chars.as_str()
}

enum Backticks<'a> {
    /// A run closed by a run of the same length.
    Span { content: &'a str, rest: &'a str },
    /// A run with no matching closer; the backticks are literal text.
    Unmatched { rest: &'a str },
}

/// `at` starts with a backtick run.
fn backticks(at: &str) -> Backticks<'_> {
    let body = at.trim_start_matches(BACKTICK);
    let run_len = at.len() - body.len();
    let mut searched = 0;
    while let Some(found) = body[searched..].find(BACKTICK) {
        let start = searched + found;
        let closer_len = body[start..].len() - body[start..].trim_start_matches(BACKTICK).len();
        if closer_len == run_len {
            return Backticks::Span {
                content: &body[..start],
                rest: &body[start + closer_len..],
            };
        }
        searched = start + closer_len;
    }
    Backticks::Unmatched { rest: body }
}

/// Parses `(destination "optional title")` at the start of `after`, returning the
/// destination and the text after the `)`.
fn link_destination(after: &str) -> Option<(&str, &str)> {
    let body = after.strip_prefix('(')?.trim_start();
    let (destination, tail) = body.split_at(destination_len(body));
    let tail = skip_title(tail.trim_start()).trim_start();
    Some((destination, tail.strip_prefix(')')?))
}

/// The byte length of the destination: up to the first whitespace or unbalanced `)`.
fn destination_len(body: &str) -> usize {
    let mut depth = 0usize;
    for (offset, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return offset,
            ')' => depth -= 1,
            c if c.is_whitespace() => return offset,
            _ => {}
        }
    }
    body.len()
}

/// Drops a leading `"title"`; anything else (including an unterminated title) is left in
/// place, which makes the caller's `)` check fail.
fn skip_title(tail: &str) -> &str {
    tail.strip_prefix('"')
        .and_then(|after_quote| after_quote.split_once('"'))
        .map_or(tail, |(_, after_title)| after_title)
}

fn classify(destination: &str) -> LinkTarget {
    if has_scheme(destination) {
        return LinkTarget::External(destination.to_owned());
    }
    if let Some(anchor) = destination.strip_prefix('#') {
        return LinkTarget::SameDoc {
            anchor: anchor.to_owned(),
        };
    }
    let (path, anchor) = match destination.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor)),
        None => (destination, None),
    };
    LinkTarget::Relative {
        path: path.to_owned(),
        anchor: anchor.filter(|a| !a.is_empty()).map(str::to_owned),
    }
}

/// Whether the destination starts with `[A-Za-z][A-Za-z0-9+.-]*:`.
fn has_scheme(destination: &str) -> bool {
    let Some((scheme, _)) = destination.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

/// The marker (known or not) that an inline code span spells, if it is marker-shaped:
/// `^\[[a-z]+([ ,][^\]]*)?\]$`.
fn code_marker(code: &str) -> Option<Inline> {
    let inside = code.strip_prefix('[')?.strip_suffix(']')?;
    let tail = inside.trim_start_matches(|c: char| c.is_ascii_lowercase());
    let word = &inside[..inside.len() - tail.len()];
    let tail_is_prose = tail.starts_with([' ', ',']) && !tail.contains(']');
    if word.is_empty() || !(tail.is_empty() || tail_is_prose) {
        return None;
    }
    Some(match Marker::from_word(word) {
        Some(marker) => Inline::Marker(marker),
        None => Inline::UnknownMarker(word.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative(path: &str, anchor: Option<&str>) -> Inline {
        Inline::Link(LinkTarget::Relative {
            path: path.to_owned(),
            anchor: anchor.map(str::to_owned),
        })
    }

    #[test]
    fn links_and_images_are_classified_by_target() {
        let found = scan(
            "[a](b.md#c) ![img](../x.png \"t\") [d](#e) [f](https://x.y/z) [g](mailto:a@b.c) [h](dir/)",
        );
        assert_eq!(
            found,
            [
                relative("b.md", Some("c")),
                relative("../x.png", None),
                Inline::Link(LinkTarget::SameDoc {
                    anchor: "e".to_owned()
                }),
                Inline::Link(LinkTarget::External("https://x.y/z".to_owned())),
                Inline::Link(LinkTarget::External("mailto:a@b.c".to_owned())),
                relative("dir/", None),
            ]
        );
    }

    #[test]
    fn links_inside_code_spans_and_escapes_are_ignored() {
        let found = scan("`[a](b.md)` and ``[c](d.md) ` e`` and \\[f](g.md) and [h](i.md)");
        assert_eq!(found, [relative("i.md", None)]);
    }

    #[test]
    fn destinations_may_hold_balanced_parentheses_and_surrounding_spaces() {
        let found = scan("[a](x_(1).md) [b]( y.md \"title\" ) [c](broken.md");
        assert_eq!(found, [relative("x_(1).md", None), relative("y.md", None)]);
    }

    #[test]
    fn nested_brackets_and_code_in_link_text_keep_the_link() {
        let found = scan("[see `a[0]` and [y]](z.md)");
        assert_eq!(found, [relative("z.md", None)]);
    }

    #[test]
    fn known_markers_count_in_code_and_plain_text() {
        let found = scan(
            "`[open]` `[later, with observability]` `[proposed default when enabled]` **[verify]** [skip]",
        );
        assert_eq!(
            found,
            [
                Inline::Marker(Marker::Open),
                Inline::Marker(Marker::Later),
                Inline::Marker(Marker::Proposed),
                Inline::Marker(Marker::Verify),
                Inline::Marker(Marker::Skip),
            ]
        );
    }

    #[test]
    fn plain_marker_followed_by_link_syntax_is_not_a_marker() {
        let found = scan("[verify](a.md) [open][ref] [decided]");
        assert_eq!(
            found,
            [relative("a.md", None), Inline::Marker(Marker::Decided)]
        );
    }

    #[test]
    fn marker_shaped_code_spans_with_unknown_words_are_reported() {
        let found =
            scan("`[none]` `[integrity, x]` `[none] x` `[u8]` `[T]` `[a-z]+` `[none, but]]` `[]`");
        assert_eq!(
            found,
            [
                Inline::UnknownMarker("none".to_owned()),
                Inline::UnknownMarker("integrity".to_owned()),
            ]
        );
    }

    #[test]
    fn bracketed_unknown_words_in_prose_are_ignored() {
        assert_eq!(scan("the [none] word and [x] arrays"), []);
    }
}
