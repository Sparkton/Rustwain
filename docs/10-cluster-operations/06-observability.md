# Observability (Deferred – Design Notes Only)

**K8s analogue:** component `/metrics` (Prometheus), metrics-server, kube-state-metrics,
cAdvisor, events, structured logging (klog JSON), OpenTelemetry tracing in apiserver,
Hubble (Cilium flow logs), kubelet stats summary API.

**Status: deferred by project scope.** We are not building a monitoring product. This
document exists so that the components we *do* build leave the right hooks, avoiding a
painful retrofit later.

## Hooks every component must have from day one
1. **Structured logs** via `tracing` with stable field names (`pod_uid`, `node`,
   `controller`, `revision`, `request_id`), JSON output option, per-module levels
   changeable at runtime (`rwctl debug log-level agent/node-3 dataplane=debug`).
2. **Metrics registry abstraction**: components record counters/gauges/histograms through
   a thin internal facade (e.g. the `metrics` crate facade) with **no exporter wired by
   default**. Adding a Prometheus/OpenMetrics exporter later is then a small change.
   Naming conventions decided now: `rustwain_<component>_<thing>_<unit>`.
3. **Request IDs / trace context** propagated across API → controllers → agent operations
   (W3C `traceparent` in gRPC metadata), so tracing can be enabled later
   (`tracing-opentelemetry`).
4. **Timings in status**: pod startup phases (`PodTimings`), rollout durations – visible
   via API without any metrics system.
5. **Drop reasons** in the dataplane recorded in per-CPU counter maps and a ring buffer –
   consumable later by a flow-log tool.
6. **Events** (see [audit and events](10-audit-and-events.md)) for human-readable history.

## What is explicitly built now (because other features need it)
- The minimal **usage pipeline** for autoscaling and `rwctl top` (see
  [autoscaling](../08-workloads/03-autoscaling.md)).
- Health endpoints (`/healthz`, `/readyz` with per-check detail) for every server/agent.

## What is deferred
- Prometheus exporters, kube-state-metrics equivalent, dashboards, alerting rules,
  log aggregation, tracing backends, flow visibility UI.

## Open questions
- When observability work resumes: OpenTelemetry-native (OTLP push) vs Prometheus pull?
  OTLP push avoids scraping thousands of agents; Prometheus is the ecosystem default.

## Build checklist (hooks only)
- [ ] `tracing` setup crate with JSON formatter and runtime level control.
- [ ] Metrics facade + naming lint.
- [ ] Trace context propagation in gRPC client/server layers.
- [ ] Health endpoints.
