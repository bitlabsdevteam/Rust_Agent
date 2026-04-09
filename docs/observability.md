# Observability

This scaffold can export runtime traces to both LangSmith and Langfuse through OpenTelemetry OTLP/HTTP.

## What Is Traced

- `main_agent.run` root spans for normal agent runs
- `planner.decide_next_step` spans for planner decisions
- `tool.*` spans for tool execution
- `skill.*` spans for skill execution
- `openai.plan` and `openai.skill.*` spans for OpenAI-backed planner or installed skill calls
- runtime log lines as span events when a traced span is active
- retry, stop, and final-output metadata on the active span

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
