# SurrealQL Functions, Schema, and Administration

## Common Functions

### String

`string::concat(a, b)`, `string::contains(s, sub)`, `string::lowercase(s)`, `string::uppercase(s)`, `string::trim(s)`, `string::split(s, sep)`, `string::reverse(s)`, `string::len(s)`, `string::starts_with(s, prefix)`, `string::ends_with(s, suffix)`, `string::replace(s, old, new)`, `string::slug(s)`, `string::is::email(s)`, `string::is::numeric(s)`

> [!WARNING]
> **No `parse_json()` or `keys()` on strings.** If a field stores JSON as a string, use `string::split()` to extract values: `string::split(string::split(payload, '"type_":"')[1], '"')[0]`

### Array

**Mutate / build:**
`array::push(a,v)`, `array::append(a,v)`, `array::prepend(a,v)`, `array::pop(a)`, `array::insert(a,v,i)`, `array::remove(a,i)` *(by index, not value)*, `array::fill(a,v,start,len)`, `array::repeat(v,n)`, `array::range(s,e)`, `array::sequence(s,e)`

**Set operations (two arrays):**
`array::complement(a,b)` *(in a NOT in b)*, `array::difference(a,b)` *(symmetric diff)*, `array::intersect(a,b)`, `array::union(a,b)`, `array::combine(a,b)`, `array::concat(a,b)`

**Query / inspect:**
`array::len(a)`, `array::first(a)`, `array::last(a)`, `array::at(a,i)`, `array::find(a,v)`, `array::find_index(a,v)`, `array::filter_index(a,v)`, `array::matches(a,v)`, `array::is_empty(a)`, `array::min(a)`, `array::max(a)`, `array::any(a,cond)`, `array::all(a,cond)`

**Transform:**
`array::distinct(a)`, `array::flatten(a)`, `array::reverse(a)`, `array::sort(a)`, `array::shuffle(a)`, `array::slice(a,s,l)`, `array::clump(a,size)`, `array::swap(a,i1,i2)`, `array::transpose(a)`, `array::windows(a,size)`

**Higher-item:**
`array::map(a, closure)`, `array::filter(a, closure)`, `array::fold(a, init, closure)`, `array::reduce(a, closure)`, `array::group(a)` *(aggregate only — see Aggregate Functions in the main skill file)*

> [!WARNING]
> **No `array::remove_value()`.** Use `array::complement(a, [vals])` to remove known values, or inline `arr[WHERE status != NONE]` to filter by linked-record fields.

### Closures / Lambdas

Used with `array::map` and other higher-item functions:

```sql
array::map($values, |$v: any| IF !type::is::none($v) { $v } ELSE { 0 });
array::map($items, |$i: record<item>| fn::link_count($i, $since));
```

### Math

`math::abs(n)`, `math::ceil(n)`, `math::floor(n)`, `math::round(n)`, `math::sqrt(n)`, `math::pow(base, exp)`, `math::log(n, base)`, `math::ln(n)`, `math::log10(n)`

### Time

`time::now()`, `time::format(dt, fmt)`, `time::floor(dt, duration)`, `time::round(dt, duration)`

### Type

`type::thing(table, id)` — construct record ID

`type::is::record(v)`, `type::is::array(v)`, `type::is::bool(v)`, `type::is::int(v)`, `type::is::string(v)`, `type::is::number(v)`, `type::is::object(v)`, `type::is::datetime(v)`, `type::is::none(v)`, `type::is::null(v)`

### Record

`record::id(r)` / `meta::id(r)` — extract key

`record::tb(r)` / `meta::tb(r)` — extract table name

### Random

`rand::uuid()`, `rand::ulid()`, `rand::int(min, max)`, `rand::float(min, max)`

## Schema Definition

### OVERWRITE keyword

Use `OVERWRITE` to replace an existing definition without error:

```sql
DEFINE FUNCTION OVERWRITE fn::my_func() { ... };
DEFINE EVENT OVERWRITE my_event ON TABLE item WHEN ... THEN { ... };
DEFINE FIELD OVERWRITE status ON TABLE item TYPE string DEFAULT 'open';
```

### Tables, Fields, Indexes

```sql
-- Table
DEFINE TABLE item SCHEMAFULL;
DEFINE TABLE item TYPE NORMAL;
DEFINE TABLE link TYPE RELATION FROM item TO item;

-- Field types (including option<T> for nullable)
DEFINE FIELD name      ON TABLE item   TYPE string;
DEFINE FIELD key       ON TABLE item   TYPE string ASSERT string::len($value) > 0;
DEFINE FIELD status    ON TABLE item   TYPE string DEFAULT 'open';
DEFINE FIELD updated   ON TABLE item   TYPE datetime VALUE time::now();
DEFINE FIELD bucket    ON TABLE item   TYPE option<record<bucket>>; -- nullable record link
DEFINE FIELD meta      ON TABLE link   TYPE object;
DEFINE FIELD tags      ON TABLE item   TYPE array<string>;

-- Prefer idempotent definitions so schema can be re-applied safely.
DEFINE INDEX IF NOT EXISTS idx_status ON TABLE item FIELDS status;
DEFINE INDEX IF NOT EXISTS idx_key    ON TABLE item FIELDS key UNIQUE;

-- Permissions
DEFINE TABLE item PERMISSIONS
  FOR select WHERE id = $auth
  FOR update WHERE id = $auth
  FOR create, delete NONE
;

-- Parameters
DEFINE PARAM $default_region VALUE 'eu';
```

### DEFINE FUNCTION (custom functions)

```sql
DEFINE FUNCTION OVERWRITE fn::percent_of($part: float, $whole: float) {
  RETURN IF $whole = 0 {
    0
  } ELSE {
    math::round($part / $whole * 100)
  };
} COMMENT 'returns part as a rounded percentage of whole'
  PERMISSIONS FULL;

-- With typed params and return type
DEFINE FUNCTION fn::recent_links(
  $item: record<item>,
  $since: datetime,
  $limit: option<number>  -- optional param
) -> any {
  RETURN (SELECT * FROM link WHERE in = $item AND at > $since LIMIT $limit ?? 10);
} PERMISSIONS FULL;
```

Call custom functions with `fn::` prefix:

```sql
SELECT fn::percent_of(3, 12);
LET $result = fn::recent_links(item:⟨abc⟩, time::now() - 7d, 5);
```

### DEFINE EVENT (triggers)

```sql
DEFINE EVENT OVERWRITE item_closed_trigger ON TABLE item
  WHEN $before.status != $after.status
    AND ($after.status = 'paid' OR $after.status = 'cancelled')
  THEN {
    -- $event is 'CREATE', 'UPDATE', or 'DELETE'
    -- $before is the record state before the change
    -- $after is the record state after the change
    -- $value is the current record value
    CREATE job SET payload = $after.id;
  };
```

Event variables: `$event` (action type), `$before` (previous state), `$after` (new state), `$value` (current value).

### REMOVE statements

```sql
REMOVE TABLE old_table;
REMOVE FIELD old_field ON TABLE item;
REMOVE INDEX old_idx ON TABLE item;
REMOVE EVENT old_event ON TABLE item;
REMOVE FUNCTION fn::old_func;
```

## Transactions

```sql
BEGIN TRANSACTION;
  CREATE item:⟨i1⟩ SET parent = item:⟨abc⟩, status = 'open';
  RELATE item:⟨abc⟩ -> link -> item:⟨i2⟩;
COMMIT TRANSACTION;

-- Or rollback:
BEGIN TRANSACTION;
  DELETE item:⟨abc⟩;
CANCEL TRANSACTION;  -- nothing deleted
```

## Info Statements

```sql
INFO FOR DB;          -- all tables, params, users, etc.
INFO FOR TABLE item;  -- fields, indexes, events for a table
INFO FOR NS;          -- namespace-level info
```

## Durations

Used in `TIMEOUT`, time arithmetic, etc.: `1ns`, `1us`, `1ms`, `1s`, `1m`, `1h`, `1d`, `1w`

```sql
SELECT * FROM item WHERE created_at > time::now() - 7d;
UPDATE item SET status = 'stale' WHERE updated_at < time::now() - 1h TIMEOUT 5s;
```

> [!WARNING]
> **Cannot subtract two datetimes.** `datetime - datetime` does not produce a duration. Use `time::now() - 7d` (datetime minus duration) instead.

## SurrealDB Type Reference

| SurrealQL Type | Description |
|----------------|-------------|
| `any` | Accepts any type |
| `bool` | Boolean |
| `int` | 64-bit signed integer |
| `float` | 64-bit float |
| `decimal` | Arbitrary-precision decimal |
| `number` | int, float, or decimal |
| `string` | Text |
| `datetime` | Date and time |
| `duration` | Time duration |
| `object` | Key-value map |
| `array` | Ordered list |
| `array<string>` | Typed array |
| `set` | Unique unordered collection |
| `record` | Any record link |
| `record<item>` | Record link to specific table |
| `option<T>` | Nullable — either `T` or `NONE` |
| `bytes` | Byte array |
| `uuid` | UUID |
| `null` | Explicit null |
| `none` | Absence of value |