//! The `docs report` table.

use super::model::{Doc, DocSet, Marker};

const TITLE: &str = "### Decision markers";
const HEADER: &str = "| Doc | [verify] | [open] | [proposed] |";
const ALIGNMENT: &str = "|---|---:|---:|---:|";

/// Markdown table of `[verify]`, `[open]` and `[proposed]` counts per doc, for
/// `docs report` and the job summary. Only docs with at least one of the three get a
/// row; rows are sorted by path and a final row totals the columns.
pub fn marker_report(set: &DocSet) -> String {
    let rows: Vec<_> = set
        .docs
        .iter()
        .map(|(path, doc)| (path, Counts::of(doc)))
        .filter(|(_, counts)| !counts.is_zero())
        .collect();
    let total = rows
        .iter()
        .fold(Counts::default(), |total, (_, counts)| total.plus(counts));

    let mut lines = vec![
        TITLE.to_owned(),
        String::new(),
        HEADER.to_owned(),
        ALIGNMENT.to_owned(),
    ];
    lines.extend(
        rows.iter()
            .map(|(path, counts)| counts.row(&format!("`{path}`"))),
    );
    lines.push(total.row("**Total**"));
    lines.push(String::new());
    lines.join("\n")
}

#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    verify: usize,
    open: usize,
    proposed: usize,
}

impl Counts {
    fn of(doc: &Doc) -> Self {
        doc.markers
            .iter()
            .fold(Self::default(), |counts, (marker, _)| match marker {
                Marker::Verify => Self {
                    verify: counts.verify + 1,
                    ..counts
                },
                Marker::Open => Self {
                    open: counts.open + 1,
                    ..counts
                },
                Marker::Proposed => Self {
                    proposed: counts.proposed + 1,
                    ..counts
                },
                Marker::Decided | Marker::Later | Marker::Skip => counts,
            })
    }

    fn plus(self, other: &Self) -> Self {
        Self {
            verify: self.verify + other.verify,
            open: self.open + other.open,
            proposed: self.proposed + other.proposed,
        }
    }

    fn is_zero(&self) -> bool {
        self.verify + self.open + self.proposed == 0
    }

    fn row(&self, label: &str) -> String {
        format!(
            "| {label} | {} | {} | {} |",
            self.verify, self.open, self.proposed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::{RelPath, parse_doc};

    fn set(docs: &[(&str, &str)]) -> DocSet {
        DocSet {
            docs: docs
                .iter()
                .map(|(path, text)| (RelPath::new(*path), parse_doc(text)))
                .collect(),
            ..DocSet::default()
        }
    }

    #[test]
    fn report_counts_the_three_markers_per_doc_and_in_total() {
        let set = set(&[
            ("docs/b.md", "`[open]` `[open]` **[verify]**\n`[later]`"),
            (
                "docs/a.md",
                "`[proposed]` `[decided]`\n[verify]\n```\n`[open]`\n```",
            ),
            ("docs/c.md", "`[later]` `[skip]`"),
        ]);

        assert_eq!(
            marker_report(&set),
            "### Decision markers\n\
             \n\
             | Doc | [verify] | [open] | [proposed] |\n\
             |---|---:|---:|---:|\n\
             | `docs/a.md` | 1 | 0 | 1 |\n\
             | `docs/b.md` | 1 | 2 | 0 |\n\
             | **Total** | 2 | 2 | 1 |\n"
        );
    }

    #[test]
    fn report_without_markers_has_only_a_zero_total() {
        let set = set(&[("docs/a.md", "plain")]);

        assert_eq!(
            marker_report(&set),
            "### Decision markers\n\
             \n\
             | Doc | [verify] | [open] | [proposed] |\n\
             |---|---:|---:|---:|\n\
             | **Total** | 0 | 0 | 0 |\n"
        );
    }
}
