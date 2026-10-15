# CLI and User Experience

**K8s analogue:** kubectl (get/describe/apply/diff/edit/logs/exec/port-forward/rollout/
debug/explain/auth can-i/top/wait), kubeconfig and contexts, krew plugins, YAML manifests,
kustomize built into kubectl, `kubectl events`.

## Design goals
1. **Answer "why"** – the most common user question is "why isn't X working?"; Kubernetes
   makes you correlate `describe`, events, logs, and conditions across objects.
2. **Short commands** for common tasks, structured output for scripts.
3. **Safety**: diffs before applies, confirmation for destructive cluster-wide actions,
   dry-run everywhere.
4. **Fast**: single static binary; startup < 20 ms; uses aggregated discovery cached on disk.

## Command overview

```
rwctl get <kind>[/<name>]        [-n ns | -A] [-l sel] [-o table|wide|yaml|json|full|name]
rwctl describe <kind>/<name>     # object + related objects + recent events + blocking reasons
rwctl why <kind>/<name>          # causal explanation (see below)
rwctl apply -f <file|dir|->      [--dry-run=server] [--diff] [--prune --apply-set=<name>]
rwctl diff -f ...
rwctl delete <kind>/<name>       [--cascade=background|foreground|orphan]
rwctl edit <kind>/<name>
rwctl logs <pod|workload>        [-c container] [-f] [--since] [--previous] [--all-containers]
rwctl exec / attach / port-forward / cp
rwctl debug pod/<x> | node/<n>
rwctl rollout status|history|undo|pause|resume|promote|abort <workload>
rwctl scale <workload> --replicas N
rwctl top nodes|pods
rwctl wait <kind>/<name> --for=Ready|Deleted|condition=X [--timeout]
rwctl events [--for <obj>] [--watch]
rwctl explain <kind>[.field.path]
rwctl auth can|who-can|explain
rwctl net can <src> <dst:port>   # policy analysis
rwctl net trace <pod>            # live drop/trace events [later]
rwctl node drain|cordon|uncordon|remove <n>
rwctl lint -f ...                # best-practice checks offline
rwctl import -f k8s.yaml         # Kubernetes manifests → Rustwain
rwctl convert -f ... --to v1
rwctl login                      # OIDC
rwctl context use|list
rwctl cluster info|features|health
rwctl upgrade plan|apply|finalize
rwctl store snapshot save|restore
rwctl pki status|rotate
```

## `rwctl why`
Traverses the object graph and status to produce a causal chain:

```
$ rwctl why deployment/web
Deployment web: 2/5 available (Progressing)
└─ Revision web-6f9c: 3 pods not ready
   ├─ pod web-6f9c-x2k: Pending 4m
   │  └─ Unschedulable: 0/12 nodes fit
   │     ├─ 8 nodes: insufficient memory (need 2Gi, max free 1.2Gi on node-4)
   │     └─ 4 nodes: taint gpu=true:NoSchedule not tolerated
   │     hint: reduce requests.memory or add capacity to pool "web"
   ├─ pod web-6f9c-p9q: CrashLooping (exit 1, 7 restarts, next in 40s)
   │  └─ last log: "error: DATABASE_URL not set"
   │     hint: env DATABASE_URL references secret db/url which does not exist
   └─ pod web-6f9c-z1m: Running, readiness failing: GET :8080/ready → 503 (12 times)
```

Implemented client-side from API data plus server-provided `blocking` reasons; extensible
per kind (custom types can register "why" rules via conditions conventions).

## Configuration
- `~/.config/rustwain/config` (TOML): clusters, users (credential references, not inline
  secrets where avoidable – exec credential plugins supported), contexts.
- `RWCONFIG` env var; `--context` flag.
- Kubernetes' kubeconfig confusion (merging multiple files, current-context in shared
  file) → keep it simple: one file, explicit contexts, `rwctl context use` writes only the
  `current` key.

## Manifest format
- YAML (and JSON) for files; **TOML not used for manifests** (deep nesting).
- Multi-document YAML, directories, `-k` overlays (native overlays – see
  [packaging and GitOps](08-packaging-and-gitops.md)).
- Strict decoding: unknown fields are **errors** by default (Kubernetes silently dropped
  unknown fields for years; field validation became server-side strict in 1.27 with
  warnings). Typos must fail loudly.

## Output design
- Tables sized to terminal, with `AGE`, `READY`, `STATUS`, and a `REASON` column showing
  the blocking reason when not healthy.
- Colour when TTY; `NO_COLOR` respected.
- `-o full` shows defaulted fields marked as defaults.
- Machine output stable (JSON with versioned schema).

## Errors
Every error includes: what failed, why (server reason code), and a hint when known.
```
error: apply deployment/web: field spec.template.spec.containers[0].resources.requests.cpu:
       invalid quantity "500mi" (did you mean "500m"?)
```

## Plugins
`rwctl-<name>` binaries on PATH become subcommands (like kubectl/krew). No plugin manager
initially.

## Shell completion
`rwctl completion bash|zsh|fish` – dynamic completion of resource names using cached
discovery and a fast API call.

## Open questions

- Should `describe` and `why` merge? (`describe` = full dump; `why` = diagnosis). Keep both.
- Interactive TUI (k9s-like)? Not a frontend per se, but scope creep; `[later]` maybe.

## Build checklist

- [ ] `rwctl` crate with clap; config/context management; output formatters.
- [ ] apply/diff with SSA and apply sets (prune).
- [ ] `why` engine with per-kind rules.
- [ ] Strict manifest decoding with suggestions (edit distance on field names).
- [ ] `import` for core Kubernetes kinds.
- [ ] Completion.
