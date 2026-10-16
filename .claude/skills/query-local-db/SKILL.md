---
name: query-local-db
description: How to query the local SurrealDB instance for debugging and data inspection. Use when the task involves inspecting DB tables, checking record counts, verifying data written by a feature or job, or debugging failed background work.
---

# Query Local DB

Use the `surreal-local` script to run SurrealQL against the local SurrealDB instance. The script refuses any endpoint that is not `localhost`/`127.0.0.1`, so it **cannot** reach a shared or production database.

## When to Use

- Inspecting table contents or schema
- Debugging failed background jobs or queue records
- Verifying data written by a feature under development
- Checking record counts or data integrity
- Any task that says "check the DB" or "what's in the database"

## Usage

The script lives at `.agent/skills/query-local-db/surreal-local`. Symlink it onto your `PATH` (see Setup) so `surreal-local` can be added to agent tool allow-lists as a simple command name.

```bash
surreal-local <<< "INFO FOR DB;"
surreal-local <<< "SELECT count() FROM item GROUP ALL; SELECT count() FROM item GROUP ALL;"
surreal-local            # interactive REPL
```

### Configuration

| Variable | Default |
|----------|---------|
| `SURREAL_ENDPOINT` | `ws://localhost:8000` |
| `SURREAL_NS` | `app` |
| `SURREAL_DB` | `app` |
| `SURREAL_USER` | `root` |
| `SURREAL_PASS` | `root` |

Match these to the project's local dev environment (for example `.env.local`).

## Discover the Schema

Do not assume table names. Inspect the live schema first:

```sql
INFO FOR DB;            -- tables, functions, params
INFO FOR TABLE item;    -- fields, indexes, events for one table
```

Then cross-check against the project's model definitions in source.

## Common Queries

```sql
-- Row counts
SELECT count() FROM item GROUP ALL;

-- Most recent records by timestamp
SELECT * FROM item ORDER BY created_at DESC LIMIT 5;

-- Follow a graph edge (in = item, out = item)
SELECT ->link->item FROM item:⟨ID⟩;
```

## Setup

1. Ensure the SurrealDB binary is available as `surreal` on `PATH`.
2. Symlink the script:
   ```bash
   sudo ln -sf "$(pwd)/.agent/skills/query-local-db/surreal-local" /usr/local/bin/surreal-local
   ```
3. Verify: `surreal-local <<< "INFO FOR DB;"`

## Safety

- **Local only** — the script rejects non-local endpoints.
- **Prefer read-only queries** — use `SELECT`/`INFO`; avoid `DELETE`/`UPDATE` unless explicitly asked.
- **Dev credentials only** — never point this script at real credentials.

## Notes

- The SurrealDB CLI writes `history.txt` in the working directory with every executed query. Keep it git-ignored.
- For SurrealQL syntax, invoke and follow the `surrealql-syntax` skill.
- If a query fails on syntax that `surrealql-syntax` does not cover, fix the query **and** add the correct syntax to that skill.
