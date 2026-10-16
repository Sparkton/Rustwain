---
name: log-anti-pattern
description: Append a lean entry to docs/anti_patterns.md capturing a codebase pattern that agents have repeatedly violated. Use when explicitly requested, when distilling recurring PR/CI feedback, or after the same material repository-wide mistake has been corrected at least twice in the current task or PR. Do not trigger for first-time, trivial, preference-only, stale-comment, or already-documented corrections. Semi-automatic — drafts the entry and waits for explicit user approval before writing.
---

# Log Anti-Pattern

Distills a repeated-violation pattern into `docs/anti_patterns.md` so future agents avoid it when modifying relevant code.

## When to Use

- User runs `/log-anti-pattern` (optionally with a source argument).
- User just corrected you on a pattern and asks you to record it.
- The same material repository-wide mistake has been corrected at least twice in the current task or PR.
- You're reviewing PR comments or CI output and notice a recurring critique.

Do not use this skill for first-time mistakes, trivial corrections, preferences, stale comments, or rules already documented in a canonical skill or guide unless the user explicitly asks to record them.

## Sources

`/log-anti-pattern` accepts an optional source:

- _no arg_ — extract from the most recent user correction in this session.
- `pr <number>` — run `gh pr view <number> --comments` and `gh pr view <number> --json reviews,comments` and distill repeated reviewer feedback.
- `ci <url-or-path>` — read the CI log and look for repeated lint/clippy/test-shape failures.
- Anything else — treat the rest of the arg as pasted context (e.g. a Slack quote).

## Steps

1. **Resolve source** per the list above. If the source is thin or ambiguous, stop and ask the user for more context rather than inventing a violation.
2. **Check for overlap without loading the whole file.** Search headings and candidate keywords in `docs/anti_patterns.md`, then read only the matching entry and relevant section boundary. Read the whole file only when targeted searches cannot establish the layout or overlap.
3. **Extract candidates.** For each distinct violation, identify:
   - what agents keep doing wrong,
   - the correct approach plus one pointer (file path, skill name, or existing helper),
   - the source of the feedback.
4. **Deduplicate.** For each candidate, grep the existing entries for overlap.
   - Overlap → draft an **update** to the existing entry (amend Source, tighten wording).
   - No overlap → draft a **new** entry.
5. **Enforce format** (max 5 lines, no code blocks — link out instead):

   ```md
   ### <short pattern name>
   - **Don't:** <the violation, one line>
   - **Do:** <correct approach + pointer, one line>
   - **Source:** <PR #, commit SHA, skill name, or "repeated review feedback">
   ```

6. **Show the draft** as a diff against `docs/anti_patterns.md` and ask the user to approve, edit, or reject. Do not proceed without explicit approval.
7. **On approval**, apply with the Edit tool. Place new entries in the relevant existing section; when no section fits, add a concise area-specific section. Do not write on rejection.

## Anti-patterns for this skill

- **Don't** write prose entries. If a pattern needs more than 5 lines, it belongs in a skill, not here.
- **Don't** auto-append without the user's explicit approval.
- **Don't** create a new entry when an existing one already covers the pattern — update instead.
- **Don't** load the entire anti-pattern file when a targeted heading or keyword search is enough.
- **Don't** invent violations that weren't in the source. Thin source → ask, don't fabricate.