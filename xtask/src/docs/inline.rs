//! Inline scanning of markdown text, which is one paragraph (lines joined by `\n`) or one
//! heading: links, images and decision markers, plus the plain text of a heading.
//!
//! Deliberate limitations, each because the design library does not use the construct:
//! - Reference-style links (`[text][ref]`, `[ref]: url`) are not recognised, so their
//!   targets are not checked.
//! - Backslash escapes inside destinations, percent-encoding, and titles that contain
//!   their own closing delimiter are not handled. A bare destination ends at the first
//!   whitespace or unbalanced `)`; use `<...>` for a destination with spaces.
//! - Raw HTML is ignored, except that `<!-- ... -->` comments are skipped. An unclosed
//!   `<!--` is plain text.
//! - A `[` or backtick run with no partner is literal text.
//!
//! A marker-shaped code span (`` `[word]` ``) is reported as an unknown marker only when
//! its word is a near miss of a known marker: invented words are left to review because
//! `[bans]`, `[error]` and `[min, max]` are legitimate code in a design library. See
//! `code_marker`.

use super::model::{LinkTarget, Marker};

pub(super) const HTML_COMMENT_OPEN: &str = "<!--";
pub(super) const HTML_COMMENT_CLOSE: &str = "-->";

const BACKTICK: char = '`';
const ESCAPE: char = '\\';
const EMPHASIS_UNDERSCORE: char = '_';
const EMPHASIS_STAR: char = '*';

/// Words up to this long may differ from a marker by one edit, longer words by two.
const SHORT_WORD_LEN: usize = 5;
const SHORT_WORD_EDITS: usize = 1;
const LONG_WORD_EDITS: usize = 2;

/// What [`scan`] finds, and where: `offset` is the byte offset in the scanned text of the
/// `[` of a link or bracketed marker, or of the opening backtick of a code span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Found {
    pub offset: usize,
    pub item: Inline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Inline {
    Link(LinkTarget),
    Marker(Marker),
    /// The word of a marker-shaped code span that is a near miss of a known marker.
    UnknownMarker(String),
}

/// Scans one paragraph or heading outside any fenced code block.
///
/// A marker is a known `[word]` in plain text that is not followed by `(` or `[`, or an
/// inline code span shaped like `[word]`, `[word rest]` or `[word, rest]`.
pub(super) fn scan(text: &str) -> Vec<Found> {
    let mut found = Vec::new();
    scan_into(text, 0, &mut found);
    found
}

/// The text a heading renders as, for its anchor: links reduced to their text, code span
/// content kept verbatim without its backticks, HTML comments dropped, and emphasis
/// delimiters dropped. `*` is always a delimiter; `_` only when it is not inside a word,
/// so `snake_case` keeps its underscore. A `_` is not told apart as unmatched.
pub(super) fn plain_text(markup: &str) -> String {
    let mut plain = String::with_capacity(markup.len());
    push_plain(markup, &mut plain);
    plain
}

fn scan_into(text: &str, base: usize, found: &mut Vec<Found>) {
    let mut rest = text;
    while let Some(offset) = rest.find([ESCAPE, BACKTICK, '[', '<']) {
        let at = &rest[offset..];
        let at_offset = base + (text.len() - at.len());
        rest = match at.chars().next() {
            Some(ESCAPE) => skip_escape(at),
            Some(BACKTICK) => match backticks(at) {
                Backticks::Span { content, rest } => {
                    found.extend(code_marker(content).map(|item| Found {
                        offset: at_offset,
                        item,
                    }));
                    rest
                }
                Backticks::Unmatched { rest } => rest,
            },
            Some('[') => bracket(at, at_offset, found),
            _ => skip_comment(at),
        };
    }
}

/// `at` starts with `[` at byte `at_offset`. Records a link or a marker and returns the
/// text after it; for plain brackets returns the text just after the `[`, so the
/// contents are scanned too.
fn bracket<'a>(at: &'a str, at_offset: usize, found: &mut Vec<Found>) -> &'a str {
    let Some(Bracketed { inside, after }) = bracketed(at) else {
        return &at[1..];
    };
    if let Some((destination, rest)) = link_destination(after) {
        found.push(Found {
            offset: at_offset,
            item: Inline::Link(classify(destination)),
        });
        scan_into(inside, at_offset + 1, found);
        return rest;
    }
    let followed_by_link_syntax = after.starts_with(['(', '[']);
    if let Some(marker) = Marker::from_word(inside).filter(|_| !followed_by_link_syntax) {
        found.push(Found {
            offset: at_offset,
            item: Inline::Marker(marker),
        });
        return after;
    }
    &at[1..]
}

/// `[inside]` and the text `after` it.
struct Bracketed<'a> {
    inside: &'a str,
    after: &'a str,
}

/// `at` starts with `[`.
fn bracketed(at: &str) -> Option<Bracketed<'_>> {
    let after_open = &at[1..];
    let close = closing_bracket(after_open)?;
    Some(Bracketed {
        inside: &after_open[..close],
        after: &after_open[close + 1..],
    })
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

/// `at` starts with `<`: drops a whole `<!-- ... -->` comment, else just the `<`.
fn skip_comment(at: &str) -> &str {
    after_comment(at).unwrap_or(&at[1..])
}

/// The text after the `-->` of a closed comment at the start of `at`.
fn after_comment(at: &str) -> Option<&str> {
    let (_, after) = at
        .strip_prefix(HTML_COMMENT_OPEN)?
        .split_once(HTML_COMMENT_CLOSE)?;
    Some(after)
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
/// destination and the text after the `)`. The destination is bare or in `<...>`, and
/// the title is quoted with `"` or `'` or parenthesised.
fn link_destination(after: &str) -> Option<(&str, &str)> {
    let body = after.strip_prefix('(')?.trim_start();
    let (destination, tail) = match body.strip_prefix('<') {
        Some(angled) => angled_destination(angled)?,
        None => body.split_at(destination_len(body)),
    };
    let tail = skip_title(tail.trim_start()).trim_start();
    Some((destination, tail.strip_prefix(')')?))
}

/// `after_open` follows a `<`: the destination up to the closing `>`, which must come
/// before another `<` or a line break, and the text after that `>`.
fn angled_destination(after_open: &str) -> Option<(&str, &str)> {
    let (destination, from_close) = after_open.split_at(after_open.find(['>', '<', '\n'])?);
    Some((destination, from_close.strip_prefix('>')?))
}

/// The byte length of a bare destination: up to the first whitespace or unbalanced `)`.
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

/// Drops a leading `"title"`, `'title'` or `(title)`; anything else (including an
/// unterminated title) is left in place, which makes the caller's `)` check fail.
fn skip_title(tail: &str) -> &str {
    let closing = match tail.chars().next() {
        Some('"') => '"',
        Some('\'') => '\'',
        Some('(') => ')',
        _ => return tail,
    };
    tail[1..]
        .split_once(closing)
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

/// The marker that an inline code span spells, if it is marker-shaped
/// (`^\[[a-z]+([ ,][^\]]*)?\]$`) and its word is a known marker or a near miss of one.
///
/// Only near misses are reported as unknown, because marker shapes such as `[bans]`,
/// `[error]` or `[min, max]` are ordinary code in a design library. The rule targets
/// typos like `[verfiy]` or `[propsed]`; a deliberately invented marker is left to review.
fn code_marker(code: &str) -> Option<Inline> {
    let inside = code.strip_prefix('[')?.strip_suffix(']')?;
    let tail = inside.trim_start_matches(|c: char| c.is_ascii_lowercase());
    let word = &inside[..inside.len() - tail.len()];
    let tail_is_prose = tail.starts_with([' ', ',']) && !tail.contains(']');
    if word.is_empty() || !(tail.is_empty() || tail_is_prose) {
        return None;
    }
    match Marker::from_word(word) {
        Some(marker) => Some(Inline::Marker(marker)),
        None => is_near_miss(word).then(|| Inline::UnknownMarker(word.to_owned())),
    }
}

/// Whether an unknown `word` is within a couple of edits of a known marker: one edit for
/// words of up to five letters, two for longer ones.
fn is_near_miss(word: &str) -> bool {
    let allowed = if word.len() <= SHORT_WORD_LEN {
        SHORT_WORD_EDITS
    } else {
        LONG_WORD_EDITS
    };
    Marker::ALL
        .iter()
        .any(|marker| edit_distance(word, marker.word()) <= allowed)
}

/// Optimal string alignment distance: insertions, deletions, substitutions and swaps of
/// two adjacent characters (Damerau–Levenshtein without repeated edits of a substring).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut rows: Vec<Vec<usize>> = vec![(0..=b.len()).collect()];
    for (i, &a_char) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, &b_char) in b.iter().enumerate() {
            let substitution = rows[i][j] + usize::from(a_char != b_char);
            let deletion = rows[i][j + 1] + 1;
            let insertion = row[j] + 1;
            let mut best = substitution.min(deletion).min(insertion);
            if i > 0 && j > 0 && a_char == b[j - 1] && a[i - 1] == b_char {
                best = best.min(rows[i - 1][j - 1] + 1);
            }
            row.push(best);
        }
        rows.push(row);
    }
    rows[a.len()][b.len()]
}

fn push_plain(markup: &str, plain: &mut String) {
    let mut rest = markup;
    while let Some(offset) = rest.find([
        ESCAPE,
        BACKTICK,
        '[',
        '<',
        EMPHASIS_STAR,
        EMPHASIS_UNDERSCORE,
    ]) {
        plain.push_str(&rest[..offset]);
        let at = &rest[offset..];
        rest = match at.chars().next() {
            Some(ESCAPE) => {
                let after_escaped = skip_escape(at);
                plain.push_str(&at[1..at.len() - after_escaped.len()]);
                after_escaped
            }
            Some(BACKTICK) => match backticks(at) {
                Backticks::Span { content, rest } => {
                    plain.push_str(content);
                    rest
                }
                Backticks::Unmatched { rest } => {
                    plain.push_str(&at[..at.len() - rest.len()]);
                    rest
                }
            },
            Some('[') => match link_text(at) {
                Some((text, after_link)) => {
                    push_plain(text, plain);
                    after_link
                }
                None => {
                    plain.push('[');
                    &at[1..]
                }
            },
            Some('<') => after_comment(at).unwrap_or_else(|| {
                plain.push('<');
                &at[1..]
            }),
            Some(EMPHASIS_STAR) => &at[1..],
            _ => {
                let after_run = at.trim_start_matches(EMPHASIS_UNDERSCORE);
                if !is_emphasis_run(plain.chars().next_back(), after_run.chars().next()) {
                    plain.push_str(&at[..at.len() - after_run.len()]);
                }
                after_run
            }
        };
    }
    plain.push_str(rest);
}

/// `at` starts with `[`: the text of an inline link and what follows the link.
fn link_text(at: &str) -> Option<(&str, &str)> {
    let Bracketed { inside, after } = bracketed(at)?;
    let (_, rest) = link_destination(after)?;
    Some((inside, rest))
}

/// A run of `_` delimits emphasis unless it sits inside a word (alphanumeric on both
/// sides) or has whitespace or nothing on both sides.
fn is_emphasis_run(before: Option<char>, after: Option<char>) -> bool {
    let inside_word =
        before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric);
    let touches_text =
        before.is_some_and(|c| !c.is_whitespace()) || after.is_some_and(|c| !c.is_whitespace());
    touches_text && !inside_word
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

    fn items(text: &str) -> Vec<Inline> {
        scan(text).into_iter().map(|found| found.item).collect()
    }

    #[test]
    fn links_and_images_are_classified_by_target() {
        let found = items(
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
    fn links_report_the_offset_of_their_opening_bracket() {
        let found = scan("see [one](a.md) and `x` [two *and* [three](c.md)](b.md)");

        let offsets: Vec<_> = found.iter().map(|f| f.offset).collect();
        assert_eq!(offsets, [4, 24, 35]);
    }

    #[test]
    fn links_inside_code_spans_and_escapes_are_ignored() {
        let found = items("`[a](b.md)` and ``[c](d.md) ` e`` and \\[f](g.md) and [h](i.md)");
        assert_eq!(found, [relative("i.md", None)]);
    }

    #[test]
    fn destinations_may_hold_balanced_parentheses_and_surrounding_spaces() {
        let found = items("[a](x_(1).md) [b]( y.md \"title\" ) [c](broken.md");
        assert_eq!(found, [relative("x_(1).md", None), relative("y.md", None)]);
    }

    #[test]
    fn single_quoted_titles_do_not_drop_the_link() {
        let found = items("[a](gone.md 'Title') [b](b.md 'two words') [c](c.md 'open");
        assert_eq!(found, [relative("gone.md", None), relative("b.md", None)]);
    }

    #[test]
    fn parenthesised_titles_do_not_drop_the_link() {
        let found = items("[a](gone.md (Title)) [b](b.md (two words)) [c](c.md (open)");
        assert_eq!(found, [relative("gone.md", None), relative("b.md", None)]);
    }

    #[test]
    fn angle_bracket_destinations_may_contain_spaces() {
        let found = items("[a](<b c.md>) [b](<d.md#x> 'T') [c](<broken.md) [d](<e\n.md>)");
        assert_eq!(
            found,
            [relative("b c.md", None), relative("d.md", Some("x"))]
        );
    }

    #[test]
    fn nested_brackets_and_code_in_link_text_keep_the_link() {
        let found = items("[see `a[0]` and [y]](z.md)");
        assert_eq!(found, [relative("z.md", None)]);
    }

    #[test]
    fn links_and_code_spans_may_wrap_across_lines() {
        let found = items("see [the long\nlink text](gone.md) and `a\nspan [x](skipped.md)` end");
        assert_eq!(found, [relative("gone.md", None)]);
    }

    #[test]
    fn html_comments_are_skipped_but_unclosed_ones_are_text() {
        let found = items("a <!-- [x](gone.md)\n`[open]` --> b [y](y.md) <!-- [z](z.md)");
        assert_eq!(found, [relative("y.md", None), relative("z.md", None)]);
    }

    #[test]
    fn known_markers_count_in_code_and_plain_text() {
        let found = items(
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
        let found = items("[verify](a.md) [open][ref] [decided]");
        assert_eq!(
            found,
            [relative("a.md", None), Inline::Marker(Marker::Decided)]
        );
    }

    #[test]
    fn marker_typos_in_code_spans_are_reported() {
        let found = items("`[verfiy]` `[propsed]` `[opne, soon]` `[Verify]`");
        assert_eq!(
            found,
            [
                Inline::UnknownMarker("verfiy".to_owned()),
                Inline::UnknownMarker("propsed".to_owned()),
                Inline::UnknownMarker("opne".to_owned()),
            ]
        );
    }

    #[test]
    fn marker_shaped_code_that_is_no_typo_is_left_alone() {
        let found = items(
            "`[bans]` `[error]` `[workspace]` `[min, max]` `[none]` `[integrity]` `[u8]` `[T]` `[a-z]+` `[]`",
        );
        assert_eq!(found, []);
    }

    #[test]
    fn bracketed_unknown_words_in_prose_are_ignored() {
        assert_eq!(items("the [none] word and [verfiy] arrays"), []);
    }

    #[test]
    fn plain_text_reduces_links_and_drops_emphasis_but_keeps_code_verbatim() {
        let cases = [
            ("See [intro](a.md) now", "See intro now"),
            ("See [`x`](a.md#b) and [t](<c d.md> 'T')", "See x and t"),
            ("_Why_ **now** and *then*", "Why now and then"),
            (
                "snake_case and __init__ and a _ b",
                "snake_case and init and a _ b",
            ),
            ("`snake_case` `a*b` kept", "snake_case a*b kept"),
            ("[proposed] [x](", "[proposed] [x]("),
            (
                "Title <!-- note --> end \\_literal\\_",
                "Title  end _literal_",
            ),
        ];
        for (markup, expected) in cases {
            assert_eq!(plain_text(markup), expected, "markup: {markup}");
        }
    }

    #[test]
    fn edit_distance_counts_adjacent_swaps_as_one_edit() {
        assert_eq!(edit_distance("verfiy", "verify"), 1);
        assert_eq!(edit_distance("propsed", "proposed"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "open"), 4);
    }
}
