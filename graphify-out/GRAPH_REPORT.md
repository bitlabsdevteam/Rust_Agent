# Graph Report - .  (2026-04-25)

## Corpus Check
- 21 files · ~0 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 599 nodes · 1114 edges · 34 communities detected
- Extraction: 54% EXTRACTED · 46% INFERRED · 0% AMBIGUOUS · INFERRED: 517 edges (avg confidence: 0.5)
- Token cost: 0 input · 0 output

## God Nodes (most connected - your core abstractions)
1. `MainAgent` - 46 edges
2. `temp_root()` - 27 edges
3. `parse_command()` - 20 edges
4. `test_agent()` - 14 edges
5. `lookup()` - 10 edges
6. `agent_memory_paths()` - 9 edges
7. `McpClient` - 9 edges
8. `Observability` - 9 edges
9. `Tool` - 8 edges
10. `planner_prompt()` - 8 edges

## Surprising Connections (you probably didn't know these)
- `infer_tool_request()` --calls--> `normalize_tool_name()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 20 → community 16_
- `compact_session_state()` --calls--> `merge_compacted_summary()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 0 → community 1_
- `load_memory_stack_with_request()` --calls--> `agent_memory_paths()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 8 → community 19_
- `short_term_memory_round_trips_recent_state()` --calls--> `agent_memory_paths()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 8 → community 0_
- `load_subagent_specs()` --calls--> `load_subagent_dir()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 0 → community 16_

## Communities

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (17): active_skill_allowed_tools_are_enforced(), AgentResult, automatic_compaction_summarizes_older_history(), compact_session_state(), ConversationMessage, explicit_compaction_updates_short_term_snapshot(), load_custom_commands(), load_short_term_memory() (+9 more)

### Community 1 - "Community 1"
Cohesion: 0.04
Nodes (50): ActiveSkill, ActualContextSnapshot, AgentConfig, AgentMemoryPaths, AnthropicContentBlock, ApiInputMessage, collect_candidate_files(), collect_candidate_files_recursive() (+42 more)

### Community 2 - "Community 2"
Cohesion: 0.07
Nodes (50): build_eval_suite(), build_eval_suite_errors_when_fixture_dir_is_empty(), build_eval_suite_reports_passing_and_failing_cases(), CliError, Command, defaults_to_session_when_no_args_are_provided(), EvalOptions, help_flag_returns_general_help() (+42 more)

### Community 3 - "Community 3"
Cohesion: 0.1
Nodes (13): Bus, bus_trait_accepts_inbound_and_outbound_envelopes(), EventId, EventSource, EventSourceKind, generated_event_ids_use_event_prefix(), in_process_bus_stores_and_forwards_events_in_fifo_order(), inbound_and_outbound_events_capture_ids_and_source_metadata() (+5 more)

### Community 4 - "Community 4"
Cohesion: 0.09
Nodes (18): drain_stderr(), empty_to_default(), format_mcp_tool_result(), formats_structured_mcp_tool_result(), load_mcp_catalog_from_env(), McpCallToolResult, McpCatalog, McpClient (+10 more)

### Community 5 - "Community 5"
Cohesion: 0.12
Nodes (15): build_langfuse_target(), build_langsmith_target(), build_provider(), builds_langfuse_target_with_basic_auth_header(), builds_langsmith_target_with_project_header(), compact_text(), compact_text_truncates_long_values(), env_flag() (+7 more)

### Community 6 - "Community 6"
Cohesion: 0.09
Nodes (13): ChannelAdapter, ChannelWorker, DeliveryReceipt, Tool, ToolHandler, extract_web_search_query(), format_perplexity_response(), PerplexityChoice (+5 more)

### Community 7 - "Community 7"
Cohesion: 0.12
Nodes (17): eval_suite_render_includes_pass_fail_summary(), load_eval_fixtures(), load_eval_fixtures_reads_sorted_json_files(), parse_eval_fixture(), parse_eval_fixture_allows_empty_user_input_for_stop_action(), parse_eval_fixture_rejects_missing_target_for_targeted_actions(), parse_eval_fixture_supports_user_input_observations_and_expected_action(), PlannerEvalActualDecision (+9 more)

### Community 8 - "Community 8"
Cohesion: 0.14
Nodes (24): agent_memory_paths(), agent_memory_paths_use_workspace_directory(), append_long_term_memory(), append_long_term_memory_uses_mem0_when_configured(), empty_input_reports_noop_usage(), load_memory_stack(), load_memory_stack_follows_imports(), load_memory_stack_includes_long_term_memory_file() (+16 more)

### Community 9 - "Community 9"
Cohesion: 0.12
Nodes (10): AnthropicEngine, AnthropicResponse, AnthropicUsageBody, build_planner_input_from_prompt(), CompactionResult, OpenAiEngine, planner_history_summary(), planner_prompt() (+2 more)

### Community 10 - "Community 10"
Cohesion: 0.14
Nodes (11): FakeWorkerAgent, MainAgent, MainAgentWorker, MainAgentWorker<'_, A>, worker_executes_session_commands_through_wait_mode(), worker_executes_user_message_requests_through_agent_run(), WorkerAgentRuntime, WorkerMode (+3 more)

### Community 11 - "Community 11"
Cohesion: 0.19
Nodes (6): cli_channel_represents_interactive_session_as_command_event(), cli_channel_represents_one_shot_input_as_user_message_event(), cli_delivery_worker_records_outbound_delivery_receipts(), CliChannel, CliDeliveryWorker, CliInvocation

### Community 12 - "Community 12"
Cohesion: 0.18
Nodes (9): ConcurrencyGate, FakeWorker, gated_worker_runtime_releases_slot_after_execution(), GatedWorkerRuntime, GatedWorkerRuntime<W, G>, local_concurrency_gate_limits_and_releases_worker_slots(), local_concurrency_gate_rejects_zero_slots(), LocalConcurrencyGate (+1 more)

### Community 13 - "Community 13"
Cohesion: 0.16
Nodes (7): file_memory_agent_reads_and_appends_long_term_notes(), FileMemoryAgent, MemoryAgent, MemoryAgentSnapshot, stub_memory_agent_exposes_a_future_memory_worker_shape(), StubMemoryAgent, temp_path()

### Community 14 - "Community 14"
Cohesion: 0.16
Nodes (8): configured_env(), http_client(), insert_optional_json_field(), Mem0Config, Mem0MemoryAgent, parse_mem0_add_response(), parse_mem0_list_response(), render_mem0_memory_contents()

### Community 15 - "Community 15"
Cohesion: 0.2
Nodes (9): dispatch_request_targets_a_subagent_without_direct_loop_coupling(), Dispatcher, DispatchRequest, DispatchResponse, DispatchTarget, local_dispatcher_returns_structured_dispatch_response(), LocalDispatcher, LocalDispatcher<H> (+1 more)

### Community 16 - "Community 16"
Cohesion: 0.18
Nodes (14): empty_to_default(), file_stem_name(), load_command_dir(), load_subagent_dir(), normalize_tool_name(), parse_list_field(), parse_planner_tool_arguments(), parse_skill_spec() (+6 more)

### Community 17 - "Community 17"
Cohesion: 0.23
Nodes (5): local_scheduler_emits_only_explicitly_enqueued_events(), local_scheduler_produces_no_background_events_by_default(), LocalScheduler, ScheduledEvent, Scheduler

### Community 18 - "Community 18"
Cohesion: 0.2
Nodes (9): ActiveSkillLayer, ChannelMetadataLayer, ObservationLayer, planner_prompt_layers_render_explicit_sections_in_stable_order(), PlannerPromptLayers, ProjectMemoryLayer, render_list(), SystemPromptLayer (+1 more)

### Community 19 - "Community 19"
Cohesion: 0.22
Nodes (9): extract_imports(), load_memory_source(), load_memory_stack_with_request(), long_term_memory_agent(), memory_load_request_normalizes_focus_paths_under_root(), memory_selector_hint(), MemoryLoadRequest, normalize_repo_relative_path() (+1 more)

### Community 20 - "Community 20"
Cohesion: 0.2
Nodes (10): heuristic_plan(), infer_skill_request(), infer_skill_request_routes_explicit_skill_requests(), infer_subagent_name(), infer_tool_request(), infer_tool_request_normalizes_legacy_web_search_name(), infer_tool_request_routes_web_queries_to_web_search_tool(), parse_explicit_skill_request() (+2 more)

### Community 21 - "Community 21"
Cohesion: 0.28
Nodes (9): copy_directory_recursive(), create_skill_scaffold(), create_skill_scaffold_writes_skill_md(), default_skill_template(), install_skill_from_source(), normalize_skill_name(), parse_skill_install_source(), resolve_local_skill_source() (+1 more)

### Community 22 - "Community 22"
Cohesion: 0.28
Nodes (9): capture_context_history_entry(), civil_from_days(), context_history_appends_entries_to_history_file(), context_history_path(), emit_context_preview(), iso8601ish_now(), persist_context_history(), render_context_preview() (+1 more)

### Community 23 - "Community 23"
Cohesion: 0.33
Nodes (6): default_router_routes_session_commands_to_main_worker(), default_router_routes_user_messages_to_main_worker(), DefaultRouter, RouteDecision, Router, RouteTarget

### Community 24 - "Community 24"
Cohesion: 0.33
Nodes (6): install_skill_from_local_collection_copies_skill_dir(), invalid_skill_files_are_reported_and_skipped(), load_skill_dir(), load_skill_specs(), project_skill_overrides_user_skill(), valid_skill_markdown()

### Community 25 - "Community 25"
Cohesion: 0.5
Nodes (1): TokenUsageRecord

### Community 26 - "Community 26"
Cohesion: 1.0
Nodes (1): StepOutcome

### Community 27 - "Community 27"
Cohesion: 1.0
Nodes (1): MemoryScope

### Community 28 - "Community 28"
Cohesion: 1.0
Nodes (1): OpenAiResponse

### Community 29 - "Community 29"
Cohesion: 1.0
Nodes (1): OpenAiUsageBody

### Community 30 - "Community 30"
Cohesion: 1.0
Nodes (1): Decision

### Community 31 - "Community 31"
Cohesion: 1.0
Nodes (1): DefinitionScope

### Community 32 - "Community 32"
Cohesion: 1.0
Nodes (1): MainAgentWorker<'a, A>

### Community 33 - "Community 33"
Cohesion: 1.0
Nodes (0): 

## Knowledge Gaps
- **87 isolated node(s):** `ToolHandler`, `PerplexityChoice`, `PerplexityMessage`, `PerplexitySearchResult`, `EventSourceKind` (+82 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 26`** (2 nodes): `StepOutcome`, `.fmt()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 27`** (2 nodes): `MemoryScope`, `.fmt()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 28`** (2 nodes): `OpenAiResponse`, `.output_text()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 29`** (2 nodes): `OpenAiUsageBody`, `.as_token_record()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 30`** (2 nodes): `Decision`, `.fmt()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 31`** (2 nodes): `DefinitionScope`, `.fmt()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 32`** (2 nodes): `MainAgentWorker<'a, A>`, `.new()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 33`** (1 nodes): `private.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `MainAgent` connect `Community 0` to `Community 8`, `Community 1`?**
  _High betweenness centrality (0.098) - this node is a cross-community bridge._
- **Are the 26 inferred relationships involving `temp_root()` (e.g. with `agent_memory_paths_use_workspace_directory()` and `scaffold_claude_project_creates_system_prompt_file()`) actually correct?**
  _`temp_root()` has 26 INFERRED edges - model-reasoned connections that need verification._
- **Are the 19 inferred relationships involving `parse_command()` (e.g. with `parse_common_flags()` and `.new()`) actually correct?**
  _`parse_command()` has 19 INFERRED edges - model-reasoned connections that need verification._
- **Are the 13 inferred relationships involving `test_agent()` (e.g. with `agent_memory_paths()` and `.new()`) actually correct?**
  _`test_agent()` has 13 INFERRED edges - model-reasoned connections that need verification._
- **Are the 9 inferred relationships involving `lookup()` (e.g. with `.from_lookup()` and `langsmith_enabled()`) actually correct?**
  _`lookup()` has 9 INFERRED edges - model-reasoned connections that need verification._
- **What connects `ToolHandler`, `PerplexityChoice`, `PerplexityMessage` to the rest of the system?**
  _87 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06 - nodes in this community are weakly interconnected._