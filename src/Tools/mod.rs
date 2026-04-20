mod web_search_tool_perplexity;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use web_search_tool_perplexity::{
    extract_web_search_query, format_perplexity_response, PerplexityResponse,
};

use crate::mainAgent::StepOutcome;
use crate::mcp::McpToolRegistration;
use serde_json::Value;
use web_search_tool_perplexity::tool_web_search_perplexity;

type BuiltInToolHandler = fn(&str, &Value) -> StepOutcome;

pub struct Tool {
    name: String,
    description: String,
    planner_description: String,
    handler: ToolHandler,
}

impl Tool {
    fn built_in(
        name: impl Into<String>,
        description: impl Into<String>,
        handler: BuiltInToolHandler,
    ) -> Self {
        let name = name.into();
        let description = description.into();
        Self {
            name: name.clone(),
            planner_description: format!("{}: {}", name, description),
            description,
            handler: ToolHandler::BuiltIn(handler),
        }
    }

    pub fn from_mcp(registration: McpToolRegistration) -> Self {
        Self {
            name: registration.local_name.clone(),
            description: registration.description.clone(),
            planner_description: registration.planning_description(),
            handler: ToolHandler::Mcp(registration),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn planner_description(&self) -> &str {
        &self.planner_description
    }

    pub fn run(&self, user_input: &str, arguments: &Value) -> StepOutcome {
        match &self.handler {
            ToolHandler::BuiltIn(handler) => handler(user_input, arguments),
            ToolHandler::Mcp(registration) => match registration.call(arguments) {
                Ok(output) => StepOutcome::Success(format!(
                    "MCP tool `{}` output:\n{}",
                    registration.local_name, output
                )),
                Err(reason) => StepOutcome::Retry(format!(
                    "MCP tool `{}` failed: {}",
                    registration.local_name, reason
                )),
            },
        }
    }

    pub fn is_named(&self, name: &str) -> bool {
        self.name == name
    }
}

enum ToolHandler {
    BuiltIn(BuiltInToolHandler),
    Mcp(McpToolRegistration),
}

pub fn default_tools() -> Vec<Tool> {
    vec![
        Tool::built_in(
            "web_search",
            "Run grounded web research through the Perplexity Sonar API and return an answer with citations.",
            tool_web_search_perplexity,
        ),
    ]
}
