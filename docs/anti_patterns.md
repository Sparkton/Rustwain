# Anti-Patterns

Patterns agents must not introduce. Each entry is a constraint when modifying the relevant code. Add or amend entries with the `log-anti-pattern` skill.

## Project Scope

### Non-Rust code or tooling
- **Don't:** add Python, shell-heavy, Node, or other non-Rust scripts, build steps, or helper tools.
- **Do:** write automation as `cargo xtask` subcommands or Rust binaries in the workspace.
- **Source:** project policy

### Frontend or WASM targets
- **Don't:** add web frontends, UI crates, or `wasm32` targets and dependencies.
- **Do:** expose functionality through Rust APIs, CLIs, and wire protocols only.
- **Source:** project policy

## Rust, Not Python

### Stringly-typed data
- **Don't:** pass `HashMap<String, Value>`, `serde_json::Value`, or magic strings for data with a known shape.
- **Do:** model it as structs and enums; parse once at the boundary (see `write-rust-code`).
- **Source:** project policy

### Strings and flags as state
- **Don't:** track lifecycle with `status: String`, bool flags, or `Option` fields that are "only set when" some state applies.
- **Do:** use an enum whose variants carry their own data, or typestate.
- **Source:** project policy

### Exceptions as control flow
- **Don't:** `unwrap()`/`expect()`/`panic!` on recoverable failures, or `Box<dyn Error>`/`String` errors in library crates.
- **Do:** return `Result` with a typed `thiserror` enum; panic only on broken invariants.
- **Source:** project policy

### Duck typing and runtime type checks
- **Don't:** route behavior with `dyn Any` downcasting, type-name strings, or `match` on a "kind" field.
- **Do:** use enums for closed sets and traits plus generics for open behavior.
- **Source:** project policy

### Inheritance emulation
- **Don't:** build class hierarchies with `Deref` to a "base" struct, or traits that mirror base classes.
- **Do:** compose structs and implement small capability traits.
- **Source:** project policy

### Shared mutable object graphs
- **Don't:** default to `Rc<RefCell<_>>`/`Arc<Mutex<_>>` webs so any object can mutate any other.
- **Do:** give state one owner, pass borrows or messages, and use arenas or IDs for graphs.
- **Source:** project policy

### Clone to appease the borrow checker
- **Don't:** sprinkle `.clone()` or `.to_string()` until it compiles.
- **Do:** restructure ownership or borrow scopes; clone only as a deliberate, cheap, or required copy.
- **Source:** project policy

### Index-based loops
- **Don't:** write `for i in 0..v.len()` with `v[i]` when no position logic is needed.
- **Do:** use iterator adapters (`iter`, `map`, `filter`, `zip`, `enumerate`, `windows`).
- **Source:** project policy

### Kwargs-style configuration
- **Don't:** mimic keyword args with structs of all-`Option` fields resolved at runtime, or with dynamic config dicts.
- **Do:** use required fields for required data, `Default` plus struct-update syntax, or a typed builder.
- **Source:** project policy

### Getters and setters for everything
- **Don't:** add trivial `get_x`/`set_x` pairs on every field.
- **Do:** expose fields that carry no invariant; use methods only where an invariant is enforced.
- **Source:** project policy

### Grab-bag utility modules
- **Don't:** create `utils.rs`, `helpers.rs`, or `common` crates that collect unrelated functions.
- **Do:** put each function next to the type or module that owns its concept.
- **Source:** project policy
