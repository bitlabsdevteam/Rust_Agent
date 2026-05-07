# Step 16: Concurrency Control

> Too many Pickle are running at the same time?

## Prerequisites

Same as Step 09 - copy the config file and add your API key:

```bash
cp default_workspace/config.example.yaml default_workspace/config.user.yaml
# Edit config.user.yaml to add your API key
```

## What We Will Build

Prevent resource exhaustion by limiting how many instances of an agent can run simultaneously.

<img src="16-concurrency-control.svg" align="center" width="100%" />

## Key Components

- **AgentDef.max_concurrency** - Configurable per-agent limit
- **Semaphore Based Concurrency Control** - Blocks when concurrency limit reached


[src/mybot/server/agent_worker.py](src/mybot/server/agent_worker.py)

```python
class AgentWorker(SubscriberWorker):
    def __init__(self, context: "SharedContext"):
        super().__init__(context)
        self._semaphores: dict[str, asyncio.Semaphore] = {}

    async def exec_session(self, event, agent_def: "AgentDef") -> None:
        sem = self._get_or_create_semaphore(agent_def)

        async with sem:  # Blocks if limit reached
            # ... execute session ...

        self._maybe_cleanup_semaphores(agent_def)

    def _get_or_create_semaphore(self, agent_def: "AgentDef") -> asyncio.Semaphore:
        if agent_def.id not in self._semaphores:
            self._semaphores[agent_def.id] = asyncio.Semaphore(
                agent_def.max_concurrency
            )
        return self._semaphores[agent_def.id]
```

## Try it out

`Cookie` has `max_concurrency` as 1, dispatch from two different source (Non-cli) should trigger this. 

## Note

Concurrency control can be implemented using different granularities depending on your use case:

- **By Agent** (shown here) - Limits concurrent executions per agent type. Useful when certain agents are resource-intensive or have rate limits.
- **By Source** - Limits concurrent requests from the same user/client. Useful for preventing abuse or ensuring fair resource distribution.
- **By Priority** - Different concurrency limits for different priority levels. High-priority tasks could have reserved capacity.

## What's Next

[Step 17: Memory](../17-memory/) - Long-term knowledge system.
