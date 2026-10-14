---
name: write-code
description: Shared implementation, documentation, dependency-boundary, security, and testing conventions for project code. Use whenever writing, modifying, or reviewing code, together with the relevant language or platform skill.
---

# Writing Code

Apply these rules before `write-rust-code`. The project is Rust-only: no frontend, no WASM targets, and no non-Rust tooling (scripts and automation live in `cargo xtask`). Keep language-specific commands and conventions in the specialized skill.

## Understand the Existing System

- Before modifying code, review the relevant sections of `docs/anti_patterns.md` (when present) and treat applicable entries as constraints.
- Consult design docs, architecture decision records, `docs/ai_generated/`, package-level READMEs, and `docs/evolution/` when they cover the area being changed.
- Find the closest existing implementation, owner, helper, configuration path, and tests. Extend the established boundary instead of creating a parallel one.
- Use DeepWiki, when configured, to inspect public dependency repositories. Verify version-sensitive behavior against the pinned dependency, installed source or API surface, and primary vendor documentation; summaries are supporting evidence, not the API contract.

## Keep the Change Focused

- Prefer the simplest design that fully handles the supported behavior and failure modes.
- Refactor complex functions into cohesive phases or responsibilities.
- Add an abstraction only when it removes meaningful duplication, clarifies ownership, or creates a necessary test or dependency boundary.
- Keep core logic in its owning layer and keep transport, storage, and runtime glue narrow.
- Order declarations top-down: public or top-level entrypoints first, followed by the lower-level helpers they call.
- Replace non-obvious fixed values with named constants; do not name literals whose meaning is already clear at the call site.

## Keep Files Bounded

- Keep cohesive responsibilities together. Split a file only when the extracted responsibility has a clear owner and can be understood and changed independently.
- Keep module roots declarative. A module root (`mod.rs`, or a `lib.rs` facade) holds module declarations, re-exports, and its module-level overview. Logic belongs in a sibling source file.
- Once a handwritten source file exceeds 1,000 non-test lines, do not add another independent responsibility to it. Extract the new or an existing coherent responsibility into a clearly named sibling module first.
- Do not create one-function modules, thin wrappers, or arbitrary fragments solely to reduce line count. Keep tests beside their owning code, and preserve stable callers with explicit re-exports when appropriate.

## Document Intent

- Give each new module that owns meaningful behavior, data flow, policy, integration, or invariants a `//!` module-level overview. One sentence for a small module; for a complex one, explain its boundary, responsibilities, relationships, and invariants in proportion.
- Omit module overviews for generated files, pure re-exports, registration glue, and other trivial boundaries.
- Document new APIs, types, or state only when the documentation adds non-obvious purpose, ownership, contract, or failure semantics. Do not restate names, signatures, types, or attributes.
- Document non-obvious constraints and the invariants they protect.
- Document a deliberate limitation where the code accepts it: the case it leaves out, why, and how that case is handled instead. A reviewer who can't see the reason will report it as a bug.
- Separate paragraphs in doc comments with a blank `//!` or `///` line.
- Omit boilerplate headers and comments that merely narrate the code.

## Protect Boundaries

- Use the existing configuration and secret-loading mechanisms. Never hardcode or log secrets, or raw sensitive payloads.
- Isolate external SDK, network, storage, clock, and runtime behavior behind the smallest practical interface when deterministic testing or lifecycle orchestration requires it.
- Change source definitions first and regenerate derived artifacts with the owning generator; do not hand-edit generated output.

## Fix Bugs

For simple, localized, low-risk fixes with an obvious cause and expected behavior, implement the fix directly. A user-provided reproduction or explicit instruction to fix the issue is sufficient confirmation.

For complex, ambiguous, risky, regression-prone, or broad-impact bugs, establish the intended behavior and cause before editing. If expected behavior remains ambiguous, report the uncertainty instead of guessing.

## Test and Verify

- Inspect existing coverage before adding a test. Add the smallest test at a stable seam only when meaningful behavior or material regression risk is otherwise unprotected. Default to one focused test; add more only for distinct important behaviors or failure modes.
- Do not add tests for trivial wiring, implementation details, framework or vendor behavior, derive or default behavior, behavior already covered, coverage numbers, or simple low-risk fixes.
- Fake external boundaries. When no useful test seam exists, use proportionate verification instead of forcing a low-value test.
- When CI exists, treat it as the default test and compilation runner. Run tests locally only to reproduce a complex bug, guide implementation, diagnose a CI failure, or satisfy an explicit request; then run only the smallest focused test once per cohesive batch.
- Run required formatters, generators, and artifact checks locally. Report exact commands, warnings, and any unverified gaps.
