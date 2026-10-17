//! `BranchName`: pushed branches are `sparkton/<brief>`.

use crate::report::{Check, Finding};

const MAIN_BRANCH: &str = "main";
const BRANCH_PREFIX: &str = "sparkton/";

const ENV_ACTIONS: &str = "GITHUB_ACTIONS";
const ENV_HEAD_REF: &str = "GITHUB_HEAD_REF";
const ENV_REF_TYPE: &str = "GITHUB_REF_TYPE";
const ENV_REF_NAME: &str = "GITHUB_REF_NAME";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchContext {
    pub name: String,
    pub source: BranchSource,
}

impl BranchContext {
    pub fn new(name: impl Into<String>, source: BranchSource) -> Self {
        Self {
            name: name.into(),
            source,
        }
    }
}

/// Where the branch name came from, which decides the severity of a bad name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchSource {
    /// `GITHUB_HEAD_REF` of a pull request.
    PullRequest,
    /// `GITHUB_REF_NAME` of a branch push.
    Push,
    /// The local checkout's current branch.
    Local,
}

/// `None` when there is no branch to check (detached HEAD, a tag push).
///
/// A bad name is an error for a pull request or a push, and only a warning for a local
/// checkout, because a local branch name matters once it is pushed.
pub fn branch_name(branch: Option<&BranchContext>) -> Vec<Finding> {
    let Some(branch) = branch.filter(|branch| !is_valid_branch_name(&branch.name)) else {
        return Vec::new();
    };
    let message = format!(
        "branch `{}` must be `{BRANCH_PREFIX}<brief>` (AGENTS.md)",
        branch.name
    );
    let finding = match branch.source {
        BranchSource::PullRequest | BranchSource::Push => {
            Finding::error(Check::BranchName, None, message)
        }
        BranchSource::Local => Finding::warning(Check::BranchName, None, message),
    };
    vec![finding]
}

/// `main`, or `sparkton/` and a brief of lowercase words joined by single dashes.
fn is_valid_branch_name(name: &str) -> bool {
    name == MAIN_BRANCH
        || name
            .strip_prefix(BRANCH_PREFIX)
            .is_some_and(|brief| brief.split('-').all(is_brief_word))
}

fn is_brief_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// The variables that name the branch on a GitHub Actions run, read once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct GithubEnv {
    actions: Option<String>,
    head_ref: Option<String>,
    ref_type: Option<String>,
    ref_name: Option<String>,
}

impl GithubEnv {
    pub(super) fn from_process() -> Self {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// An empty variable counts as unset.
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let var = |name: &str| var(name).filter(|value| !value.is_empty());
        Self {
            actions: var(ENV_ACTIONS),
            head_ref: var(ENV_HEAD_REF),
            ref_type: var(ENV_REF_TYPE),
            ref_name: var(ENV_REF_NAME),
        }
    }
}

/// How to find the branch under check, decided from the environment alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BranchLookup {
    /// A CI run that names its branch.
    Named(BranchContext),
    /// A CI run without a branch, such as a tag push.
    NoBranch,
    /// Not CI: ask `git` for the checkout's current branch.
    LocalCheckout,
}

impl BranchLookup {
    pub(super) fn from_env(env: &GithubEnv) -> Self {
        if let Some(head_ref) = &env.head_ref {
            return Self::Named(BranchContext::new(head_ref, BranchSource::PullRequest));
        }
        if env.actions.as_deref() != Some("true") {
            return Self::LocalCheckout;
        }
        match (env.ref_type.as_deref(), &env.ref_name) {
            (Some("branch"), Some(name)) => {
                Self::Named(BranchContext::new(name, BranchSource::Push))
            }
            _ => Self::NoBranch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Severity;

    fn env(vars: &[(&str, &str)]) -> GithubEnv {
        GithubEnv::from_vars(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn main_and_sparkton_briefs_are_valid_names() {
        let valid = [
            "main",
            "sparkton/ci-pipeline",
            "sparkton/t1",
            "sparkton/a-b-2",
        ];

        let findings: Vec<_> = valid
            .map(|name| BranchContext::new(name, BranchSource::PullRequest))
            .iter()
            .flat_map(|branch| branch_name(Some(branch)))
            .collect();

        assert_eq!(findings, []);
    }

    #[test]
    fn other_names_are_errors_for_pushes_and_warnings_for_local_checkouts() {
        let invalid = [
            "feature/x",
            "sparkton",
            "sparkton/",
            "sparkton/Upper",
            "sparkton/two/levels",
            "sparkton/-lead",
            "sparkton/trail-",
            "sparkton/double--dash",
            "sparkton/under_score",
            "Main",
        ];
        // One severity per name; a name that raised no finding would show as an empty list.
        let severities = |source| -> Vec<Vec<Severity>> {
            invalid
                .iter()
                .map(|name| branch_name(Some(&BranchContext::new(*name, source))))
                .map(|findings| findings.iter().map(|finding| finding.severity).collect())
                .collect()
        };
        let every = |severity| vec![vec![severity]; invalid.len()];

        assert_eq!(
            severities(BranchSource::PullRequest),
            every(Severity::Error)
        );
        assert_eq!(severities(BranchSource::Push), every(Severity::Error));
        assert_eq!(severities(BranchSource::Local), every(Severity::Warning));
        assert_eq!(
            branch_name(Some(&BranchContext::new("feature/x", BranchSource::Push))),
            [Finding::error(
                Check::BranchName,
                None,
                "branch `feature/x` must be `sparkton/<brief>` (AGENTS.md)"
            )]
        );
        assert_eq!(branch_name(None), []);
    }

    #[test]
    fn environment_decides_where_the_branch_name_comes_from() {
        let pull_request =
            BranchLookup::Named(BranchContext::new("sparkton/x", BranchSource::PullRequest));
        let push = BranchLookup::Named(BranchContext::new("sparkton/y", BranchSource::Push));
        let cases = [
            // A pull request run names its head branch, whatever else is set.
            (
                env(&[
                    ("GITHUB_ACTIONS", "true"),
                    ("GITHUB_HEAD_REF", "sparkton/x"),
                    ("GITHUB_REF_TYPE", "branch"),
                    ("GITHUB_REF_NAME", "12/merge"),
                ]),
                pull_request,
            ),
            // A branch push uses the ref name; an empty head ref counts as unset.
            (
                env(&[
                    ("GITHUB_ACTIONS", "true"),
                    ("GITHUB_HEAD_REF", ""),
                    ("GITHUB_REF_TYPE", "branch"),
                    ("GITHUB_REF_NAME", "sparkton/y"),
                ]),
                push,
            ),
            // A tag push has no branch.
            (
                env(&[
                    ("GITHUB_ACTIONS", "true"),
                    ("GITHUB_REF_TYPE", "tag"),
                    ("GITHUB_REF_NAME", "v1"),
                ]),
                BranchLookup::NoBranch,
            ),
            // Outside Actions, ask git.
            (env(&[]), BranchLookup::LocalCheckout),
            (
                env(&[("GITHUB_REF_TYPE", "branch"), ("GITHUB_REF_NAME", "main")]),
                BranchLookup::LocalCheckout,
            ),
        ];

        let lookups: Vec<_> = cases
            .iter()
            .map(|(env, _)| BranchLookup::from_env(env))
            .collect();

        let expected: Vec<_> = cases.into_iter().map(|(_, lookup)| lookup).collect();
        assert_eq!(lookups, expected);
    }
}
