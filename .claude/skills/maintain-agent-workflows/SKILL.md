---
name: maintain-agent-workflows
description: Maintain the project's agent instruction layer. Use whenever creating, editing, reviewing, renaming, or removing AGENTS.md/CLAUDE.md policy, project-local skills under .agent/skills, their .claude/skills mirror, or agent-workflow evolution records.
---

# Maintain Agent Workflows

Keep active agent guidance concise, discoverable, and owned by one layer, while preserving the rationale for material workflow changes.

## Establish Ownership

Place each instruction at the narrowest reliable loading layer:

- `AGENTS.md`: always-loaded cross-task policy and safety boundaries.
- `CLAUDE.md`: only `@AGENTS.md`, so Claude Code loads the same policy. Never add rules here.
- `.agent/skills/<name>/SKILL.md`: canonical, discoverable task-specific procedures and non-obvious domain knowledge.
- `.claude/skills/<name>`: checked-in Claude Code mirror of the canonical skill directory. Never edit it directly.
- Task runner recipes (`Justfile`, `Makefile`, `xtask`): executable commands and their implementation details.
- `docs/evolution/agent-workflows.md`: rationale and history for material agent-workflow decisions.
- `docs/design/`: component design decisions (owned by `write-design-doc`), not agent policy.

Apply these boundaries:

- Give each active rule one owner.
- Treat references between instruction layers like software interfaces. State only the required contract; keep procedure, rationale, and implementation in the owning layer.
- When another skill owns a decision, route with `Invoke and follow <skill> when its trigger applies.`

## Write for Semantic Retrieval

- Keep each heading, paragraph, and bullet focused on one concept so it carries a clear retrieval signal.
- Separate distinct concepts into their own sections or list items.
- Lead with the required behavior and name the canonical command or workflow.
- Reserve negative instructions for essential prohibitions, permission boundaries, or safety constraints with no equally clear positive replacement.

## Maintain Always-Loaded Policy

- Keep policy short enough to stay useful when always loaded.
- State durable behavior and decision boundaries, not command tutorials.
- Rely on native skill discovery instead of duplicating skill names, descriptions, or paths.

## Maintain Project Skills

- Treat `.agent/skills` as the canonical project skill directory.
- Use a lowercase kebab-case folder and a matching frontmatter `name`.
- Put only `name` and a trigger-complete `description` in YAML frontmatter. Describe both the capability and the requests or files that should activate it.
- Keep skills project-generic: no names, tables, crates, or commands from other projects, and no hardcoded paths that do not exist in this repository.
- Keep guidance provider-neutral where possible. Agent-runtime-specific skills (such as orchestration via a particular agent tool) are allowed when the workflow inherently depends on that runtime; say so in the skill.
- Write the body in imperative form. Include only workflow knowledge an agent cannot reliably infer from the repository.
- Prefer a concise `SKILL.md`; add scripts, references, or assets only when they provide reusable value. Keep folder names free of spaces.
- After adding, renaming, or removing a skill, mirror the canonical directory and commit the mirror with the skill change:
  ```bash
  rsync -a --delete .agent/skills/ .claude/skills/
  ```

## Check Changes

- After renaming or removing guidance, search for stale names and paths.
- Verify the mirror matches: `diff -r .agent/skills .claude/skills`.
- Run `git diff --check` and review Git status to avoid including unrelated changes.
