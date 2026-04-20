# Graph Report - .  (2026-04-20)

## Corpus Check
- 7 files · ~16,053 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 253 nodes · 414 edges · 22 communities detected
- Extraction: 63% EXTRACTED · 37% INFERRED · 0% AMBIGUOUS · INFERRED: 152 edges (avg confidence: 0.5)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 7|Community 7]]
- [[_COMMUNITY_Community 8|Community 8]]
- [[_COMMUNITY_Community 9|Community 9]]
- [[_COMMUNITY_Community 10|Community 10]]
- [[_COMMUNITY_Community 11|Community 11]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 16|Community 16]]
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Community 18|Community 18]]
- [[_COMMUNITY_Community 19|Community 19]]
- [[_COMMUNITY_Community 20|Community 20]]
- [[_COMMUNITY_Community 21|Community 21]]

## God Nodes (most connected - your core abstractions)
1. `MainAgent` - 28 edges
2. `Observability` - 13 edges
3. `lookup()` - 10 edges
4. `parse_command()` - 10 edges
5. `McpClient` - 9 edges
6. `Tool` - 8 edges
7. `suggest_relevant_files()` - 6 edges
8. `temp_root()` - 6 edges
9. `test_agent()` - 6 edges
10. `infer_tool_request()` - 5 edges

## Surprising Connections (you probably didn't know these)
- `suggest_relevant_files()` --calls--> `collect_candidate_files()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 0 → community 21_
- `temp_root()` --calls--> `load_memory_stack_follows_imports()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 6 → community 14_
- `temp_root()` --calls--> `project_subagent_overrides_user_subagent()`  [INFERRED]
  src/mainAgent.rs → src/mainAgent.rs  _Bridges community 6 → community 10_

## Communities

### Community 0 - "Community 0"
Cohesion: 0.1
Nodes (5): ConversationMessage, DelegationResult, MainAgent, SessionState, suggest_relevant_files()

### Community 1 - "Community 1"
Cohesion: 0.09
Nodes (18): drain_stderr(), empty_to_default(), format_mcp_tool_result(), formats_structured_mcp_tool_result(), load_mcp_catalog_from_env(), McpCallToolResult, McpCatalog, McpClient (+10 more)

### Community 2 - "Community 2"
Cohesion: 0.06
Nodes (23): AgentConfig, AnthropicContentBlock, ApiInputMessage, CommandOutcome, ContextPacket, CustomCommand, DelegationRequest, MemorySource (+15 more)

### Community 3 - "Community 3"
Cohesion: 0.18
Nodes (16): CliError, Command, defaults_to_session_when_no_args_are_provided(), help_flag_returns_general_help(), HelpTopic, main(), parse_command(), parse_common_flags() (+8 more)

### Community 4 - "Community 4"
Cohesion: 0.13
Nodes (11): AnthropicEngine, AnthropicResponse, AnthropicUsageBody, compact_history(), empty_to_default(), OpenAiEngine, OpenAiResponse, OpenAiUsageBody (+3 more)

### Community 5 - "Community 5"
Cohesion: 0.25
Nodes (15): build_langfuse_target(), build_langsmith_target(), build_provider(), builds_langfuse_target_with_basic_auth_header(), builds_langsmith_target_with_project_header(), compact_text(), compact_text_truncates_long_values(), env_flag() (+7 more)

### Community 6 - "Community 6"
Cohesion: 0.21
Nodes (7): AgentResult, empty_input_reports_noop_usage(), render_usage_summary(), render_usage_summary_lists_main_and_subagent_requests(), temp_root(), test_agent(), web_queries_use_web_search_path_without_explicit_tool_wrapper()

### Community 7 - "Community 7"
Cohesion: 0.23
Nodes (8): extract_web_search_query(), format_perplexity_response(), PerplexityChoice, PerplexityEngine, PerplexityMessage, PerplexityResponse, PerplexitySearchResult, tool_web_search_perplexity()

### Community 8 - "Community 8"
Cohesion: 0.2
Nodes (1): Observability

### Community 9 - "Community 9"
Cohesion: 0.18
Nodes (2): Tool, ToolHandler

### Community 10 - "Community 10"
Cohesion: 0.25
Nodes (9): file_stem_name(), load_command_dir(), load_custom_commands(), load_subagent_dir(), load_subagent_specs(), parse_tools_field(), project_subagent_overrides_user_subagent(), split_frontmatter() (+1 more)

### Community 11 - "Community 11"
Cohesion: 0.33
Nodes (6): heuristic_plan(), infer_subagent_name(), infer_tool_request(), infer_tool_request_routes_web_queries_to_web_search(), parse_explicit_tool_request(), should_use_web_search()

### Community 12 - "Community 12"
Cohesion: 0.4
Nodes (0): 

### Community 13 - "Community 13"
Cohesion: 0.5
Nodes (4): execute_explore_subagent(), execute_general_subagent(), execute_plan_subagent(), execute_subagent()

### Community 14 - "Community 14"
Cohesion: 0.5
Nodes (4): extract_imports(), load_memory_source(), load_memory_stack(), load_memory_stack_follows_imports()

### Community 15 - "Community 15"
Cohesion: 0.5
Nodes (1): TokenUsageRecord

### Community 16 - "Community 16"
Cohesion: 1.0
Nodes (1): DefinitionScope

### Community 17 - "Community 17"
Cohesion: 1.0
Nodes (2): expand_custom_command(), expand_custom_command_replaces_arguments()

### Community 18 - "Community 18"
Cohesion: 1.0
Nodes (1): MemoryScope

### Community 19 - "Community 19"
Cohesion: 1.0
Nodes (1): StepOutcome

### Community 20 - "Community 20"
Cohesion: 1.0
Nodes (1): Decision

### Community 21 - "Community 21"
Cohesion: 1.0
Nodes (2): collect_candidate_files(), collect_candidate_files_recursive()

## Knowledge Gaps
- **31 isolated node(s):** `ObservabilityTarget`, `AgentConfig`, `MessageRole`, `WaitModeConfig`, `MemorySource` (+26 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 16`** (2 nodes): `DefinitionScope`, `.fmt()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 17`** (2 nodes): `expand_custom_command()`, `expand_custom_command_replaces_arguments()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 18`** (2 nodes): `MemoryScope`, `.fmt()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 19`** (2 nodes): `StepOutcome`, `.fmt()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 20`** (2 nodes): `Decision`, `.fmt()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 21`** (2 nodes): `collect_candidate_files()`, `collect_candidate_files_recursive()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `MainAgent` connect `Community 0` to `Community 2`, `Community 6`?**
  _High betweenness centrality (0.079) - this node is a cross-community bridge._
- **Why does `TokenUsageRecord` connect `Community 15` to `Community 2`?**
  _High betweenness centrality (0.011) - this node is a cross-community bridge._
- **Why does `Observability` connect `Community 8` to `Community 5`?**
  _High betweenness centrality (0.008) - this node is a cross-community bridge._
- **Are the 9 inferred relationships involving `lookup()` (e.g. with `.from_lookup()` and `langsmith_enabled()`) actually correct?**
  _`lookup()` has 9 INFERRED edges - model-reasoned connections that need verification._
- **Are the 7 inferred relationships involving `parse_command()` (e.g. with `parse_common_flags()` and `.new()`) actually correct?**
  _`parse_command()` has 7 INFERRED edges - model-reasoned connections that need verification._
- **What connects `ObservabilityTarget`, `AgentConfig`, `MessageRole` to the rest of the system?**
  _31 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.1 - nodes in this community are weakly interconnected._