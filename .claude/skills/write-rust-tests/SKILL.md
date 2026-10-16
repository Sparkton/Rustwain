---
name: write-rust-tests
description: Rust-specific unit and integration test conventions for this Cargo workspace. Use when adding, modifying, reviewing, or refactoring Rust tests, or reproducing a Rust bug with regression coverage.
---

# Writing Rust Tests

This skill defines Rust-specific unit-test conventions. It extends `write-code` and `write-rust-code`, so their shared workflow and Rust patterns apply inside tests too.

## Test Data: Prefer `fake`

Use the `fake` crate to generate test data instead of hand-crafting structs with hardcoded values. This keeps tests focused on the behavior under test, not on irrelevant field values.

### Pattern: struct-update with `..Faker.fake()`

Use a **struct-update literal** with `..Faker.fake()` as the base. Set only the fields the test actually cares about; let `Faker` fill the rest. This form is concise, avoids a `mut` binding, and makes every meaningful override visible at the construction site. It also composes naturally for nested structs — apply `..Faker.fake()` at each level where only a few fields matter.

```rust
// GOOD: only the fields the test asserts on are specified
let task = Task {
    phase: TaskPhase::Pending,
    priority: 10,
    owner: Node {
        name: "node-a".to_owned(),
        ..Faker.fake()
    },
    ..Faker.fake()
};

// BAD: manual struct with irrelevant hardcoded values
let node = Node {
    name: "test-node".to_owned(),
    region: Some("test-region".to_owned()),
    labels: BTreeMap::new(),
    status: NodeStatus::Ready,
    ..Default::default()
};
```

### What to override

Override fields that the test **asserts on** or that are **required for the scenario to function correctly** (e.g. a specific DB key, a domain-significant value the pipeline reads). Do **not** override fields just because the Dummy impl happens to generate a different value — the Dummy impl can change, and compensating for it leaks those implementation details into tests.

```rust
// GOOD: override only the key (asserted on) and the limit (functionally required)
let quota = Quota {
    id: Quota::key(&tenant_id, resource),
    limit: 8, // asserted: plan.rejected[0].limit == 8
    ..Faker.fake()
};

// BAD: overriding fields just because Dummy generates "wrong" defaults
let quota = Quota {
    id: Quota::key(&tenant_id, resource),
    limit: 8,
    history: vec![],    // ← not needed, Dummy already generates this
    reservations: vec![], // ← not needed
    ..Faker.fake()
};
```

### When `fake` works

- The type implements `fake::Dummy<fake::Faker>` (derive `fake::Dummy` or search for existing `impl fake::Dummy` blocks).
- You need an instance where most field values are irrelevant to the test.
- Override only fields the test asserts on or that are directly required for correctness.

### When `fake` doesn't apply

- The test asserts on a majority of specific field values that must be deterministic (e.g. a specific ID used for consistent hashing).
- The type doesn't have a `Dummy` impl and adding one isn't worthwhile (e.g. simple enums, one-off config structs).
- External DTOs deserialized from JSON fixtures.

### Adding new `Dummy` impls

When a type is frequently constructed in tests, prefer `#[derive(fake::Dummy)]`; write a manual impl only when fields need constrained values:

```rust
impl fake::Dummy<fake::Faker> for MyType {
    fn dummy_with_rng<R: fake::Rng + ?Sized>(_: &fake::Faker, rng: &mut R) -> Self {
        Self {
            name: fake::faker::lorem::en::Word().fake_with_rng(rng),
            replicas: (1..=5).fake_with_rng(rng),
            ..Default::default()
        }
    }
}
```

## Test granularity

- For coverage justified by `write-code`, prefer a unit test with asserts when there is one deterministic expected outcome.
- Prefer one compact three-step test when possible: setup data, perform a calculation or mutation, confirm the outcome with assertions.
- Prefer BDD tests if there are multiple valid outcomes, e.g. scheduling decisions, reconciliation loops, or leader election.
- Prefer BDD tests if there are multiple steps and outcomes that need to be done sequentially, resulting in a specific outcome
- Prefer BDD test when scenarios are complex multi-step sequences, and it's hard to distinguish between the important logical expectations checking from the test boilerplate code

### Unit tests with asserts

- Asserts should have the actual variable or result of the tests on the first left parameter, and the expected value on the second right parameter
- If the assertion comparison has low complexity do not add a custom failure message, and leave expressions inline, specially when the failed `assert` default message and the custom message will say the same thing, for example:

```rust
assert!(queue.is_empty());
assert!(cache.get(&key).is_none());
assert_eq!(node.status(), NodeStatus::Ready);
```

- If the assertion comparison is complex or results in multiple split lines of code, prefer moving the values of the comparison to informative named variables and compare those, also add an informative message, for example:

  ```rust
  let placement = scheduler.place(&task_requesting_more_than_any_node);
  assert!(placement.is_none(), "Oversized task should not be placed");
  ```

### BDD tests

- Create a scenario using the workspace's Given-When-Then macros (in the shared testing crate). They are standard BDD constructs that expand to plain Rust. If they don't exist yet, add them there rather than per crate. General approach:
  - Construct a state with a new world and `given*!` macros
  - Make calculations and mutations to that state with `given*!` macros
  - Check expectations with `then*!` macros
- Use existing BDD tests in the workspace as the style reference.
- Invoke BDD macros with braces (`given! { ... }`) instead of parentheses (`given!(...)`). Rustfmt leaves brace-delimited macro bodies unchanged, preserving the intentional spacing that makes each action phrase easier to scan; format their contents carefully.
- Simple checks should be in the style of `then*!`
- BDD tests should not contain any standalone `assert` statements, first try to implement them as `then*!` expressions, and for very complex checks and expressions those should be moved to an isolated `fn` in the world implementation
- Note the macros make liberal use of `unwrap` and `expect` for code simplicity and conciseness
- All test state and state manipulation should live in the world object struct, to encapsulate test implementation into a specific place, away from pure testing logic
- Leverage `fake` and follow the rest of testing guidance (except usage of asserts)
- Example:

```rust
let mut w = SchedulerWorld::new().await;
scenario! {w, "Pending task is bound to the only ready node"};
given! {w, "a fake", ready_node};
given! {w, "a fake", pending_task};
given_async! {w, "we", start_the_scheduler};

when_async! {w, "we", run_one_scheduling_cycle};
then! {w, "the", task_phase == TaskPhase::Scheduled};
then_fn! {w, "and the", task_is_bound_to_ready_node};
```

- Avoid mutating or interrogating the state outside the world fields and methods, that is what the world methods and attributes are there for


## Test Structure

### Placement

- Unit tests live in a `#[cfg(test)] mod tests` block at the bottom of the source file.
- Shared test helpers live in the workspace's shared testing crate, or in the crate that owns what they build, behind its `test-support` feature (see below).

### Test helpers for other crates

A crate's `#[cfg(test)]` items are visible only to its own tests. To share a helper with other crates' tests, put it behind the crate's `test-support` feature:

- Name the feature `test-support` in every crate, not `testing`, `test-utils` or another name.
- Gate the helper with `#[cfg(any(test, feature = "test-support"))]`, so the crate's own tests see it too.
- Turn the feature on only from `[dev-dependencies]`, such as `store = { path = "../store", features = ["test-support"] }`, or from another crate's `test-support`. A normal dependency or a production feature never turns it on, so it never reaches a production binary.
- Keep production code out of it. Code under `test-support` never ships.

### Tests requiring storage or external services

Use one shared in-memory or fake helper per backend (exposed via `test-support`) instead of duplicating setup functions in every test module.

### Constructors in tests

Use type constructors (`::new()` or named constructors) instead of struct literals — same rule as production code:

```rust
// GOOD
let mut spec = TaskSpec::new("web", image);
spec.replicas = 3;

// BAD
let spec = TaskSpec {
    id: Some(task_id.clone()),
    name: "web".to_owned(),
    image,
    replicas: 3,
    created_at: Utc::now(),
    updated_at: Utc::now(),
};
```

### Unwraps in tests

- For conciseness, it is ok to do direct unwraps and not expects if any failure will signal an obvious problem, for instance this is perfectly valid:

```rust
let store = Store::in_memory().await.unwrap();
```

### Naming

- Test function names describe the scenario: `scheduler_prefers_node_with_most_free_memory`
- Use `snake_case`, no `test_` prefix (the `#[test]` attribute is sufficient).

### Test comments

- Only add a short comment above a new or substantially rewritten test when the intent is not obvious from the test name and assertions.
- The comment should explain what functionality is being verified and what outcome is expected.
- Keep comments to one concise sentence when possible, for example:

```rust
// Verifies tainted nodes are skipped even when they have the most free capacity.
#[test]
fn scheduler_skips_tainted_node_with_most_capacity() {
    // ...
}
```

## Running tests

Follow `write-code` to decide whether local execution is justified. When it is, always pass `-q` when running `cargo test` manually. Default output prints every test name plus per-target build banners — orders of magnitude more tokens than the pass/fail summary an agent actually needs. `-q` keeps failure output (it's only the running-test banner that gets suppressed).

```bash
cargo test -p <crate> --lib -q
cargo test -p <crate> -q -- <test_name>
```

## Verification

- [ ] Made a conscious decision on which test approach to take
- [ ] Kept assert-style and BDD-style tests consistent within a module, not mixed
- [ ] Used `fake` for data generation where applicable
- [ ] No duplicated storage/fake setup helpers
- [ ] Constructors used instead of struct literals
- [ ] Tests are independent and don't rely on shared mutable state
- [ ] Followed `write-code` for test authorship and execution decisions
