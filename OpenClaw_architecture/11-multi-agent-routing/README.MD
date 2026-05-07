# Step 11: Multi-Agent Routing

> Route right job to right agent.

## Prerequisites

Same as Step 10 - copy the config file and add your API key:

```bash
cp default_workspace/config.example.yaml default_workspace/config.user.yaml
# Edit config.user.yaml to add your API keys
```

## What We Will Build

Route messages to different agents based on where they come from. 

<img src="11-multi-agent-routing.svg" align="center" width="100%" />

## Key Components

- **AgentLoader** - Agent Discoveries for multi agent definition support
- **RoutingTable** - Routes sources to agents using regex bindings with tiered specificity
- **Binding** - A source pattern + agent mapping with automatic tier computation
- **New Commands** - `/route`, `/bindings`, `/agents` for managing multi-agent setup

[src/mybot/core/agent_loader.py](src/mybot/core/agent_loader.py)

```python
class AgentLoader:
    def discover_agents(self) -> list[AgentDef]:
        """Scan agents directory and return list of valid AgentDef."""
        return discover_definitions(
            self.config.agents_path, "AGENT.md", self._parse_agent_def
        )
```

Define customized agent in `<workspace>/agents/<agent_id>/AGENT.md`

[src/mybot/core/routing.py](src/mybot/core/routing.py)

```python
@dataclass
class Binding:
    agent: str
    value: str
    tier: int 
    pattern: Pattern  # Compiled regex

    def _compute_tier(self) -> int:
        """Compute specificity tier."""
        if not any(c in self.value for c in r".*+?[]()|^$"):
            return 0  # Exact match
        if ".*" in self.value:
            return 2  # Wildcard
        return 1  # Specific regex

@dataclass
class RoutingTable:
    def _load_bindings(self) -> list[Binding]:
        bindings_data = self.context.config.routing.get("bindings", [])

        bindings_with_order = [
            (Binding(agent=b["agent"], value=b["value"]), i)
            for i, b in enumerate(bindings_data)
        ]
        bindings_with_order.sort(key=lambda x: (x[0].tier, x[1]))
        self.bindings = [b for b, _ in bindings_with_order]

        return self.bindings

    def resolve(self, source: str) -> str:
        for binding in self._load_bindings():
            if binding.pattern.match(source):
                return binding.agent
        return self.context.config.default_agent

    def get_or_create_session_id(self, source: EventSource) -> str:
        source_session = self.context.config.sources.get(str(source))
        if source_session:
            return source_session.session_id

        # Resolve agent and create new session
        agent_id = self.resolve(str(source))
        agent_def = self.context.agent_loader.load(agent_id)
        agent = Agent(agent_def, self.context)
        session = agent.new_session(source)

        self.context.config.set_runtime(
            f"sources.{str(source)}", SourceSessionConfig(session_id=session.session_id)
        )
        return session.session_id
```

- **Tiered Routing Rules**: Find rules matching inbound source, starting from most specific rules.
- **Default Fallback**: Fall back to global default agent if no rules match.


[src/mybot/server/channel_worker.py](src/mybot/server/channel_worker.py)

```python
async def callback(message: str, source: EventSource) -> None:
    # ... validation ...

    # Use routing_table to resolve agent from bindings
    session_id = self.context.routing_table.get_or_create_session_id(source)

    # Publish event
    event = InboundEvent(session_id=session_id, source=source, content=message)
    await self.context.eventbus.publish(event)
```

## Try it out

```bash
cd 11-multi-agent-routing
uv run my-bot agents chat

# You: /agent
# pickle: **Agents:**
# - `cookie`: Memory manager for storing, organizing, and retrieving memories
# - `pickle`: A friendly cat assistant talk to user directly, managing daily tasks. (current)

# You: /bindings
# pickle: No routing bindings configured.

# You: /route platform-ws:* cookie
# pickle: ✓ Route bound: `platform-ws:*` → `cookie`
```

## What's Next

[Step 12: Cron + Heartbeat](../12-cron-heartbeat/) - Scheduled tasks and health monitoring.
