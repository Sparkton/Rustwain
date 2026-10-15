---
name: surrealql-syntax
description: SurrealQL language reference for writing and debugging SurrealDB queries. Use when writing raw SurrealQL, building query strings in Rust, or debugging query syntax errors.
---

# SurrealQL Syntax Reference

Quick reference for SurrealQL. Examples use placeholder tables only — `item`, `bucket`, a `link` graph edge (`in` = item, `out` = item), and a `log` table with composite ids. They do not describe any real schema; substitute the project's own tables.

> **Maintenance:** When you hit a SurrealQL syntax error or use a pattern or function not documented here, add the correct syntax and an example before moving on. Note the SurrealDB version when a rule is version-specific.

## Load Supporting Reference

Read [references/functions-schema-and-administration.md](references/functions-schema-and-administration.md) when working with built-in functions, closures, schema definitions, indexes, permissions, events, transactions, metadata statements, durations, or type declarations.

## Statements

When piping a multi-line statement into `surreal sql`, pass `--multi`. Without it, each newline is submitted separately even before a semicolon.

### SELECT

```sql
SELECT [VALUE] fields | *
  [OMIT field, ...]
  FROM [ONLY] target
  [WITH INDEX index_name]
  [WHERE condition]
  [SPLIT field]
  [GROUP BY field | GROUP ALL]
  [ORDER BY field [ASC|DESC]]
  [START offset]
  [LIMIT count]
  [FETCH field]
  [TIMEOUT duration]
  [EXPLAIN [FULL]]
;
```

Key modifiers:
- `VALUE` — returns raw values instead of `{ field: value }` objects
- `ONLY` — expects exactly one record; errors if multiple
- `OMIT` — exclude fields when using `*`
- `FETCH` — resolve record links inline
- `SPLIT` — Cartesian item on array fields

```sql
SELECT * FROM item WHERE status = 'open' ORDER BY created_at DESC LIMIT 10;
SELECT VALUE name FROM item WHERE bucket = bucket:⟨b1⟩;
SELECT count() AS total, status FROM item GROUP BY status;
SELECT count() FROM item GROUP ALL;
SELECT *, math::abs(price - $target) AS distance
FROM item
ORDER BY distance ASC, updated_at DESC
LIMIT 5;
```

Observed parser and planner pitfalls (verify against the pinned SurrealDB version):

- **Order by an alias, not an expression.** `ORDER BY math::abs(...)` can fail to parse even though the function is valid in the `SELECT` list. Select it into an alias and item by that. The `ORDER BY` field must be in the selection, or the parser reports `Missing item idiom`.
- **Keep range predicates indexable.** Compare the indexed field directly against a precomputed bound; `created_at + 6h > time::now()` is not indexable. `status IN [NONE, 'active']` can keep a range-index plan where `(status IS NONE OR status = 'active')` falls back to a table scan.
- **Order and limit forced-index scans in an outer query.** With `WITH INDEX`, `LIMIT` has been observed applying before a descending `ORDER BY`, returning an older slice.

```sql
SELECT * FROM (
  SELECT *
  FROM item WITH INDEX idx_item_bucket_created
  WHERE bucket = $bucket AND created_at < $before
)
ORDER BY created_at DESC
LIMIT $limit;
```

For fields inside arrays of objects, project through the array with a wildcard: `children.*.id` returns the `id` of each object; `children.id` may not.

```sql
SELECT children.*.id AS child_ids FROM item:1234;
```

With `SPLIT`, the exact split idiom must be in the selection. Project the whole element in an inner query, then select nested fields outside; otherwise the parser reports `Missing split idiom`.

```sql
SELECT endpoints.port AS port
FROM (
  SELECT endpoints FROM item SPLIT endpoints
);
```

### CREATE

```sql
CREATE [ONLY] target
  [CONTENT { ... } | SET field = value, ...]
  [RETURN NONE | BEFORE | AFTER | DIFF | VALUE field]
  [TIMEOUT duration]
;
```

```sql
CREATE item:⟨abc123⟩ CONTENT { name: 'alpha', bucket: bucket:⟨b1⟩ };
CREATE item SET name = 'alpha', bucket = bucket:⟨b1⟩;
```

### UPDATE

```sql
UPDATE [ONLY] target
  [WITH INDEX index_name]
  [SET field = value | MERGE { ... } | PATCH [...] | CONTENT { ... } | UNSET field]
  [WHERE condition]
  [RETURN NONE | BEFORE | AFTER | DIFF]
  [TIMEOUT duration]
;
```

```sql
UPDATE item:⟨abc123⟩ SET name = 'beta';
UPDATE item:⟨abc123⟩ MERGE { meta: { note: 'x' } };
UPDATE item SET status = 'expired' WHERE created_at < time::now() - 30d;
```

Use one mutation clause per `UPDATE`. Combining `SET` and `UNSET` in one statement has failed to parse; run separate statements.

```sql
UPDATE item SET name = 'beta';
UPDATE item UNSET legacy_name;
```

### UPSERT

Create-or-update: tries `CREATE`, falls back to `UPDATE` if the record exists.

```sql
UPSERT [ONLY] target
  [CONTENT { ... } | SET field = value | MERGE { ... }]
  [WHERE condition]
  [RETURN NONE | BEFORE | AFTER | DIFF]
  [TIMEOUT duration]
;
```

```sql
UPSERT item:⟨abc123⟩ CONTENT { name: 'alpha', bucket: bucket:⟨b1⟩ };
```

### INSERT

```sql
INSERT [IGNORE] INTO table
  (fields) VALUES (values), (values), ...
  [ON DUPLICATE KEY UPDATE field = value, ...]
;
-- OR with an object:
INSERT INTO table { ... };
```

```sql
INSERT INTO item (id, parent, status) VALUES ('i1', item:⟨a⟩, 'open'), ('i2', item:⟨b⟩, 'paid');
INSERT INTO item { id: item:⟨abc⟩, name: 'alpha' } ON DUPLICATE KEY UPDATE name = $input.name;
```

Object-form `INSERT` has accepted only one object per statement on some versions. Issue separate `INSERT IGNORE` statements when records need different shapes.

### DELETE

```sql
DELETE [ONLY] target
  [WITH INDEX index_name]
  [WHERE condition]
  [RETURN NONE | BEFORE | AFTER | DIFF]
  [TIMEOUT duration]
;
```

```sql
DELETE FROM link WHERE out = NONE;
DELETE item:⟨abc123⟩;
-- Several explicit records: target the table and filter by id.
-- `DELETE [item:one, item:two]` is not valid.
DELETE FROM item WHERE id IN [item:one, item:two];
```

### RELATE (graph edges)

```sql
RELATE [ONLY] source -> edge_table -> target
  [CONTENT { ... } | SET field = value, ...]
  [RETURN NONE | BEFORE | AFTER | DIFF]
  [TIMEOUT duration]
;
```

```sql
RELATE item:⟨abc⟩ -> link -> item:⟨i2⟩ SET weight = 2, at = time::now();
```

---

## Record IDs

Format: `table:key`

| Style | Example |
|-------|---------|
| String key | `item:⟨abc-123⟩` |
| Integer key | `item:42` |
| Random | `CREATE item;` (auto-generated) |
| `rand()` | `CREATE item:rand();` |
| `ulid()` | `CREATE item:ulid();` |
| `uuid()` | `CREATE item:uuid();` |
| Array (composite) key | `log:[$item, $at]` |
| Object key | `CREATE stat:{ region: 'eu', year: 2024 };` |

Construct dynamically: `type::thing('item', 'abc-123')` → `item:⟨abc-123⟩`

Extract parts:
- `meta::id(record_link)` or `record::id(record_link)` — the key
- `meta::tb(record_link)` or `record::tb(record_link)` — the table name

> [!WARNING]
> Composite record IDs use `[]`, not `()`. `log:(item:abc, 'created')` is a parse error — use `log:[item:abc, 'created']`.

### Record Ranges

```sql
-- Simple range
SELECT * FROM item:1..100;

-- Composite-key prefix range: all log rows for one item
SELECT * FROM log:[$item, NONE]..[$item, time::now()];

-- Inclusive end with =
SELECT * FROM log:[$item, $start]..=[$item, $end];
```

Prefer record-id range selects when records share an array-id prefix. They are usually faster and clearer than filtering on `id[0] = ...`.

```sql
-- Good: scan the shared prefix directly
SELECT VALUE id FROM log:[$item, NONE]..=[$item, ..];

-- Usually worse when a prefix range is available
SELECT VALUE id FROM log WHERE id[0] = $item;
```

Avoid reverse record-id scans to fetch the last row in a range. Some storage backends (including the in-memory test backend) fail `ORDER BY id DESC` over a range with `The underlying datastore does not support reversed scans`. Use the prefix range plus `ORDER BY <timestamp> DESC LIMIT 1` instead.

---

## Graph Traversal

```sql
-- Outbound: follow ->
SELECT ->link->item FROM item:⟨abc⟩;

-- Inbound: follow <-
SELECT <-link<-item FROM item:⟨i2⟩;

-- Bidirectional
SELECT <->link<->item FROM item:⟨abc⟩;

-- Multi-hop
SELECT ->link->item->link->item FROM item:⟨abc⟩;

-- Filter edges inline
SELECT ->(link WHERE weight > 2)->item FROM item:⟨abc⟩;

-- Items reached through items that link to this one
SELECT <-link<-item->link->item FROM item:⟨i2⟩;
```

---

## Type Casting

```sql
<int> "42"          -- string → int
<float> 13456       -- int → float
<string> 42         -- int → string
<datetime> "2024-03-15"
<bool> "true"
<decimal> 3.14
<duration> "1h30m"
<array<string, 5>> ["a","b","c","d","e"]
```

When comparing a linked-record datetime with a bound parameter, cast both sides; casting only the field can compare unexpectedly.

```sql
WHERE <datetime>item.created_at >= <datetime>$start
  AND <datetime>item.created_at <= <datetime>$end
```

---

## Operators

| Operator | Meaning |
|----------|---------|
| `=`, `!=` | Equality |
| `<`, `>`, `<=`, `>=` | Comparison |
| `AND`, `OR`, `NOT` | Logical |
| `??` | Null coalescing (right side if left is `NULL`/`NONE`) |
| `?:` | Elvis (right side if left is falsy) |
| `CONTAINS` / `∋` | Value contains item |
| `CONTAINSALL` | Contains all items |
| `CONTAINSANY` | Contains at least one |
| `CONTAINSNONE` | Contains none |
| `INSIDE` / `∈` | Item is inside value |
| `IS`, `IS NOT` | Type checking |
| `~` | Fuzzy match |
| `@@` | Full-text search match |

---

## Control Flow

```sql
IF $score > 90 { 'A' } ELSE IF $score > 80 { 'B' } ELSE { 'C' };
THROW 'Something went wrong';
RETURN $result;

LET $open = (SELECT * FROM item WHERE status = 'open');
SELECT * FROM $open WHERE total > 100;
```

---

## Subqueries

```sql
-- In FROM
SELECT * FROM (SELECT * FROM item WHERE active = true) WHERE bucket = bucket:⟨b1⟩;

-- Inline in SELECT fields
SELECT *, (SELECT VALUE count() FROM ->link GROUP ALL) AS links FROM item;

-- In WHERE
SELECT * FROM item WHERE id INSIDE (SELECT VALUE in FROM link WHERE out = item:⟨i2⟩);

-- With LET
LET $active = (SELECT VALUE id FROM item WHERE active = true);
SELECT * FROM link WHERE in INSIDE $active;
```

---

## Aggregate Functions

Used with `GROUP BY field` or `GROUP ALL`:

| Function | Description |
|----------|-------------|
| `count()` | Count records |
| `math::sum(field)` | Sum |
| `math::mean(field)` | Average |
| `math::min(field)` | Minimum |
| `math::max(field)` | Maximum |
| `math::median(field)` | Median |
| `math::stddev(field)` | Standard deviation |
| `math::variance(field)` | Variance |
| `array::group(field)` | Collect into array |

```sql
SELECT status, count() AS items, math::mean(total) AS avg_total FROM item GROUP BY status;
SELECT count() AS total FROM item GROUP ALL;
```

> [!WARNING]
> **`ORDER BY` on aliased aggregates is ignored with `GROUP BY`.** Results sort by the bucket field. Wrap in a subquery: `SELECT * FROM (SELECT x, count() AS cnt FROM t GROUP BY x) ORDER BY cnt DESC;`

> [!WARNING]
> **No standalone `min()`/`max()`.** Use `math::min()`/`math::max()`; they work only on numeric fields in aggregate context and return `NONE` on strings.

> [!WARNING]
> **`array::group()` is aggregate-only.** Outside `GROUP BY`/`GROUP ALL` it errors with "expected a array but found ...".
