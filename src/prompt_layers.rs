#![allow(dead_code)]

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPromptLayer {
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMemoryLayer {
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskContractLayer {
    pub goal: String,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub relevant_files: Vec<String>,
    pub stop_condition: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSkillLayer {
    pub summary: String,
    pub instructions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationLayer {
    pub entries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMetadataLayer {
    pub channel_name: String,
    pub surface: String,
    pub delivery_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerPromptLayers {
    pub system: SystemPromptLayer,
    pub project_memory: ProjectMemoryLayer,
    pub task_contract: TaskContractLayer,
    pub available_skills: Vec<String>,
    pub active_skill: Option<ActiveSkillLayer>,
    pub observations: ObservationLayer,
    pub channel_metadata: ChannelMetadataLayer,
    pub tool_context: Vec<String>,
    pub subagent_context: Vec<String>,
    pub history_summary: String,
    pub retry_count: u8,
}

impl PlannerPromptLayers {
    pub fn render(&self) -> String {
        let tool_list = render_list(&self.tool_context, "- none loaded");
        let subagent_list = render_list(&self.subagent_context, "- none loaded");
        let available_skill_list = render_list(&self.available_skills, "- none loaded");
        let active_skill_summary = self
            .active_skill
            .as_ref()
            .map(|skill| skill.summary.clone())
            .unwrap_or_else(|| "none active".to_string());
        let active_skill_instructions = self
            .active_skill
            .as_ref()
            .map(|skill| render_list(&skill.instructions, "- none"))
            .unwrap_or_else(|| "- none".to_string());
        let observation_summary =
            render_list(&self.observations.entries, "No observations recorded yet.");
        let relevant_files = if self.task_contract.relevant_files.is_empty() {
            "none inferred".to_string()
        } else {
            self.task_contract.relevant_files.join(", ")
        };

        format!(
            "System instructions:\n{}\n\n\
Channel metadata:\n- channel: {}\n- surface: {}\n- delivery mode: {}\n\n\
Project memory:\n{}\n\n\
Available tools:\n{}\n\n\
Available subagents:\n{}\n\n\
Available skills:\n{}\n\n\
Active skill:\n- summary: {}\n- instructions:\n{}\n\n\
Task contract:\n- goal: {}\n- constraints: {}\n- acceptance criteria: {}\n- relevant files: {}\n- stop condition: {}\n\n\
Retry count: {}\n\n\
Recorded observations:\n{}\n\n\
Compacted prior history:\n{}\n",
            self.system.instructions,
            self.channel_metadata.channel_name,
            self.channel_metadata.surface,
            self.channel_metadata.delivery_mode,
            self.project_memory.summary,
            tool_list,
            subagent_list,
            available_skill_list,
            active_skill_summary,
            active_skill_instructions,
            self.task_contract.goal,
            join_or_default(&self.task_contract.constraints, "none"),
            join_or_default(&self.task_contract.acceptance_criteria, "none"),
            relevant_files,
            self.task_contract.stop_condition,
            self.retry_count,
            observation_summary,
            self.history_summary,
        )
    }
}

fn join_or_default(items: &[String], fallback: &str) -> String {
    if items.is_empty() {
        fallback.to_string()
    } else {
        items.join(" | ")
    }
}

fn render_list(items: &[String], empty_message: &str) -> String {
    if items.is_empty() {
        empty_message.to_string()
    } else {
        items.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_prompt_layers_render_explicit_sections_in_stable_order() {
        let layers = PlannerPromptLayers {
            system: SystemPromptLayer {
                instructions: "Plan exactly one next action.".to_string(),
            },
            project_memory: ProjectMemoryLayer {
                summary: "Project memory summary".to_string(),
            },
            task_contract: TaskContractLayer {
                goal: "Implement prompt layers".to_string(),
                constraints: vec!["Keep the change small".to_string()],
                acceptance_criteria: vec!["Expose explicit prompt sections".to_string()],
                relevant_files: vec!["src/mainAgent.rs".to_string()],
                stop_condition: "Return the next bounded action.".to_string(),
            },
            available_skills: vec!["- developer: reusable coding workflow".to_string()],
            active_skill: Some(ActiveSkillLayer {
                summary: "developer [project] activated by planner".to_string(),
                instructions: vec!["- keep the change small".to_string()],
            }),
            observations: ObservationLayer {
                entries: vec!["latest observation".to_string()],
            },
            channel_metadata: ChannelMetadataLayer {
                channel_name: "cli".to_string(),
                surface: "terminal".to_string(),
                delivery_mode: "local-session".to_string(),
            },
            tool_context: vec!["- web_search_tool: query the web".to_string()],
            subagent_context: vec!["- plan: produce a compact plan".to_string()],
            history_summary: "No prior session history was available.".to_string(),
            retry_count: 1,
        };

        let prompt = layers.render();

        assert!(prompt.contains("System instructions:"));
        assert!(prompt.contains("Channel metadata:"));
        assert!(prompt.contains("Project memory:"));
        assert!(prompt.contains("Available skills:"));
        assert!(prompt.contains("Active skill:"));
        assert!(prompt.contains("Task contract:"));
        assert!(prompt.contains("Recorded observations:"));
        assert!(prompt.contains("- channel: cli"));
        assert!(prompt.contains("- goal: Implement prompt layers"));

        let system_index = prompt.find("System instructions:").expect("system section");
        let channel_index = prompt.find("Channel metadata:").expect("channel section");
        let memory_index = prompt.find("Project memory:").expect("memory section");
        let task_index = prompt.find("Task contract:").expect("task section");

        assert!(system_index < channel_index);
        assert!(channel_index < memory_index);
        assert!(memory_index < task_index);
    }
}
