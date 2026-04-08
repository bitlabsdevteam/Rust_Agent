use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::time::Duration;

use crate::mainAgent::{StepOutcome, DEFAULT_PERPLEXITY_MODEL};

const DEFAULT_PERPLEXITY_BASE_URL: &str = "https://api.perplexity.ai/v1/sonar";

pub(crate) fn tool_web_search_perplexity(user_input: &str, arguments: &Value) -> StepOutcome {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| extract_web_search_query(user_input));
    if query.is_empty() {
        return StepOutcome::Retry("Web search requires a non-empty query.".to_string());
    }

    let Some(engine) = PerplexityEngine::from_env() else {
        return StepOutcome::Retry(
            "Perplexity web search is unavailable because `PERPLEXITY_API_KEY` is not set."
                .to_string(),
        );
    };

    match engine.research(&query) {
        Ok(response) => StepOutcome::Success(format_perplexity_response(&query, &response)),
        Err(reason) => StepOutcome::Retry(format!("Perplexity web search failed: {reason}")),
    }
}

pub(crate) fn extract_web_search_query(user_input: &str) -> String {
    let trimmed = user_input.trim();
    let lower = trimmed.to_ascii_lowercase();

    let candidates = [
        "use tool web_search to ",
        "use tool web_search ",
        "tool: web_search ",
        "search:",
        "research:",
    ];

    for prefix in candidates {
        if lower.starts_with(prefix) {
            let query = trimmed[prefix.len()..].trim();
            if !query.is_empty() {
                return query.to_string();
            }
        }
    }

    trimmed.to_string()
}

pub(crate) fn format_perplexity_response(query: &str, response: &PerplexityResponse) -> String {
    let mut sections = vec![format!("Perplexity web research for `{query}`")];

    match response.answer_text() {
        Some(answer) => sections.push(format!("Answer:\n{answer}")),
        None => sections.push("Answer:\nPerplexity returned no answer text.".to_string()),
    }

    if !response.citations.is_empty() {
        let citations = response
            .citations
            .iter()
            .enumerate()
            .map(|(index, url)| format!("{}. {}", index + 1, url))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("Citations:\n{citations}"));
    }

    if !response.search_results.is_empty() {
        let search_results = response
            .search_results
            .iter()
            .take(5)
            .map(|result| {
                let title = result
                    .title
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("Untitled result");
                let url = result
                    .url
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("URL unavailable");
                let date = result
                    .date
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("date unknown");
                let snippet = result
                    .snippet
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("No snippet provided.");
                format!("- {title} ({date})\n  {url}\n  {snippet}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("Top search results:\n{search_results}"));
    }

    if !response.related_questions.is_empty() {
        let related = response
            .related_questions
            .iter()
            .take(5)
            .map(|question| format!("- {question}"))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("Related follow-up questions:\n{related}"));
    }

    if let Some(model) = response
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("Model: {model}"));
    }

    sections.join("\n\n")
}

#[derive(Debug, Clone)]
struct PerplexityEngine {
    api_key: String,
    base_url: String,
    model: String,
}

impl PerplexityEngine {
    fn from_env() -> Option<Self> {
        let api_key = env::var("PERPLEXITY_API_KEY").ok()?;
        let api_key = api_key.trim().to_string();
        if api_key.is_empty()
            || api_key.starts_with("replace-with-")
            || api_key.starts_with("your-")
        {
            return None;
        }

        let base_url = env::var("PERPLEXITY_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_PERPLEXITY_BASE_URL.to_string());
        let model = env::var("PERPLEXITY_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_PERPLEXITY_MODEL.to_string());

        Some(Self {
            api_key,
            base_url,
            model,
        })
    }

    fn research(&self, query: &str) -> Result<PerplexityResponse, String> {
        let request = json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a web research assistant. Answer with grounded findings, keep factual claims tied to sources, and call out uncertainty when the evidence is mixed."
                },
                {
                    "role": "user",
                    "content": query
                }
            ],
            "web_search_options": {
                "search_mode": "web",
                "return_related_questions": true
            }
        });

        self.send_request(request)
    }

    fn send_request(&self, request: Value) -> Result<PerplexityResponse, String> {
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|error| format!("client build error: {error}"))?;

        let response = client
            .post(&self.base_url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .map_err(|error| format!("request error: {error}"))?;

        let status = response.status();
        let body = response
            .text()
            .map_err(|error| format!("response read error: {error}"))?;

        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status.as_u16(), body));
        }

        serde_json::from_str(&body)
            .map_err(|error| format!("response JSON parse error: {error}; body={body}"))
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct PerplexityResponse {
    #[allow(dead_code)]
    id: Option<String>,
    model: Option<String>,
    #[serde(default)]
    choices: Vec<PerplexityChoice>,
    #[serde(default)]
    citations: Vec<String>,
    #[serde(default)]
    search_results: Vec<PerplexitySearchResult>,
    #[serde(default)]
    related_questions: Vec<String>,
}

impl PerplexityResponse {
    fn answer_text(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .map(str::trim)
            .filter(|content| !content.is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct PerplexityChoice {
    #[allow(dead_code)]
    index: Option<u32>,
    message: PerplexityMessage,
    #[allow(dead_code)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PerplexityMessage {
    #[allow(dead_code)]
    role: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PerplexitySearchResult {
    title: Option<String>,
    url: Option<String>,
    snippet: Option<String>,
    date: Option<String>,
}
