# Agent Policy

Rustwain is a ground-up re-imagining of Kubernetes in Rust. The goal is a cross-node cluster management system with measurable improvements over Kubernetes, built bottom-up: design the foundations first, then the components that depend on them.

## Design Before Code

- Do not implement a component, protocol, or persisted format without an `Accepted` design doc in `docs/design/`. When none exists, write or update one first.
- Only the user accepts or rejects a design doc.
- When a ticket conflicts with its design doc, stop and raise it; do not silently diverge.

## Rust Only

- All code and tooling is Rust. Automation lives in `cargo xtask` or workspace binaries, never in Python, Node, or non-trivial shell scripts.
- No frontend, UI, or WASM targets.
- Think in Rust from the start: types, ownership, and `Result`-based errors, not idioms translated from other languages.
- Treat `docs/anti_patterns.md` as hard constraints.

## Delivering Code

- Every code change goes through the 3-1 workflow: three parallel Sonnet implementers managed by one Opus orchestrator that writes no logic itself.
- Keep each PR to one ticket with explicit acceptance criteria.
- Do not commit, push, or open PRs without the user's go-ahead.
- Never add a `Co-Authored-By: Claude ...` trailer (or any AI co-author or attribution line) to commit messages or PR descriptions.

## Agent Instructions

- Project skills live in `.agent/skills` (canonical) and are mirrored to `.claude/skills`. Edit only the canonical copy, then re-sync.
- Keep this file short and durable; procedures belong in skills.

## Safety

- Never hardcode, log, or commit secrets or credentials.
- Run database tooling against local instances only.
- Ask before destructive or hard-to-reverse operations (deleting data, rewriting history, force-pushing).
