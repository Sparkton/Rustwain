---
name: write-rust-code
description: Rust-specific design, implementation, and verification conventions for this Cargo workspace. Use whenever designing, writing, modifying, or reviewing Rust production code, together with the shared write-code skill.
---

# Writing Rust Code

Apply the shared `write-code` skill first. This skill adds Rust-specific conventions and verification. For Rust test work, also follow `write-rust-tests`. Treat the Rust entries in `docs/anti_patterns.md` as hard constraints.

## Design in Rust

Design the solution in Rust's terms from the start; do not sketch it in another language's idioms and translate.

- **Model with types first.** Start from enums, structs, and traits that make invalid states unrepresentable. Encode lifecycle states as enum variants or typestate, not as strings, flags, or optional fields that are "only set when".
- **Decide ownership up front.** For every piece of state, name its single owner and how others reach it: borrow, move, message, or shared handle. Reach for `Arc<Mutex<_>>` only after ruling out ownership by one task plus message passing.
- **Errors are values.** Every fallible operation returns `Result` with a typed error that callers can match on. Panics are for broken invariants only.
- **Traits for capabilities, not hierarchies.** Use small traits and generics for behavior that varies; use enums for a closed set of variants. Never emulate inheritance.
- **Static over dynamic.** Prefer generics and `enum` dispatch; use `dyn Trait` only for genuinely open, runtime-chosen sets. Never use `Any`/downcasting to route behavior.
- **Iterators over index loops.** Express transformations with iterator adapters; index only when the algorithm genuinely needs positions.
- **Zero-cost by default.** Borrow instead of cloning, avoid needless allocation in hot paths, and make a clone a deliberate decision rather than a way to quiet the borrow checker.

## Separate Logic from I/O

- Keep decision logic (state machines, planning, reconciliation, validation) as pure, synchronous code over typed inputs and outputs, with no network, storage, clock, or process access.
- Put I/O in thin adapters behind small traits, injected into the logic's callers. This keeps core logic deterministic and testable without mocks of the world.
- Crate and layer boundaries are owned by the project's architecture docs. Read them before adding a crate or moving code between layers. Where no boundary is defined yet, propose one instead of inventing it silently.

## Workspace Conventions

- Add new dependencies to `[workspace.dependencies]` in the root `Cargo.toml` first (check it isn't already there), then use `dep = { workspace = true }` in the crate. Justify every new dependency; prefer the standard library when it suffices.
- Use typed error enums (`thiserror`) in library crates; reserve opaque errors (`anyhow`) for binaries and top-level glue.
- Use `tracing` for logging, never `println!`/`eprintln!` outside CLI user output. Prefer structured fields (`info!(key = value, "msg")`); short values central to the message, such as a count or primary ID, may be inlined.
- Use `From`/`TryFrom` for numeric conversions instead of `as` casts.
- Use `const` for constants and environment variable names.
- Model serde payloads as structs and enums, never as maps or `serde_json::Value` when the shape is known.
- Prefer derive macros (for example `clap` derive) over builder APIs when available.
- Take parameters owned when the function consumes them and let the caller decide to clone or move. Exceptions: `impl Into<String>`, `impl AsRef<str>`, `&str` for caller flexibility, or trivially cheap `Copy` values.
- Take `&` references when ownership is not needed.
- Create an owned `String` from `&str` with `to_owned()`.
- Write `unsafe` only with a `// SAFETY:` comment that states the invariant upheld, and keep it in the smallest possible module.

### Search before declaring a type

Before declaring a new enum or struct, search the workspace for its **variant or field names** in their exact declaration casing (for example `rg -w 'Pending|Running|Succeeded'`), not just the type name. Duplicates rarely collide by name; they collide on their variant set. When an existing type has the same meaning, extend it instead of declaring a parallel one. Wire or protocol types may model the same concept when their serialization differs; map them to the canonical type at the boundary.

## Patterns

- **Use constructors** — Prefer `new()` or named constructors over struct literals. Encapsulate defaults (timestamps, generated IDs, initial state) inside the type.
- **Keep data typed until the boundary** — Pass typed values through the call chain and serialize only at the wire or storage boundary.
- **Concurrency** — Prefer one owning task per piece of mutable state, driven by channels. Never hold a `std::sync` lock across `.await`. Make cancellation and shutdown explicit (`CancellationToken`, `select!`), and bound every queue and channel.
- **Embedded non-Rust assets** — Load them with `include_str!()`/`include_bytes!()` from separate files; inline single-use query strings at the call site and promote them to a `const` when reused.

## Verification

Follow `write-code` for test authorship and execution decisions. This section covers Rust-specific checks only.

### Always (no commands needed)
- [ ] The design follows "Design in Rust" and the Rust anti-patterns
- [ ] New dependencies added to `[workspace.dependencies]` first

### During Iteration
- Prefer `cargo check -p <crate>` scoped to the changed crate over whole-workspace builds.
- Do not use `cargo check` as a substitute for a test selected under `write-code`.

### Before Pushing
- Run `cargo fmt --all` and include files it reformats.
- Run `cargo clippy -p <crate> --all-targets -- -D warnings` for changed crates and fix new warnings.
- Do not push when the selected validation reports an error.
