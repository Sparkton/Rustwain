---
name: write-design-doc
description: Write, revise, and review design documents in docs/design for system components before they are implemented. Use whenever proposing a new component, subsystem, protocol, resource model, or cross-component change; when comparing an approach with how Kubernetes solves the same problem; or when a ticket lacks an accepted design to implement against.
---

# Writing Design Documents

Design docs decide **what to build and why** before any code exists. They are the contract the `orchestrate-code-changes` workflow implements against. Documentation of code that already exists belongs to `write-project-docs`, not here.

## When to Write One

- A new component, crate boundary, protocol, persisted format, or resource type.
- A change that crosses components or alters an accepted design's decisions.
- Any choice where Kubernetes has an established behavior we may keep, improve on, or drop.

Small changes inside one component's accepted design do not need a new doc; amend the existing doc's Decision Log instead.

## Build Bottom-Up

Design foundational layers before the layers that depend on them (for example: process and node primitives → node agent → state store and consensus → scheduling and control loops → cluster API). Each doc lists the docs it depends on in its header. Do not accept a doc whose dependencies are still `Draft`; resolve them first or record the assumption under Open Questions.

## Location and Naming

- Path: `docs/design/NNNN-short-slug.md`, with `NNNN` the next free four-digit number. Numbers are never reused.
- Add each doc to the index in `docs/design/README.md` (number, title, status, depends on).
- Status lifecycle: `Draft` → `In Review` → `Accepted` → (`Superseded by NNNN` | `Rejected`). Only the user moves a doc to `Accepted` or `Rejected`.

## Research Phase (Before Writing)

1. Read `docs/design/README.md` and every doc this one depends on or affects.
2. Research how Kubernetes handles the problem: the components involved, the API objects, the real failure modes, and known pain points (scalability limits, operational complexity, issue-tracker history). Cite sources (KEPs, upstream docs, source paths at a pinned version). Mark anything unverified as such.
3. Look at other relevant prior art (Nomad, Borg/Omega papers, systemd, containerd, Raft implementations) where it informs the decision.
4. Collect concrete numbers where possible: target cluster size, object counts, request rates, latency and recovery budgets.

## Template

```markdown
# NNNN: <Title>

| Field | Value |
|-------|-------|
| Status | Draft |
| Author | <name> |
| Created | YYYY-MM-DD |
| Depends on | NNNN, NNNN (or none) |

## 1. Summary
Two to four sentences: the problem and the proposed decision.

## 2. Motivation
Why this is needed now, and what breaks or is missing without it.

## 3. Kubernetes Today
How Kubernetes solves this, with citations. What works well (keep), what hurts (improve), what is unnecessary (drop).

## 4. Goals and Non-Goals
Bulleted, testable goals. Explicit non-goals to bound scope.

## 5. Proposal
### 5.1 Model
Core Rust types and traits as signatures only — enums for states, structs for resources, traits for capabilities. No function bodies.
### 5.2 Ownership and Concurrency
Who owns each piece of state, which tasks exist, how they communicate (channels, messages), and where backpressure applies.
### 5.3 Interfaces
Public APIs, wire protocols, and persisted formats, including versioning.
### 5.4 Behavior
Normal flow step by step, as a numbered sequence.

## 6. Failure Modes and Recovery
Table: Failure | Detection | Effect | Recovery. Cover crashes, partitions, slow or lost messages, restarts mid-operation, and resource exhaustion.

## 7. Scalability and Performance
Expected limits and the measurement that will confirm them. Compare with Kubernetes where numbers exist.

## 8. Security
Trust boundaries, authentication, authorization, and secret handling.

## 9. Alternatives Considered
Each alternative with why it was not chosen.

## 10. Testing and Verification Strategy
Which properties are proven by unit tests, property tests, deterministic simulation, or benchmarks.

## 11. Implementation Plan
Ordered tickets, each sized for one 3-1 delivery cell, with the acceptance criteria each will carry.

## 12. Open Questions
Unresolved decisions, each with an owner or the doc that will resolve it.

## 13. Decision Log
Dated entries for material changes after `Accepted`.
```

Omit a section only when it truly does not apply, and say so in one line rather than deleting the heading.

## Writing Rules

- **Think in Rust.** Express the model as Rust types and ownership, not as classes, dictionaries, or pseudo-Python. Follow `write-rust-code` "Design in Rust" and the Rust entries in `docs/anti_patterns.md`.
- **Decide, don't survey.** Every section ends in a decision or an explicit open question.
- **Make improvements measurable.** A claimed improvement over Kubernetes states the metric, the expected value, and how it will be measured.
- **Keep scope Rust-only.** No frontend, WASM, or non-Rust tooling in proposals.
- **Be honest about uncertainty.** Separate verified facts (cited) from assumptions (labeled).
- Plain Markdown, numbered top-level sections, tables for structured data. Mermaid diagrams are allowed for component and sequence views when they clarify more than prose.

## Review Checklist

- [ ] Dependencies are listed and are `Accepted` (or the assumptions are recorded)
- [ ] Kubernetes behavior is cited, with keep/improve/drop stated
- [ ] Every state is an enum variant, and every piece of state has one owner
- [ ] Failure-mode table covers crash, partition, restart, and exhaustion
- [ ] Improvements over Kubernetes are measurable
- [ ] Implementation plan tickets are sized for one 3-1 cell, with acceptance criteria
- [ ] The doc is listed in `docs/design/README.md`
