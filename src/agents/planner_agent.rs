use crate::agents::ingress_agent::{read_queue_entries_lossy, resolve_queue_path, QueueEntry};
use crate::mainAgent::StepOutcome;
use crate::runtime_log;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const WORKSPACE_CONTEXT_DIR: &str = "Workspace";
const REPO_CONTEXT_FILES: &[&str] = &["AGENTS.md", "TDD.md"];
const DEFAULT_PLANNER_QUEUE_FILE: &str = "logs/planner_queue.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannerQueueEntry {
    pub planner_id: String,
    pub source_queue_id: String,
    pub planned_at_epoch_ms: u128,
    pub status: String,
    pub source_queue: String,
    pub target_queue: String,
    pub next_step: String,
    pub rationale: String,
}

pub(crate) fn run(user_input: &str) -> StepOutcome {
    runtime_log::info("planner_agent", "planner skill run started");
    let documents = match load_agent_markdown_documents() {
        Ok(documents) => documents,
        Err(reason) => {
            runtime_log::error(
                "planner_agent",
                format!("failed to load markdown context: {reason}"),
            );
            return StepOutcome::Retry(format!(
                "Planner skill could not load markdown context: {reason}"
            ));
        }
    };
    let queue_path = resolve_queue_path();
    let (entries, warnings) = match read_queue_entries_lossy(&queue_path) {
        Ok(result) => result,
        Err(reason) => {
            runtime_log::error(
                "planner_agent",
                format!(
                    "failed to read ingress queue {}: {reason}",
                    queue_path.display()
                ),
            );
            return StepOutcome::Retry(format!(
                "Planner skill could not read queue file {}: {}",
                queue_path.display(),
                reason
            ));
        }
    };
    let planner_queue_path = resolve_planner_queue_path();
    let (existing_plans, planner_queue_warnings) =
        match read_planner_queue_entries_lossy(&planner_queue_path) {
            Ok(result) => result,
            Err(reason) => {
                runtime_log::error(
                    "planner_agent",
                    format!(
                        "failed to read planner queue {}: {reason}",
                        planner_queue_path.display()
                    ),
                );
                return StepOutcome::Retry(format!(
                    "Planner skill could not read planner queue file {}: {}",
                    planner_queue_path.display(),
                    reason
                ));
            }
        };

    let planned_queue_ids = existing_plans
        .iter()
        .map(|entry| entry.source_queue_id.clone())
        .collect::<BTreeSet<_>>();
    let pending_entries = entries
        .iter()
        .filter(|entry| !planned_queue_ids.contains(&entry.queue_id))
        .collect::<Vec<_>>();
    let latest_entry_summary = entries
        .last()
        .map(latest_queue_entry_summary)
        .unwrap_or_else(|| "No queued items were found.".to_string());
    let markdown_summary = summarize_markdown_documents(&documents).join("\n- ");
    let queue_warning_summary = render_warning_section("Queue warnings", &warnings);
    let planner_warning_summary =
        render_warning_section("Planner queue warnings", &planner_queue_warnings);
    let request_focus = if user_input.trim().is_empty() {
        "No extra planner instruction was provided.".to_string()
    } else {
        user_input.trim().to_string()
    };

    runtime_log::info(
        "planner_agent",
        format!(
            "loaded {} ingress item(s), {} existing plan(s), {} pending item(s)",
            entries.len(),
            existing_plans.len(),
            pending_entries.len()
        ),
    );

    let mut created_plans = Vec::new();
    for entry in &pending_entries {
        let plan = build_planner_queue_entry(entry, user_input);
        if let Err(reason) = append_planner_queue_entry(&planner_queue_path, &plan) {
            runtime_log::error(
                "planner_agent",
                format!(
                    "failed to append planner queue entry to {}: {reason}",
                    planner_queue_path.display()
                ),
            );
            return StepOutcome::Retry(format!(
                "Planner skill could not append planner queue file {}: {}",
                planner_queue_path.display(),
                reason
            ));
        }
        runtime_log::info(
            "planner_agent",
            format!(
                "planned queue_id {} with next step {}",
                entry.queue_id, plan.next_step
            ),
        );
        created_plans.push(plan);
    }

    let planner_summary = if created_plans.is_empty() {
        "No unplanned ingress items were found.".to_string()
    } else {
        created_plans
            .iter()
            .map(render_planner_queue_entry_summary)
            .collect::<Vec<_>>()
            .join("\n")
    };

    StepOutcome::Success(format!(
        "Planner agent analysis\n\
Request focus: {}\n\
Ingress queue file: {}\n\
Queued entries: {}\n\
Planner queue file: {}\n\
Pending entries processed: {}\n\
Latest queue item: {}\n\
\n\
Context files reviewed:\n\
- {}\n\
{}{}{}\
\n\
Task contract\n\
- goal: read ingress queue items, decide the next explicit processing action, and persist that action in the planner queue\n\
- constraints: follow repo-local markdown guidance, keep the loop legible, call one tool or one skill per step, and stop instead of looping after 3 retries\n\
- acceptance_criteria: each unplanned ingress item receives one planner queue entry with a clear next step\n\
- relevant_files: {}, {}, {}, {}\n\
- stop_condition: stop once every queued ingress item has a corresponding planner queue step or there is no queued work to plan\n\
\n\
Planner queue updates:\n{}",
        request_focus,
        queue_path.display(),
        entries.len(),
        planner_queue_path.display(),
        pending_entries.len(),
        latest_entry_summary,
        if markdown_summary.is_empty() {
            "No markdown files were available.".to_string()
        } else {
            markdown_summary
        },
        queue_warning_summary,
        planner_warning_summary,
        queue_path.display(),
        planner_queue_path.display(),
        "AGENTS.md",
        resolve_workspace_context_dir().display(),
        crate::agents::ingress_agent::resolve_conversation_history_path().display(),
        planner_summary
    ))
}

pub fn pending_queue_entries() -> Result<Vec<QueueEntry>, String> {
    let queue_path = resolve_queue_path();
    let planner_queue_path = resolve_planner_queue_path();
    let (entries, _) = read_queue_entries_lossy(&queue_path)?;
    let (existing_plans, _) = read_planner_queue_entries_lossy(&planner_queue_path)?;

    let planned_queue_ids = existing_plans
        .iter()
        .map(|entry| entry.source_queue_id.clone())
        .collect::<BTreeSet<_>>();

    Ok(entries
        .into_iter()
        .filter(|entry| !planned_queue_ids.contains(&entry.queue_id))
        .collect())
}

pub fn resolve_planner_queue_path() -> PathBuf {
    env::var("AGENT_PLANNER_QUEUE_FILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PLANNER_QUEUE_FILE))
}

fn resolve_workspace_context_dir() -> PathBuf {
    env::var("AGENT_WORKSPACE_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(WORKSPACE_CONTEXT_DIR))
}

fn load_markdown_documents(workspace_dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut documents = Vec::new();

    for relative_path in REPO_CONTEXT_FILES {
        let path = PathBuf::from(relative_path);
        if !path.is_file() {
            continue;
        }
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("read {} failed: {error}", path.display()))?;
        documents.push((path, contents));
    }

    if workspace_dir.is_dir() {
        let mut workspace_paths = fs::read_dir(workspace_dir)
            .map_err(|error| format!("could not read {}: {error}", workspace_dir.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .map(|extension| extension.eq_ignore_ascii_case("md"))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        workspace_paths.sort();

        for path in workspace_paths {
            let contents = fs::read_to_string(&path)
                .map_err(|error| format!("read {} failed: {error}", path.display()))?;
            documents.push((path, contents));
        }
    }

    Ok(documents)
}

fn load_agent_markdown_documents() -> Result<Vec<(PathBuf, String)>, String> {
    load_markdown_documents(&resolve_workspace_context_dir())
}

fn summarize_markdown_documents(documents: &[(PathBuf, String)]) -> Vec<String> {
    documents
        .iter()
        .map(|(path, contents)| {
            let heading = contents
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("(empty)");
            format!("{} -> {}", path.display(), heading)
        })
        .collect()
}

fn latest_queue_entry_summary(entry: &QueueEntry) -> String {
    let classification = &entry.analysis.classification;
    format!(
        "queue_id=`{}` source=`{}` target_queue=`{}` priority=`{}` routing_hint=`{}` summary=\"{}\"",
        entry.queue_id,
        entry.source,
        classification.target_queue,
        classification.priority,
        entry.analysis.routing_hint,
        entry.analysis.summary
    )
}

fn build_planner_queue_entry(entry: &QueueEntry, user_input: &str) -> PlannerQueueEntry {
    let planned_at_epoch_ms = now_epoch_ms();
    PlannerQueueEntry {
        planner_id: format!("planner-{planned_at_epoch_ms}"),
        source_queue_id: entry.queue_id.clone(),
        planned_at_epoch_ms,
        status: "planned".to_string(),
        source_queue: resolve_queue_path().display().to_string(),
        target_queue: entry.analysis.classification.target_queue.clone(),
        next_step: infer_planner_next_action(entry, user_input),
        rationale: entry.analysis.classification.rationale.clone(),
    }
}

fn infer_planner_next_action(entry: &QueueEntry, user_input: &str) -> String {
    let mut signal_text = user_input.to_ascii_lowercase();

    for item in &entry.items {
        if let Some(text) = &item.text {
            signal_text.push(' ');
            signal_text.push_str(&text.to_ascii_lowercase());
        }
        if let Some(transcript) = &item.transcript {
            signal_text.push(' ');
            signal_text.push_str(&transcript.to_ascii_lowercase());
        }
        if let Some(notes) = &item.notes {
            signal_text.push(' ');
            signal_text.push_str(&notes.to_ascii_lowercase());
        }
    }

    if ["weather", "latest", "today", "news", "current"]
        .iter()
        .any(|needle| signal_text.contains(needle))
    {
        "call tool `web_search` because the queued request is time-sensitive and should be grounded"
            .to_string()
    } else if signal_text.contains("summary") || signal_text.contains("summarize") {
        "call skill `summarize` to compress the queued request before downstream handling"
            .to_string()
    } else if let Some(context_packet) = entry
        .metadata
        .get("ingress_context")
        .and_then(|value| value.get("context_packet"))
        .and_then(|value| value.as_str())
    {
        format!(
            "review the ingress context packet, then process the queued item in `{}` with a single explicit tool-or-skill step. Context digest: {}",
            entry.analysis.classification.target_queue,
            shorten_with_limit(context_packet, 240)
        )
    } else {
        format!(
            "process the queued item in `{}` and keep the run to one explicit tool or skill decision",
            entry.analysis.classification.target_queue
        )
    }
}

fn render_planner_queue_entry_summary(entry: &PlannerQueueEntry) -> String {
    format!(
        "- planner_id=`{}` source_queue_id=`{}` target_queue=`{}` next_step=\"{}\"",
        entry.planner_id, entry.source_queue_id, entry.target_queue, entry.next_step
    )
}

fn append_planner_queue_entry(path: &Path, entry: &PlannerQueueEntry) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create planner queue directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "failed to open planner queue file {}: {error}",
                path.display()
            )
        })?;

    let encoded = serde_json::to_string(entry)
        .map_err(|error| format!("failed to encode planner queue entry: {error}"))?;
    file.write_all(encoded.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|error| {
            format!(
                "failed to append planner queue entry to {}: {error}",
                path.display()
            )
        })
}

fn read_planner_queue_entries_lossy(
    path: &Path,
) -> Result<(Vec<PlannerQueueEntry>, Vec<String>), String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(error) => {
            return Err(format!(
                "failed to read planner queue file {}: {error}",
                path.display()
            ));
        }
    };

    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for (line_number, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<PlannerQueueEntry>(trimmed) {
            Ok(entry) => entries.push(entry),
            Err(error) => warnings.push(format!(
                "skipping malformed planner queue entry at {} line {}: {error}",
                path.display(),
                line_number + 1
            )),
        }
    }

    Ok((entries, warnings))
}

fn render_warning_section(label: &str, warnings: &[String]) -> String {
    if warnings.is_empty() {
        String::new()
    } else {
        format!("\n{label}:\n- {}\n", warnings.join("\n- "))
    }
}

fn shorten_with_limit(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    let shortened = trimmed.chars().take(limit).collect::<String>();
    if trimmed.chars().count() > limit {
        format!("{shortened}...")
    } else {
        shortened
    }
}

fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::ingress_agent::{
        analyze_request, append_queue_entry, build_queue_entry, parse_ingress_request,
    };
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn planner_writes_next_step_for_unplanned_queue_items() {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let temp_root = std::env::temp_dir().join(format!("planner-queue-test-{}", now_epoch_ms()));
        let workspace_dir = temp_root.join("workspace");
        let ingress_queue_path = temp_root.join("ingress.jsonl");
        let planner_queue_path = temp_root.join("planner.jsonl");
        fs::create_dir_all(&workspace_dir).expect("workspace dir should exist");
        fs::write(
            workspace_dir.join("MEMORY.md"),
            "# MEMORY\n\n- durable fact\n",
        )
        .expect("memory file should be written");

        std::env::set_var("AGENT_WORKSPACE_DIR", &workspace_dir);
        std::env::set_var("AGENT_QUEUE_FILE", &ingress_queue_path);
        std::env::set_var("AGENT_PLANNER_QUEUE_FILE", &planner_queue_path);

        let request =
            parse_ingress_request("Summarize this support request.", &serde_json::Value::Null)
                .expect("request should parse");
        let entry = build_queue_entry(&request, analyze_request(&request));
        append_queue_entry(&ingress_queue_path, &entry)
            .expect("ingress queue write should succeed");

        let outcome = run("No extra instruction.");
        match outcome {
            StepOutcome::Success(output) => {
                assert!(output.contains("Pending entries processed: 1"));
                assert!(output.contains("planner.jsonl"));
            }
            StepOutcome::Retry(reason) => panic!("planner run should succeed: {reason}"),
        }

        let (planner_entries, warnings) = read_planner_queue_entries_lossy(&planner_queue_path)
            .expect("planner queue should load");
        assert!(warnings.is_empty());
        assert_eq!(planner_entries.len(), 1);
        assert_eq!(planner_entries[0].source_queue_id, entry.queue_id);

        let _ = fs::remove_file(&ingress_queue_path);
        let _ = fs::remove_file(&planner_queue_path);
        let _ = fs::remove_file(workspace_dir.join("MEMORY.md"));
        let _ = fs::remove_dir(&workspace_dir);
        let _ = fs::remove_dir(&temp_root);
        std::env::remove_var("AGENT_WORKSPACE_DIR");
        std::env::remove_var("AGENT_QUEUE_FILE");
        std::env::remove_var("AGENT_PLANNER_QUEUE_FILE");
    }

    #[test]
    fn pending_queue_entries_excludes_already_planned_items() {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let temp_root =
            std::env::temp_dir().join(format!("planner-pending-test-{}", now_epoch_ms()));
        let workspace_dir = temp_root.join("workspace");
        let ingress_queue_path = temp_root.join("ingress.jsonl");
        let planner_queue_path = temp_root.join("planner.jsonl");
        fs::create_dir_all(&workspace_dir).expect("workspace dir should exist");

        std::env::set_var("AGENT_WORKSPACE_DIR", &workspace_dir);
        std::env::set_var("AGENT_QUEUE_FILE", &ingress_queue_path);
        std::env::set_var("AGENT_PLANNER_QUEUE_FILE", &planner_queue_path);

        let request =
            parse_ingress_request("Summarize this support request.", &serde_json::Value::Null)
                .expect("request should parse");
        let first_entry = build_queue_entry(&request, analyze_request(&request));
        append_queue_entry(&ingress_queue_path, &first_entry)
            .expect("first ingress queue write should succeed");

        let second_request = parse_ingress_request(
            "Check the latest weather for this shipment route.",
            &serde_json::Value::Null,
        )
        .expect("second request should parse");
        let second_entry = build_queue_entry(&second_request, analyze_request(&second_request));
        append_queue_entry(&ingress_queue_path, &second_entry)
            .expect("second ingress queue write should succeed");

        append_planner_queue_entry(
            &planner_queue_path,
            &build_planner_queue_entry(&first_entry, "No extra instruction."),
        )
        .expect("planner queue write should succeed");

        let pending = pending_queue_entries().expect("pending queue entries should load");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].queue_id, second_entry.queue_id);

        let _ = fs::remove_file(&ingress_queue_path);
        let _ = fs::remove_file(&planner_queue_path);
        let _ = fs::remove_dir(&workspace_dir);
        let _ = fs::remove_dir(&temp_root);
        std::env::remove_var("AGENT_WORKSPACE_DIR");
        std::env::remove_var("AGENT_QUEUE_FILE");
        std::env::remove_var("AGENT_PLANNER_QUEUE_FILE");
    }
}
