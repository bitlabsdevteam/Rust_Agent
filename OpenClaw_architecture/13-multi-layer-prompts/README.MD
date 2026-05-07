# Step 13: Multi-Layer Prompts

> More Context, More Context, More Context.

## Prerequisites

Same as Step 09 - copy the config file and add your API key:

```bash
cp default_workspace/config.example.yaml default_workspace/config.user.yaml
# Edit config.user.yaml to add your API key
```

## What We Will Build

Rich system prompts assembled from multiple layers—identity, personality, workspace context, runtime info. 

<img src="13-multi-layer-prompts.svg" align="center" width="100%" />

## Key Components

- **AgentDef** - `soul_md` extension
- **PromptBuilder** - Assembles all prompt layers into final system prompt

[src/mybot/core/agent_loader.py](src/mybot/core/agent_loader.py)

```python
class AgentDef(BaseModel):
    id: str
    name: str
    description: str = ""
    agent_md: str
    soul_md: str = ""  # NEW: Personality layer (optional)
    llm: LLMConfig
    allow_skills: bool = False
```

[src/mybot/core/prompt_builder.py](src/mybot/core/prompt_builder.py)
```python
class PromptBuilder:
    def build(self, state: "SessionState") -> str:
        layers = []

        # Layer 1: Identity
        layers.append(state.agent.agent_def.agent_md)

        # Layer 2: Soul (optional)
        if state.agent.agent_def.soul_md:
            layers.append(f"## Personality\n\n{state.agent.agent_def.soul_md}")

        # Layer 3: Bootstrap context (BOOTSTRAP.md + AGENTS.md + crons)
        bootstrap = self._load_bootstrap_context()
        if bootstrap:
            layers.append(bootstrap)

        # Layer 4: Runtime context
        layers.append(self._build_runtime_context(agent_id, timestamp))

        # Layer 5: Channel hint
        layers.append(self._build_channel_hint(source))

        return "\n\n".join(layers)
```

### Example Workspace Setup

- [default_workspace/agents/pickle/AGENT.md](../default_workspace/agents/pickle/AGENT.md) - Agent identity, capabilities, and behavioral guidelines (with YAML frontmatter for config)
- [default_workspace/agents/pickle/SOUL.md](../default_workspace/agents/pickle/SOUL.md) - Personality layer defining the agent's character and tone
- [default_workspace/BOOTSTRAP.md](../default_workspace/BOOTSTRAP.md) - Workspace guide describing directory structure, file purposes, and path templates for agents, skills, crons, and memories
- [default_workspace/AGENTS.md](../default_workspace/AGENTS.md) - Lists all available agents and provides dispatching patterns for delegating tasks to specialized agents (e.g., memory operations)


## Try it out

```bash
cd 13-multi-layer-prompts
uv run my-bot server

# From Channel of your choice:

# You: When are Where are we talking?
# pickle: Meow! Let me check... We're chatting right now via Telegram! *twitches ears*

# The current time is 2026-03-13 at 23:04:45. So we're here, in this conversation, happening in real-time. 🐱
```

## Note

This architecture is extensible - more prompt layers can be added as needed. For example, a **memory layer** could inject relevant memories from past conversations, providing context about the user's preferences, ongoing projects, or historical interactions. 

## What's Next

[Step 14: Post Message Back](../14-post-message-back/) - Agent-initiated communication.
