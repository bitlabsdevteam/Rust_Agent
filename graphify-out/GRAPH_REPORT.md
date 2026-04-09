# Graph Report - .  (2026-04-09)

## Corpus Check
- 15 files · ~0 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 360 nodes · 675 edges · 12 communities detected
- Extraction: 53% EXTRACTED · 47% INFERRED · 0% AMBIGUOUS · INFERRED: 315 edges (avg confidence: 0.5)
- Token cost: 0 input · 0 output

## God Nodes (most connected - your core abstractions)
1. `MainAgent` - 23 edges
2. `Observability` - 13 edges
3. `parse_ingress_request()` - 12 edges
4. `analyze_request()` - 10 edges
5. `parse_command()` - 10 edges
6. `decide_next_step_heuristic()` - 10 edges
7. `lookup()` - 10 edges
8. `run()` - 9 edges
9. `ensure_identity_profile()` - 9 edges
10. `McpClient` - 9 edges

## Surprising Connections (you probably didn't know these)
- `parse_streaming_openai_response()` --calls--> `process_stream_event()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 1 → community 8_

## Communities

### Community 0 - "Community 0"
Cohesion: 0.04
Nodes (61): AgentConfig, ApiInputMessage, build_planner_input(), build_root_span_attributes(), built_in_skills(), BuiltInSkill, collect_nested_files(), compact_text_preview() (+53 more)

### Community 1 - "Community 1"
Cohesion: 0.07
Nodes (19): AgentResult, AgentState, calls_tool_when_requested(), collect_reasoning_messages(), ConversationMessage, heuristic_explicit_tool_request_supports_json_arguments(), MainAgent, OpenAiEngine (+11 more)

### Community 2 - "Community 2"
Cohesion: 0.08
Nodes (44): analyze_request(), analyzes_multimodal_payload_for_routing(), append_conversation_history(), append_queue_entry(), appends_queue_entry_to_jsonl_file(), build_queue_entry(), build_summary(), classifies_support_payloads_and_sets_target_queue() (+36 more)

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (39): appends_identity_to_system_prompt(), build_system_prompt_with_identity(), ChatOptions, CliError, Command, defaults_to_chat_when_no_args_are_provided(), ensure_identity_profile(), examples_help_text() (+31 more)

### Community 4 - "Community 4"
Cohesion: 0.09
Nodes (18): drain_stderr(), empty_to_default(), format_mcp_tool_result(), formats_structured_mcp_tool_result(), load_mcp_catalog_from_env(), McpCallToolResult, McpCatalog, McpClient (+10 more)

### Community 5 - "Community 5"
Cohesion: 0.12
Nodes (16): build_langfuse_target(), build_langsmith_target(), build_provider(), builds_langfuse_target_with_basic_auth_header(), builds_langsmith_target_with_project_header(), compact_text(), compact_text_truncates_long_values(), env_flag() (+8 more)

### Community 6 - "Community 6"
Cohesion: 0.09
Nodes (10): Tool, ToolHandler, extract_web_search_query(), format_perplexity_response(), PerplexityChoice, PerplexityEngine, PerplexityMessage, PerplexityResponse (+2 more)

### Community 7 - "Community 7"
Cohesion: 0.14
Nodes (17): append_planner_queue_entry(), build_planner_queue_entry(), env_lock(), infer_planner_next_action(), load_agent_markdown_documents(), load_markdown_documents(), now_epoch_ms(), pending_queue_entries() (+9 more)

### Community 8 - "Community 8"
Cohesion: 0.38
Nodes (2): process_stream_event(), TerminalReasoningStreamer

### Community 9 - "Community 9"
Cohesion: 1.0
Nodes (0): 

### Community 10 - "Community 10"
Cohesion: 1.0
Nodes (0): 

### Community 11 - "Community 11"
Cohesion: 1.0
Nodes (0): 

## Knowledge Gaps
- **33 isolated node(s):** `ToolHandler`, `PerplexityChoice`, `PerplexityMessage`, `PerplexitySearchResult`, `IngressItem` (+28 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 9`** (2 nodes): `retry_once_agent.rs`, `run()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 10`** (2 nodes): `summarize_agent.rs`, `run()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 11`** (1 nodes): `private.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `MainAgent` connect `Community 1` to `Community 0`?**
  _High betweenness centrality (0.087) - this node is a cross-community bridge._
- **Are the 11 inferred relationships involving `parse_ingress_request()` (e.g. with `queue_ingress()` and `parse_request_value()`) actually correct?**
  _`parse_ingress_request()` has 11 INFERRED edges - model-reasoned connections that need verification._
- **Are the 9 inferred relationships involving `analyze_request()` (e.g. with `queue_ingress()` and `.as_str()`) actually correct?**
  _`analyze_request()` has 9 INFERRED edges - model-reasoned connections that need verification._
- **Are the 9 inferred relationships involving `parse_command()` (e.g. with `parse_common_flags()` and `.new()`) actually correct?**
  _`parse_command()` has 9 INFERRED edges - model-reasoned connections that need verification._
- **What connects `ToolHandler`, `PerplexityChoice`, `PerplexityMessage` to the rest of the system?**
  _33 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.04 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.07 - nodes in this community are weakly interconnected._