# Observability

This scaffold can export runtime traces to both LangSmith and Langfuse through OpenTelemetry OTLP/HTTP.

## What Is Traced

- `main_agent.run` root spans for normal session runs
- loop-iteration metadata for task contracts, planner decisions, observations, retries, and stop reasons
- `subagent.*` spans for Claude-style delegated subagent execution
- `tool.*` spans for tool execution
- runtime log lines as span events when a traced span is active
- final-output metadata on the active span
- terminal-visible planner context previews before model calls, with append-only JSON snapshots in `history/context_history.json`

## LangSmith Setup

Set these environment variables:

```bash
export LANGSMITH_OTEL_ENABLED=true
export LANGSMITH_API_KEY=...
export LANGSMITH_PROJECT=agent-in-rust
```

Optional:

```bash
export LANGSMITH_ENDPOINT=https://api.smith.langchain.com
export OTEL_SERVICE_NAME=agent_in_rust
```

The exporter sends traces to `LANGSMITH_ENDPOINT/otel/v1/traces`.

## Langfuse Setup

Set these environment variables:

```bash
export LANGFUSE_TRACING_ENABLED=true
export LANGFUSE_PUBLIC_KEY=...
export LANGFUSE_SECRET_KEY=...
```

Optional:

```bash
export LANGFUSE_BASE_URL=https://cloud.langfuse.com
export OTEL_SERVICE_NAME=agent_in_rust
```

The exporter sends traces to `LANGFUSE_BASE_URL/api/public/otel/v1/traces`.

## Fan-Out

If both LangSmith and Langfuse are configured, the agent exports the same trace tree to both backends.

## Notes

- No exporter is created unless at least one backend is configured correctly.
- Misconfigured observability only emits warnings into the local agent trace; it does not stop the agent loop.
- The current implementation uses simple OTLP exporters to keep the scaffold small and synchronous.
- The harness-first rewrite keeps MCP and web-search tools visible in the same trace tree as delegated subagents.
