# Graph Report - .  (2026-05-07)

## Corpus Check
- 22 files · ~0 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 668 nodes · 1280 edges · 21 communities detected
- Extraction: 52% EXTRACTED · 48% INFERRED · 0% AMBIGUOUS · INFERRED: 615 edges (avg confidence: 0.5)
- Token cost: 0 input · 0 output

## God Nodes (most connected - your core abstractions)
1. `MainAgent` - 46 edges
2. `temp_root()` - 38 edges
3. `parse_command()` - 22 edges
4. `test_agent()` - 16 edges
5. `spawn_http_test_server()` - 10 edges
6. `lookup()` - 10 edges
7. `agent_memory_paths()` - 9 edges
8. `planner_test_agent()` - 9 edges
9. `McpClient` - 9 edges
10. `Observability` - 9 edges

## Surprising Connections (you probably didn't know these)
- `compact_session_state()` --calls--> `merge_compacted_summary()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 1 → community 0_
- `load_memory_stack_with_request()` --calls--> `agent_memory_paths()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 1 → community 9_

## Communities

### Community 0 - "Community 0"
Cohesion: 0.02
Nodes (123): ActiveSkill, ActualContextSnapshot, AgentConfig, AgentMemoryPaths, AnthropicContentBlock, AnthropicEngine, AnthropicResponse, AnthropicUsageBody (+115 more)

### Community 1 - "Community 1"
Cohesion: 0.05
Nodes (48): active_skill_allowed_tools_are_enforced(), agent_memory_paths(), agent_memory_paths_use_workspace_directory(), AgentResult, append_long_term_memory(), append_long_term_memory_uses_mem0_when_configured(), automatic_compaction_summarizes_older_history(), compact_session_state() (+40 more)

### Community 2 - "Community 2"
Cohesion: 0.05
Nodes (57): build_eval_suite(), build_eval_suite_errors_when_fixture_dir_is_empty(), build_eval_suite_reports_passing_and_failing_cases(), CliError, Command, defaults_to_session_when_no_args_are_provided(), EvalOptions, help_flag_returns_general_help() (+49 more)

### Community 3 - "Community 3"
Cohesion: 0.1
Nodes (13): Bus, bus_trait_accepts_inbound_and_outbound_envelopes(), EventId, EventSource, EventSourceKind, generated_event_ids_use_event_prefix(), in_process_bus_stores_and_forwards_events_in_fifo_order(), inbound_and_outbound_events_capture_ids_and_source_metadata() (+5 more)

### Community 4 - "Community 4"
Cohesion: 0.09
Nodes (18): drain_stderr(), empty_to_default(), format_mcp_tool_result(), formats_structured_mcp_tool_result(), load_mcp_catalog_from_env(), McpCallToolResult, McpCatalog, McpClient (+10 more)

### Community 5 - "Community 5"
Cohesion: 0.12
Nodes (16): build_langfuse_target(), build_langsmith_target(), build_provider(), builds_langfuse_target_with_basic_auth_header(), builds_langsmith_target_with_project_header(), compact_text(), compact_text_truncates_long_values(), env_flag() (+8 more)

### Community 6 - "Community 6"
Cohesion: 0.09
Nodes (13): ChannelAdapter, ChannelWorker, DeliveryReceipt, Tool, ToolHandler, extract_web_search_query(), format_perplexity_response(), PerplexityChoice (+5 more)

### Community 7 - "Community 7"
Cohesion: 0.11
Nodes (17): eval_suite_render_includes_pass_fail_summary(), load_eval_fixtures(), load_eval_fixtures_reads_sorted_json_files(), parse_eval_fixture(), parse_eval_fixture_allows_empty_user_input_for_stop_action(), parse_eval_fixture_rejects_missing_target_for_targeted_actions(), parse_eval_fixture_supports_user_input_observations_and_expected_action(), PlannerEvalActualDecision (+9 more)

### Community 8 - "Community 8"
Cohesion: 0.17
Nodes (14): dispatch_request_targets_a_subagent_without_direct_loop_coupling(), Dispatcher, DispatchRequest, DispatchResponse, DispatchTarget, local_dispatcher_returns_structured_dispatch_response(), LocalDispatcher, LocalDispatcher<H> (+6 more)

### Community 9 - "Community 9"
Cohesion: 0.1
Nodes (16): configured_env(), extract_imports(), http_client(), insert_optional_json_field(), load_memory_source(), load_memory_stack_with_request(), Mem0Config, Mem0MemoryAgent (+8 more)

### Community 10 - "Community 10"
Cohesion: 0.16
Nodes (13): classify_planner_failure_kind(), Decision, failure_trace(), finish_run(), planner_failure_run(), PlannerFailureKind, PlannerRouter, PlannerRun (+5 more)

### Community 11 - "Community 11"
Cohesion: 0.14
Nodes (11): FakeWorkerAgent, MainAgent, MainAgentWorker, MainAgentWorker<'_, A>, worker_executes_session_commands_through_wait_mode(), worker_executes_user_message_requests_through_agent_run(), WorkerAgentRuntime, WorkerMode (+3 more)

### Community 12 - "Community 12"
Cohesion: 0.19
Nodes (6): cli_channel_represents_interactive_session_as_command_event(), cli_channel_represents_one_shot_input_as_user_message_event(), cli_delivery_worker_records_outbound_delivery_receipts(), CliChannel, CliDeliveryWorker, CliInvocation

### Community 13 - "Community 13"
Cohesion: 0.18
Nodes (9): ConcurrencyGate, FakeWorker, gated_worker_runtime_releases_slot_after_execution(), GatedWorkerRuntime, GatedWorkerRuntime<W, G>, local_concurrency_gate_limits_and_releases_worker_slots(), local_concurrency_gate_rejects_zero_slots(), LocalConcurrencyGate (+1 more)

### Community 14 - "Community 14"
Cohesion: 0.16
Nodes (7): file_memory_agent_reads_and_appends_long_term_notes(), FileMemoryAgent, MemoryAgent, MemoryAgentSnapshot, stub_memory_agent_exposes_a_future_memory_worker_shape(), StubMemoryAgent, temp_path()

### Community 15 - "Community 15"
Cohesion: 0.23
Nodes (5): local_scheduler_emits_only_explicitly_enqueued_events(), local_scheduler_produces_no_background_events_by_default(), LocalScheduler, ScheduledEvent, Scheduler

### Community 16 - "Community 16"
Cohesion: 0.2
Nodes (9): ActiveSkillLayer, ChannelMetadataLayer, ObservationLayer, planner_prompt_layers_render_explicit_sections_in_stable_order(), PlannerPromptLayers, ProjectMemoryLayer, render_list(), SystemPromptLayer (+1 more)

### Community 17 - "Community 17"
Cohesion: 0.33
Nodes (6): default_router_routes_session_commands_to_main_worker(), default_router_routes_user_messages_to_main_worker(), DefaultRouter, RouteDecision, Router, RouteTarget

### Community 18 - "Community 18"
Cohesion: 0.5
Nodes (1): TokenUsageRecord

### Community 19 - "Community 19"
Cohesion: 1.0
Nodes (1): MainAgentWorker<'a, A>

### Community 20 - "Community 20"
Cohesion: 1.0
Nodes (0): 

## Knowledge Gaps
- **88 isolated node(s):** `ToolHandler`, `PerplexityChoice`, `PerplexityMessage`, `PerplexitySearchResult`, `EventSourceKind` (+83 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 19`** (2 nodes): `MainAgentWorker<'a, A>`, `.new()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 20`** (1 nodes): `private.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `MainAgent` connect `Community 1` to `Community 0`?**
  _High betweenness centrality (0.087) - this node is a cross-community bridge._
- **Are the 37 inferred relationships involving `temp_root()` (e.g. with `agent_memory_paths_use_workspace_directory()` and `scaffold_claude_project_creates_system_prompt_file()`) actually correct?**
  _`temp_root()` has 37 INFERRED edges - model-reasoned connections that need verification._
- **Are the 21 inferred relationships involving `parse_command()` (e.g. with `parse_common_flags()` and `.new()`) actually correct?**
  _`parse_command()` has 21 INFERRED edges - model-reasoned connections that need verification._
- **Are the 15 inferred relationships involving `test_agent()` (e.g. with `agent_memory_paths()` and `.new()`) actually correct?**
  _`test_agent()` has 15 INFERRED edges - model-reasoned connections that need verification._
- **Are the 9 inferred relationships involving `spawn_http_test_server()` (e.g. with `load_memory_stack_uses_mem0_when_configured()` and `append_long_term_memory_uses_mem0_when_configured()`) actually correct?**
  _`spawn_http_test_server()` has 9 INFERRED edges - model-reasoned connections that need verification._
- **What connects `ToolHandler`, `PerplexityChoice`, `PerplexityMessage` to the rest of the system?**
  _88 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.02 - nodes in this community are weakly interconnected._