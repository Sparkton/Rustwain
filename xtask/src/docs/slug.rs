//! GitHub heading anchors.

use std::collections::BTreeSet;

use super::inline::plain_text;

/// The anchor GitHub generates for a heading's rendered text, before duplicate suffixes,
/// which [`Anchors`] adds.
///
/// Lowercases, drops everything except letters, numbers, spaces, `-` and `_`, then turns
/// each space into `-` without collapsing runs. Backticks are dropped but the code span's
/// content is kept.
///
/// Combining marks (Unicode `\p{M}`) are dropped, where GitHub keeps them: std has no
/// category table for them, and the library is written in English.
pub fn github_slug(heading: &str) -> String {
    heading
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// Hands out the anchors of one doc's headings in order, suffixing duplicates with `-1`,
/// `-2`, … as GitHub does.
#[derive(Debug, Default)]
pub(super) struct Anchors {
    used: BTreeSet<String>,
}

impl Anchors {
    /// The anchor for a heading written as `markup`: links, emphasis and comments are
    /// rendered away first, so `## See [intro](a.md)` anchors as `see-intro`.
    pub fn allocate(&mut self, markup: &str) -> String {
        let base = github_slug(&plain_text(markup));
        let mut anchor = base.clone();
        let mut suffix = 0u32;
        while self.used.contains(&anchor) {
            suffix += 1;
            anchor = format!("{base}-{suffix}");
        }
        self.used.insert(anchor.clone());
        anchor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_follows_github_rules() {
        let cases = [
            ("Kubernetes Today", "kubernetes-today"),
            ("M0 – Foundations", "m0--foundations"),
            ("`foo` bar", "foo-bar"),
            ("Pods [and] Nodes: a/b & c.d", "pods-and-nodes-ab--cd"),
            (
                "WireGuard `[proposed default when enabled]`",
                "wireguard-proposed-default-when-enabled",
            ),
            ("What's new (v2)?", "whats-new-v2"),
            ("no_new_privs", "no_new_privs"),
            ("Über-Cgroups 2", "über-cgroups-2"),
        ];
        for (heading, expected) in cases {
            assert_eq!(github_slug(heading), expected, "heading: {heading}");
        }
    }

    #[test]
    fn heading_markup_is_rendered_before_slugging() {
        let cases = [
            ("See [intro](a.md) now", "see-intro-now"),
            ("_Why_ **now**", "why-now"),
            ("snake_case naming", "snake_case-naming"),
            ("`snake_case` and `a*b`", "snake_case-and-ab"),
        ];
        for (markup, expected) in cases {
            assert_eq!(
                Anchors::default().allocate(markup),
                expected,
                "markup: {markup}"
            );
        }
    }

    #[test]
    fn duplicate_headings_get_numbered_suffixes() {
        let mut anchors = Anchors::default();
        let allocated: Vec<_> = ["Notes", "Notes", "Other", "Notes", "Notes 1"]
            .into_iter()
            .map(|heading| anchors.allocate(heading))
            .collect();
        assert_eq!(
            allocated,
            ["notes", "notes-1", "other", "notes-2", "notes-1-1"]
        );
    }
}
