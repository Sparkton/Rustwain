---
name: write-project-docs
description: Write and maintain project documentation in docs/ai_generated. Use for documentation requests, system or component overviews, and persisting troubleshooting findings.
---

# Writing Project Documentation

This skill describes how to produce correct, coherent, and reusable technical documentation for the `docs/ai_generated/` directory.

## When to Use

Use this skill whenever you need to document a system, feature, architectural component, or troubleshooting finding. Typical triggers:
- The user asks you to "document", "write an overview of", or "explain how X works" and wants a file.
- You have finished a deep-dive into a system and the user wants the knowledge persisted.
- You have diagnosed and resolved a non-obvious issue and the user wants the findings documented for future reference.

> **Note on Naming:** Avoid hardcoding the repository or product name in documentation. Use generic terms like 'the project' or 'the system' so docs stay accurate if the name changes.

## Output Location

Docs are organized by type:

| Type | Path | Naming |
|------|------|--------|
| System overviews | `docs/ai_generated/<topic>_overview.md` | snake_case, ending with `_overview.md` |
| Troubleshooting guides | `docs/ai_generated/troubleshooting/<topic>.md` | snake_case, descriptive topic name |

**Examples:**
*   `docs/ai_generated/scheduler_overview.md`
*   `docs/ai_generated/troubleshooting/cargo_rebuilds.md`

## Research Phase (Before Writing)

1. **Read all existing docs** in `docs/ai_generated/` to avoid duplication and to match the established tone.
2. **Explore the codebase thoroughly** — don't write from memory or assumptions. Use the available search and file-reading tools to gather facts.
3. **Trace initialization and usage paths** — identify entry points, configuration, environment variables, and how components connect.
4. **Check for gaps** — look for commented-out code, TODOs, and inactive features. Document their status honestly.
5. **Use DeepWiki for external dependencies** — use the DeepWiki MCP (if configured) to research third-party dependency APIs and patterns.

## Document Structure

Choose the template that matches your document type. Omit sections that don't apply, but keep the numbered heading pattern.

### Overview Template

Use for system, feature, or architectural documentation.

```markdown
> Last Updated: YYYY-MM-DD

# <System Name> Overview

One-sentence summary of what this document covers.

## 1. Architecture Overview

High-level bullet list of components and how they fit together. Bold component names. Use backticks for code references (crate names, function names, env vars).

## 2–N. Core Sections

Each major component or concept gets its own numbered section.

## N+1. Environment Variables Summary

Table: Variable | Used By | Effect

## N+2. Key Files Reference

Table: File (relative path) | Purpose

## N+3. Gaps / To-Do (if applicable)

Honest list of missing features, inactive code, or known issues.
```

### Troubleshooting Template

Use for documenting diagnosed issues, root causes, and fixes.

```markdown
> Last Updated: YYYY-MM-DD

# <Problem> — Troubleshooting Guide

One-sentence summary of what this document covers.

## 1. Symptom

What the user observes. Include exact error messages, logs, or build output.

## 2. Root Cause

Technical explanation of why the problem occurs. Include diagnostic commands.

## 3. Fixes

### Option A: <Recommended Fix>

Concrete steps for the primary fix.

### Option B–N: <Alternative Fixes>

Each alternative gets its own subsection.

## 4. Verification

How to confirm the fix worked — exact commands and expected output.

## 5. Key Files Reference

Table: File (relative path) | Purpose
```

## Style Rules

### Formatting
- **First line**: `> Last Updated: YYYY-MM-DD` (use the current date).
- **Headings**: `# Title`, `## 1. Section`, `### Subsection`. Always number top-level sections.
- **Lists**: Use `*   ` (3-space indent after asterisk) for unordered lists, matching the existing docs.
- **Tables**: Use markdown tables for structured data (env vars, file references, configuration options, method summaries).
- **Code blocks**: Use fenced blocks with language tags (`rust`, `bash`, `sql`, etc.).
- **Warnings**: Use `**⚠️ text**` for important caveats (e.g., inactive features, deprecated code).

### Tone & Content
- **Be factual, not aspirational.** Document what the code *does*, not what it *should* do. If something is commented out or inactive, say so explicitly.
- **Be specific.** Include file paths (relative to project root), function names, crate versions, env var names, and CLI flags.
- **Provide examples.** Show real usage from the codebase — actual CLI invocations, actual code patterns.
- **Document the "why"**, not just the "what", when the reasoning isn't obvious (e.g., why a timeout or retry budget has its value).
- **Use bold** for component names and key terms on first mention.
- **Use backticks** for all code identifiers: crate names, function names, env vars, CLI args, types.
- **Keep sentences short.** Prefer lists and tables over paragraphs.

### What NOT to Do
- Do NOT invent or assume functionality. Only document what you verified in the source code.
- Do NOT include mermaid diagrams, flow charts, or other rendered visuals — keep it plain markdown that renders everywhere.
- Do NOT duplicate content from other docs in `ai_generated/`. Cross-reference instead.
- Do NOT use GitHub alert syntax (`> [!NOTE]`) — the existing docs don't use it.
- Do NOT repeat the same information in multiple sections.
- Do NOT include the artifact directory path or any IDE-specific metadata.

## Verification Checklist

Before finishing, verify:
- [ ] Every file path mentioned actually exists.
- [ ] Every env var, CLI arg, and function name is spelled correctly (grep to confirm).
- [ ] Every "used by" claim is verified by checking imports/call sites.
- [ ] Inactive/commented-out features are clearly marked as such.
- [ ] The doc has a `> Last Updated:` line with today's date.
- [ ] The filename follows the correct convention for its type (see Output Location).
- [ ] The structure matches the appropriate template (overview or troubleshooting).
- [ ] No information is duplicated across sections.

## Example Reference

Once `docs/ai_generated/` contains docs, treat the most recent overview and troubleshooting guide as the reference for format and tone. Until then, follow the templates above exactly.
