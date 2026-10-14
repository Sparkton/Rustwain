---
name: commit-and-push
description: Commit, branch, push, and open pull requests for this repository. Use whenever the user asks to commit, push, publish, ship, or open/create a PR, or when the orchestrate-code-changes workflow reaches its ship step.
---

# Commit and Push

Only commit, push, or open a PR when the user has asked for it.

## Branch

- Name every pushed branch `sparkton/<brief>`, where `<brief>` is a short lowercase kebab-case summary of what the push contains (for example `sparkton/agent-skills-and-policy`, `sparkton/node-agent-design`).
- Never push to `main` directly. When the work sits on `main` or on a branch with any other name, create the `sparkton/` branch from the current commit before pushing:
  ```bash
  git switch -c sparkton/<brief>
  ```
- One branch per PR. Keep the brief accurate to the whole branch, not just the latest commit.

## Commit

- Review `git status` and the staged diff; stage only files that belong to the change. Never commit secrets, local env files, or build output.
- Run `git diff --cached --check` and fix whitespace errors before committing.
- Write an imperative subject line of 72 characters or fewer, then a blank line and a short body explaining what changed and why.
- Never add a `Co-Authored-By:` trailer for Claude or any AI, and no other AI attribution line.
- Do not amend, rebase, or force-push commits that are already pushed unless the user asks.

## Push

```bash
git push -u origin sparkton/<brief>
```

If the push is rejected, report why. Do not force-push to resolve it unless the user asks.

## Pull Request

Open the PR against `main` with `gh pr create`. Title: the change in a few words, in imperative mood.

Body format, with no AI attribution line:

```markdown
## What was added
- <one-liner per notable addition or change>
- <...>

## Major decisions
- <one-liner per significant decision, with the reason in a few words>
```

- Keep each bullet to one line. Two to six bullets per section is typical.
- When there were no major decisions, write `- None.` rather than omitting the section.
- When the PR delivers a design doc's ticket, link the doc under "What was added".

After opening, report the PR URL to the user.
