# Step 15: Agent Dispatch

> Your Agent want friends to work with!

## Prerequisites

Same as Step 09 - copy the config file and add your API key:

```bash
cp default_workspace/config.example.yaml default_workspace/config.user.yaml
# Edit config.user.yaml to add your API key
```

## What We Will Build

Agents can delegate work to other agents. 

<img src="15-agent-dispatch.svg" align="center" width="100%" />

## Key Components

- **subagent_tool** - Factory that creates a dispatch tool with dynamic schema

[src/mybot/tools/subagent_tool.py](src/mybot/tools/subagent_tool.py)

```python
def create_subagent_dispatch_tool(
    current_agent_id: str,
    context: "SharedContext",
) -> BaseTool | None:
    available_agents = context.agent_loader.discover_agents()
    dispatchable_agents = [a for a in available_agents if a.id != current_agent_id]

    agents_desc = "<available_agents>\n"
    for agent_def in dispatchable_agents:
        agents_desc += f'  <agent id="{agent_def.id}">{agent_def.description}</agent>\n'
    agents_desc += "</available_agents>"

    @tool(
        name="subagent_dispatch",
        description=f"Dispatch a task to a specialized subagent.\n{agents_desc}",
        parameters={...},
    )
    async def subagent_dispatch(
        agent_id: str, task: str, session: "AgentSession", context: str = ""
    ) -> str:
        agent_def = shared_context.agent_loader.load(agent_id)
        agent = Agent(agent_def, shared_context)
        agent_source = AgentEventSource(agent_id=current_agent_id)
        agent_session = agent.new_session(agent_source)
        session_id = agent_session.session_id

        user_message = task
        if context:
            user_message = f"{task}\n\nContext:\n{context}"

        loop = asyncio.get_running_loop()
        result_future: asyncio.Future[str] = loop.create_future()

        # Create temp handler that filters by session_id
        async def handle_result(event: DispatchResultEvent) -> None:
            if event.session_id == session_id:
                if not result_future.done():
                    if event.error:
                        result_future.set_exception(Exception(event.error))
                    else:
                        result_future.set_result(event.content)

        # Subscribe to DispatchResultEvent events
        shared_context.eventbus.subscribe(DispatchResultEvent, handle_result)

        try:
            event = DispatchEvent(
                session_id=session_id,
                source=AgentEventSource(agent_id=current_agent_id),
                content=user_message,
                timestamp=time.time(),
                parent_session_id=session.session_id,
            )
            await shared_context.eventbus.publish(event)

            response = await result_future
        finally:
            shared_context.eventbus.unsubscribe(handle_result)

        result = {"result": response, "session_id": session_id}
        return json.dumps(result)

    return subagent_dispatch
```

### How the Dispatch Mechanism Works

The dispatch mechanism relies on the **eventbus** pattern for communication between agents. Here's the flow:

1. **Publish**: When the main agent calls `subagent_dispatch`, it publishes a `DispatchEvent` to the eventbus containing the task and session information.
2. **Subscribe**: A temporary handler subscribes to `DispatchResultEvent` events, filtering by the session ID to receive only the response for this specific dispatch.
3. **Await**: The main agent awaits a future that gets resolved when the subagent completes its work and publishes a `DispatchResultEvent`.
4. **Cleanup**: After receiving the result, the handler unsubscribes from the eventbus.


## Try it out

```bash
cd 15-agent-dispatch
uv run my-bot chat

# You: Ask Cookie to read our README.
# pickle: Cookie has sent our README back! *purrs* 🐱

# # Step 15: Agent Dispatch

# > Your Agent want friends to work with!
# ...
```

## Notes

### Alternative Multi-Agent Patterns

Direct subagent dispatching is just one approach to multi-agent orchestration. Here are some other common patterns:

- **Shared Task Lists**: Agents coordinate by reading from and writing to a shared task queue or database. Each agent picks up tasks as they become available, agent never talk to agent directly.
- **Tmux/Screen Sessions**: `tmux` allow us running multiple processes. A Tmux skill can be provided to agent to guide it execute multiple tasks, achieving multi-agent to some extent. 

## What's Next

[Step 16: Concurrency Control](../16-concurrency-control/) - Rate limiting and queue management.
