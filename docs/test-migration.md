# Legacy test migration audit

This audit maps every static `test(...)` declaration formerly present in
`src/**/*.test.ts` to a concrete Rust or workflow-host test. The legacy
inventory contains 74 tests across 25 files.

`Mapped` means the named test asserts the same material behavior. Multiple
legacy rows may map to one parity test when that test deliberately covers the
combined scenario. No row is credited merely because the implementation
exists.

## Summary

| Behavior group | Legacy tests | Mapped | Gaps |
| --- | ---: | ---: | ---: |
| Configuration and setup | 5 | 5 | 0 |
| Input history and CLI interaction | 18 | 18 | 0 |
| Agents, providers, permissions, and tools | 25 | 25 | 0 |
| Workflows and durable execution | 18 | 18 | 0 |
| Scheduling | 8 | 8 | 0 |
| **Total** | **74** | **74** | **0** |

The workflow command cancellation and descendant-process assertions map to a
Rust `#[cfg(unix)]` test. The legacy descendant test was also skipped on
Windows; the Rust suite does not provide Windows descendant-tree coverage.

## Configuration and setup

| Legacy test | Status | Rust/workflow-host test |
| --- | --- | --- |
| `ModelSetupService` — creates the first provider and model with validated answers | **Mapped** | `flowmation-cli/src/model_setup.rs::creates_first_model_from_validated_answers` |
| `ModelSetupService` — cancels setup when input closes | **Mapped** | `flowmation-cli/src/model_setup.rs::cancels_when_input_closes` |
| `ConfigService` — loads the first-run scaffold without requiring a model | **Mapped** | `flowmation-application/src/config.rs::loads_first_run_scaffold_without_a_model` |
| `ConfigService` — saves a model setup as the active global model | **Mapped** | `flowmation-application/src/config.rs::saves_model_setup_as_active_global_model` |
| `ConfigService` — merges project model aliases and validates their targets | **Mapped** | `flowmation-domain/tests/config_legacy.rs::merges_project_model_aliases_and_validates_targets` |

## Input history and CLI interaction

| Legacy test | Status | Rust/workflow-host test |
| --- | --- | --- |
| `InputHistoryStore` — returns empty history when no persisted file exists | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::returns_empty_history_when_no_file_exists` |
| `InputHistoryStore` — persists and reloads input history | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::persists_and_reloads_input_history` |
| `InputHistoryStore` — preserves entries appended concurrently by separate stores | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::preserves_entries_appended_by_concurrent_stores` |
| `InputHistoryStore` — reclaims abandoned lock entries without disturbing concurrent writers | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::reclaims_abandoned_lock_entries_without_disturbing_concurrent_writers` |
| `InputHistoryStore` — retains only the configured number of entries | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::persisted_history_retains_only_configured_limit` |
| `InputHistoryStore` — rejects malformed persisted history | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::rejects_malformed_persisted_history` |
| `InputHistory` — navigates from newest input to oldest input | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::navigates_history_and_restores_current_draft` |
| `InputHistory` — navigates forward and restores the current draft | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::navigates_history_and_restores_current_draft` |
| `InputHistory` — ignores empty input | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::ignores_empty_input_and_retains_configured_limit` |
| `InputHistory` — loads existing entries and retains only the configured limit | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::ignores_empty_input_and_retains_configured_limit` |
| `InputHistory` — rejects invalid history limits | **Mapped** | `flowmation-domain/tests/input_history_legacy.rs::rejects_invalid_history_limits` |
| `lineEditor` — uses arrow keys to browse history and restore a draft | **Mapped** | `flowmation-cli/src/line_editor.rs::browses_history_and_restores_draft` |
| `lineEditor` — clears every previously rendered row when input wraps | **Mapped** | `flowmation-cli/src/line_editor.rs::clears_every_previously_rendered_row_when_input_wraps` |
| `lineEditor` — clears entered text on Ctrl+C without closing the prompt | **Mapped** | `flowmation-cli/src/line_editor.rs::clears_text_then_requires_two_interrupts_to_exit` |
| `lineEditor` — requires Ctrl+C twice to close an empty prompt | **Mapped** | `flowmation-cli/src/line_editor.rs::clears_text_then_requires_two_interrupts_to_exit` |
| `lineEditor` — delegates Ctrl+C to a foreground operation | **Mapped** | `flowmation-cli/src/line_editor.rs::delegates_interrupt_to_foreground_operation` |
| `CliPermissionController` — serializes concurrent permission confirmations | **Mapped** | `flowmation-cli/src/permission_prompt.rs::serializes_concurrent_permission_confirmations` |
| `WorkflowCliController` — includes workflow input in agent-invocation confirmations | **Mapped** | `flowmation-application/src/workflow_tool.rs::runs_eligible_workflows_and_confirms_with_input_details` |

## Agents, providers, permissions, and tools

| Legacy test | Status | Rust/workflow-host test |
| --- | --- | --- |
| `AgentComsService` — omits and ignores tools when an agent run disables them | **Mapped** | `flowmation-application/src/agent.rs::omits_and_ignores_tools_when_disabled` |
| `AgentComsService` — keeps tools enabled by default | **Mapped** | `flowmation-application/src/agent.rs::keeps_tools_enabled_and_omits_thinking_by_default` |
| `AgentComsService` — applies thinking to every request in a tool loop and retains history | **Mapped** | `flowmation-application/src/agent.rs::thinking_applies_to_every_request_in_tool_loop` |
| `AgentComsService` — retains thinking in tool-free history | **Mapped** | `flowmation-application/src/agent.rs::tool_free_history_retains_thinking_and_strips_tool_calls` |
| `AgentComsService` — omits thinking from provider options by default | **Mapped** | `flowmation-application/src/agent.rs::keeps_tools_enabled_and_omits_thinking_by_default` |
| `AgentComsService` — clears conversation context while restoring static system context | **Mapped** | `flowmation-application/src/agent.rs::clear_restores_static_system_context` |
| `AgentComsService` — aborts an active tool and skips remaining tool calls | **Mapped** | `flowmation-application/src/agent.rs::cancellation_aborts_active_tool_and_skips_remaining_calls` |
| `AgentComsService` — automatically allows reads and denies scheduled effects | **Mapped** | `flowmation-application/src/policy.rs::automatically_allows_reads_and_denies_scheduled_effects` |
| `AgentComsService` — lets self-managed tools authorize themselves outside scheduled runs | **Mapped** | `flowmation-application/src/policy.rs::self_managed_tools_are_allowed_outside_scheduled_runs` |
| `AgentManager` — switches project-scoped conversations and persists per-agent models | **Mapped** | `flowmation-application/src/manager.rs::switches_project_conversations_and_persists_per_agent_models` |
| `AgentManager` — execution managers do not overwrite direct conversations | **Mapped** | `flowmation-application/src/manager.rs::workflow_sessions_do_not_overwrite_direct_conversations` |
| `AgentConversationStore` — persists separate project-scoped conversations without system messages | **Mapped** | `flowmation-sqlite/tests/application_adapter.rs::conversation_trait_preserves_isolation_and_filters_system_messages` |
| `AgentPackageRegistry` — project packages replace global packages atomically | **Mapped** | `flowmation-application/src/registry.rs::project_agent_packages_replace_global_packages_atomically` |
| `AgentPackageRegistry` — fingerprints change when any package context file changes | **Mapped** | `flowmation-application/src/registry.rs::fingerprint_changes_when_package_context_changes` |
| `AgentPackageRegistry` — invalid manifests and missing required files are rejected | **Mapped** | `flowmation-application/src/registry.rs::invalid_names_and_missing_required_files_are_rejected` |
| `AgentPackageRegistry` — an invalid project package does not fall back to global | **Mapped** | `flowmation-application/src/registry.rs::invalid_project_package_does_not_fall_back_to_global` |
| `AgentPackageRegistry` — rejects symbolic links anywhere in a package | **Mapped** | `flowmation-domain/src/fingerprint.rs::rejects_symbolic_links_anywhere_in_directory` |
| `Agent` — presents workflow results in an isolated tool-free session | **Mapped** | `flowmation-application/src/manager.rs::workflow_results_use_an_isolated_tool_free_session` |
| `Agent` — switches through an alias that matches the active model name | **Mapped** | `flowmation-application/src/manager.rs::model_alias_can_match_the_active_model_name` |
| `AgentSession` — forwards thinking per run without making it sticky | **Mapped** | `flowmation-application/src/agent.rs::session_thinking_override_is_not_sticky` |
| `AgentSession` — copied session history retains prior thinking | **Mapped** | `flowmation-application/src/agent.rs::copied_session_history_retains_prior_thinking` |
| `OllamaProvider` — maps thinking modes to Ollama's top-level `think` field | **Mapped** | `flowmation-ollama/src/lib.rs::maps_thinking_modes_to_top_level_think_field` |
| `OllamaProvider` — retains response thinking and replays historical thinking | **Mapped** | `flowmation-ollama/src/lib.rs::retains_response_and_historical_thinking` |
| `runWorkflow` — runs eligible workflows and confirms when required | **Mapped** | `flowmation-application/src/workflow_tool.rs::runs_eligible_workflows_and_confirms_with_input_details` |
| `runWorkflow` — uses the current workflow policy when executing a cached tool | **Mapped** | `flowmation-application/src/workflow_tool.rs::cached_tool_uses_current_workflow_policy` |

## Workflows and durable execution

| Legacy test | Status | Rust/workflow-host test |
| --- | --- | --- |
| `WorkflowRegistry` — loads JavaScript and TypeScript through the virtual SDK | **Mapped** | `workflow-host/test/host.test.ts::handshakes and executes a workflow through a nested Rust callback`; `::loads TypeScript through the virtual SDK and refreshes a portable editor path` |
| `WorkflowRegistry` — project workflows override global and ambiguous entries are skipped | **Mapped** | `flowmation-application/src/workflow_tests.rs::project_workflows_override_global_and_ambiguous_entries_are_skipped` |
| `WorkflowRegistry` — fingerprints include every file in the directory | **Mapped** | `flowmation-application/src/workflow_tests.rs::workflow_fingerprint_changes_when_a_helper_changes` |
| `WorkflowRegistry` — keeps project workflow SDK paths portable and refreshed | **Mapped** | `workflow-host/test/host.test.ts::loads TypeScript through the virtual SDK and refreshes a portable editor path` |
| `WorkflowProcess` — cancellation stops an active command | **Mapped (Unix)** | `flowmation-application/src/workflow_tests.rs::exec_cancellation_terminates_descendant_processes` |
| `WorkflowProcess` — cancellation terminates descendant processes | **Mapped (Unix)** | `flowmation-application/src/workflow_tests.rs::exec_cancellation_terminates_descendant_processes` |
| `WorkflowRunStore` — migrates existing runs to agent and trigger metadata | **Mapped** | `flowmation-sqlite/tests/legacy_migration.rs::migrates_typescript_workflow_runs_without_rewriting_data` |
| `WorkflowEngine` — checkpoints and human responses survive a resumed run | **Mapped** | `flowmation-application/src/workflow_tests.rs::runner_resume_reuses_checkpoint_and_human_but_reruns_ordinary_agent_calls` |
| `WorkflowEngine` — ordinary agent calls rerun unless wrapped in a checkpoint | **Mapped** | `flowmation-application/src/workflow_tests.rs::runner_resume_reuses_checkpoint_and_human_but_reruns_ordinary_agent_calls` |
| `WorkflowEngine` — forwards agent thinking options to the runtime session | **Mapped** | `flowmation-application/src/workflow_tests.rs::agent_callback_forwards_thinking_and_tools_modes` |
| `WorkflowEngine` — scopes elevation thinking to runs inside the operation | **Mapped** | `flowmation-application/src/workflow_tests.rs::elevation_thinking_is_scoped_to_operation_session_runs` |
| `WorkflowEngine` — completed effects are reused when a run resumes | **Mapped** | `flowmation-application/src/workflow_tests.rs::completed_checkpoint_and_effect_values_are_reused` |
| `WorkflowEngine` — resume rejects changes in the workflow directory | **Mapped** | `flowmation-application/src/workflow_tests.rs::runner_resume_rejects_changed_source_and_records_version_mismatch` |
| `WorkflowEngine` — map limits concurrency and preserves result order | **Mapped** | `flowmation-application/src/workflow_tests.rs::map_limits_concurrency_and_preserves_input_order` |
| `WorkflowEngine` — exec captures output and rejects failed commands | **Mapped (Unix)** | `flowmation-application/src/workflow_tests.rs::exec_captures_io_environment_and_failure_status` |
| workflow types — accepts every workflow thinking mode | **Mapped** | `flowmation-domain/src/chat.rs::accepts_every_workflow_thinking_mode` |
| `SerializedWorkflowHumanAdapter` — serializes concurrent human requests | **Mapped** | `flowmation-application/src/workflow_tests.rs::concurrent_human_requests_are_serialized` |
| `SerializedWorkflowHumanAdapter` — continues the queue after a rejected prompt | **Mapped** | `flowmation-application/src/workflow_tests.rs::human_request_queue_continues_after_a_rejected_prompt` |

## Scheduling

| Legacy test | Status | Rust/workflow-host test |
| --- | --- | --- |
| `CronExpression` — parses five-field cron and advances in an IANA timezone | **Mapped** | `flowmation-domain/src/cron.rs::parses_five_field_cron_and_advances_in_iana_timezone` |
| `CronExpression` — skips nonexistent local times during DST transitions | **Mapped** | `flowmation-domain/src/cron.rs::skips_nonexistent_local_times_during_dst_transition` |
| `CronExpression` — retains both repeated local times when DST ends | **Mapped** | `flowmation-domain/src/cron.rs::retains_both_repeated_local_times_when_dst_ends` |
| `CronExpression` — rejects malformed expressions and unknown timezones | **Mapped** | `flowmation-domain/src/cron.rs::rejects_malformed_cron_and_unknown_timezones` |
| `ScheduleStore` — stores schedules, unique occurrences, and renewable leases | **Mapped** | `flowmation-sqlite/tests/fresh_database.rs::stores_unique_occurrences_and_renewable_worker_leases` |
| `ScheduleService` — reauthorizes the exact prospective fingerprint shown | **Mapped** | `flowmation-application/tests/scheduling_parity.rs::reauthorizes_the_exact_prospective_fingerprint_shown_for_approval` |
| `ScheduleWorker` — runs one catch-up occurrence and records its scheduled trigger | **Mapped** | `flowmation-sqlite/tests/scheduling_worker.rs::runs_one_catch_up_occurrence_records_trigger_and_recovers` |
| `ScheduleWorker` — rejects changed source before evaluating its module | **Mapped** | `flowmation-sqlite/tests/scheduling_worker.rs::rejects_changed_source_before_evaluating_and_persists_invalidation` |
