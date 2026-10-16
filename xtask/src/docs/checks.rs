//! The docs checks: pure functions over a [`DocSet`].

use std::{
    collections::{BTreeSet, VecDeque},
    num::NonZeroU32,
};

use super::model::{DocSet, LinkTarget, Marker, RelPath};
use crate::report::{Check, Finding, Location};

/// The doc every other doc must be reachable from.
const INDEX_DOC: &str = "docs/README.md";

/// Runs `DocLinks`, `DocAnchors`, `DocIndex` and `Markers`.
pub fn check(set: &DocSet) -> Vec<Finding> {
    let mut findings = doc_links(set);
    findings.extend(doc_anchors(set));
    findings.extend(doc_index(set));
    findings.extend(markers(set));
    findings
}

/// Relative links whose target is neither a doc in the set nor an existing path.
pub fn doc_links(set: &DocSet) -> Vec<Finding> {
    set.relative_links()
        .filter_map(|link| {
            let problem = match &link.resolved {
                None => "escapes the repository root".to_owned(),
                Some(resolved) if set.target_exists(resolved) => return None,
                Some(resolved) => format!("`{resolved}` does not exist"),
            };
            Some(Finding::error(
                Check::DocLinks,
                Some(Location::new(link.doc.as_str(), link.line)),
                format!("broken link `{}`: {problem}", link.written()),
            ))
        })
        .collect()
}

/// Anchors with no matching heading in the doc they point into. Anchors into targets
/// that are not docs are not checked.
pub fn doc_anchors(set: &DocSet) -> Vec<Finding> {
    set.docs
        .iter()
        .flat_map(|(path, doc)| {
            doc.links.iter().filter_map(move |link| {
                let (anchor, (target_path, target)) = match &link.target {
                    LinkTarget::SameDoc { anchor } => (anchor, (path, doc)),
                    LinkTarget::Relative {
                        path: relative,
                        anchor: Some(anchor),
                    } => {
                        let resolved = path.resolve_link(relative)?;
                        (anchor, set.docs.get_key_value(&resolved)?)
                    }
                    LinkTarget::Relative { anchor: None, .. } | LinkTarget::External(_) => {
                        return None;
                    }
                };
                // `[x](#)` is GitHub's link to the top of the page.
                let has_heading =
                    anchor.is_empty() || target.headings.iter().any(|h| h.anchor == *anchor);
                (!has_heading).then(|| {
                    Finding::error(
                        Check::DocAnchors,
                        Some(Location::new(path.as_str(), link.line)),
                        format!("no heading `#{anchor}` in `{target_path}`"),
                    )
                })
            })
        })
        .collect()
}

/// Docs that cannot be reached from `docs/README.md` by following relative links. A
/// missing `docs/README.md` is reported once, not once per doc.
pub fn doc_index(set: &DocSet) -> Vec<Finding> {
    let Some((index, _)) = set.docs.get_key_value(&RelPath::new(INDEX_DOC)) else {
        return vec![Finding::error(
            Check::DocIndex,
            None,
            format!("{INDEX_DOC} is missing, so no doc is indexed"),
        )];
    };
    let reached = reachable_docs(set, index);
    set.docs
        .keys()
        .filter(|path| !reached.contains(path))
        .map(|path| {
            Finding::error(
                Check::DocIndex,
                Some(Location::new(path.as_str(), NonZeroU32::MIN)),
                format!("not reachable from {INDEX_DOC}"),
            )
        })
        .collect()
}

/// Breadth-first search over relative links that resolve to docs.
fn reachable_docs<'a>(set: &'a DocSet, start: &'a RelPath) -> BTreeSet<&'a RelPath> {
    let mut reached = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(current) = queue.pop_front() {
        let Some(doc) = set.docs.get(current) else {
            continue;
        };
        let linked_docs = doc
            .links
            .iter()
            .filter_map(|link| match &link.target {
                LinkTarget::Relative { path, .. } => current.resolve_link(path),
                LinkTarget::External(_) | LinkTarget::SameDoc { .. } => None,
            })
            .filter_map(|resolved| set.docs.get_key_value(&resolved).map(|(key, _)| key));
        for next in linked_docs {
            if reached.insert(next) {
                queue.push_back(next);
            }
        }
    }
    reached
}

/// Marker-shaped inline code spans whose word is not a known marker.
pub fn markers(set: &DocSet) -> Vec<Finding> {
    let expected = Marker::ALL
        .iter()
        .map(|marker| format!("`[{}]`", marker.word()))
        .collect::<Vec<_>>()
        .join(", ");
    set.docs
        .iter()
        .flat_map(|(path, doc)| {
            doc.unknown_markers
                .iter()
                .map(move |(word, line)| (path, word, *line))
        })
        .map(|(path, word, line)| {
            Finding::error(
                Check::Markers,
                Some(Location::new(path.as_str(), line)),
                format!("unknown marker `[{word}]`; expected one of {expected}"),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::parse_doc;

    /// Builds a set from `(path, markdown)` pairs and the non-doc paths that exist.
    fn fixture(docs: &[(&str, &str)], existing: &[&str]) -> DocSet {
        DocSet {
            docs: docs
                .iter()
                .map(|(path, text)| (RelPath::new(*path), parse_doc(text)))
                .collect(),
            existing_targets: existing.iter().map(|path| RelPath::new(*path)).collect(),
        }
    }

    fn located(findings: &[Finding]) -> Vec<(Check, String, u32, String)> {
        findings
            .iter()
            .map(|f| {
                let at = f.location.as_ref().unwrap();
                (
                    f.check,
                    at.path.display().to_string(),
                    at.line.get(),
                    f.message.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn doc_links_pass_for_docs_and_existing_targets() {
        let set = fixture(
            &[
                (
                    "docs/README.md",
                    "[a](a/one.md#x) [top](../README.md) [d](a/)",
                ),
                (
                    "docs/a/one.md",
                    "# X\n[back](../README.md) [web](https://x.y/z.md)",
                ),
            ],
            &["README.md", "docs/a/"],
        );

        assert_eq!(doc_links(&set), []);
    }

    #[test]
    fn doc_links_report_missing_targets_at_their_line() {
        let set = fixture(
            &[
                (
                    "docs/README.md",
                    "text\n[a](a/gone.md#x) [up](../../x.md)\n[ok](a/one.md)",
                ),
                ("docs/a/one.md", "[dir](one.md/)"),
            ],
            &[],
        );

        assert_eq!(
            located(&doc_links(&set)),
            [
                (
                    Check::DocLinks,
                    "docs/README.md".to_owned(),
                    2,
                    "broken link `a/gone.md#x`: `docs/a/gone.md` does not exist".to_owned()
                ),
                (
                    Check::DocLinks,
                    "docs/README.md".to_owned(),
                    2,
                    "broken link `../../x.md`: escapes the repository root".to_owned()
                ),
                (
                    Check::DocLinks,
                    "docs/a/one.md".to_owned(),
                    1,
                    "broken link `one.md/`: `docs/a/one.md/` does not exist".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn doc_anchors_pass_for_existing_headings_and_non_doc_targets() {
        let set = fixture(
            &[
                (
                    "docs/README.md",
                    "# Top\n[self](#top) [empty](#) [other](a.md#m0--foundations) [two](a.md#notes-1) [code](x.rs#L10)",
                ),
                ("docs/a.md", "# M0 – Foundations\n## Notes\n## Notes"),
            ],
            &["docs/x.rs"],
        );

        assert_eq!(doc_anchors(&set), []);
    }

    #[test]
    fn doc_anchors_report_missing_same_doc_and_cross_doc_anchors() {
        let set = fixture(
            &[
                (
                    "docs/README.md",
                    "# Top\n[self](#nowhere)\n[other](a.md#missing)",
                ),
                ("docs/a.md", "# A\n[back](README.md#Top)"),
            ],
            &[],
        );

        assert_eq!(
            located(&doc_anchors(&set)),
            [
                (
                    Check::DocAnchors,
                    "docs/README.md".to_owned(),
                    2,
                    "no heading `#nowhere` in `docs/README.md`".to_owned()
                ),
                (
                    Check::DocAnchors,
                    "docs/README.md".to_owned(),
                    3,
                    "no heading `#missing` in `docs/a.md`".to_owned()
                ),
                (
                    Check::DocAnchors,
                    "docs/a.md".to_owned(),
                    2,
                    "no heading `#Top` in `docs/README.md`".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn doc_index_passes_when_every_doc_is_reachable_through_links() {
        let set = fixture(
            &[
                ("docs/README.md", "[a](a/one.md)"),
                ("docs/a/one.md", "[two](two.md#x)"),
                ("docs/a/two.md", "[home](../README.md)"),
            ],
            &[],
        );

        assert_eq!(doc_index(&set), []);
    }

    #[test]
    fn doc_index_reports_unreachable_docs_at_line_one() {
        let set = fixture(
            &[
                ("docs/README.md", "[a](a.md) [ext](https://x.y/c.md)"),
                ("docs/a.md", "text"),
                ("docs/b.md", "[a](a.md)"),
                ("docs/c.md", "[b](b.md)"),
            ],
            &[],
        );

        assert_eq!(
            located(&doc_index(&set)),
            [
                (
                    Check::DocIndex,
                    "docs/b.md".to_owned(),
                    1,
                    "not reachable from docs/README.md".to_owned()
                ),
                (
                    Check::DocIndex,
                    "docs/c.md".to_owned(),
                    1,
                    "not reachable from docs/README.md".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn doc_index_reports_a_missing_index_doc_once() {
        let set = fixture(&[("docs/a.md", "text"), ("docs/b.md", "text")], &[]);

        let findings = doc_index(&set);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].location, None);
    }

    #[test]
    fn markers_pass_for_known_markers_and_ignore_fenced_code() {
        let set = fixture(
            &[(
                "docs/README.md",
                "`[open]` **[verify]**\n```\n`[bogus]`\n```\n",
            )],
            &[],
        );

        assert_eq!(markers(&set), []);
    }

    #[test]
    fn markers_report_unknown_words_at_their_line() {
        let set = fixture(
            &[(
                "docs/README.md",
                "ok `[open]`\n\nbad `[maybe]` and `[nope, really]`",
            )],
            &[],
        );

        let expected =
            "expected one of `[proposed]`, `[decided]`, `[open]`, `[later]`, `[skip]`, `[verify]`";
        assert_eq!(
            located(&markers(&set)),
            [
                (
                    Check::Markers,
                    "docs/README.md".to_owned(),
                    3,
                    format!("unknown marker `[maybe]`; {expected}")
                ),
                (
                    Check::Markers,
                    "docs/README.md".to_owned(),
                    3,
                    format!("unknown marker `[nope]`; {expected}")
                ),
            ]
        );
    }

    #[test]
    fn check_runs_the_four_checks_in_order() {
        let set = fixture(
            &[
                ("docs/README.md", "[gone](gone.md) [x](#nope) `[maybe]`"),
                ("docs/lost.md", "text"),
            ],
            &[],
        );

        let order: Vec<_> = check(&set).iter().map(|f| f.check).collect();

        assert_eq!(
            order,
            [
                Check::DocLinks,
                Check::DocAnchors,
                Check::DocIndex,
                Check::Markers
            ]
        );
    }
}
