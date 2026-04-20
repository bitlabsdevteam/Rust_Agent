use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PlannerEvalFixture {
    pub name: String,
    pub user_input: String,
    #[serde(default)]
    pub observations: Vec<String>,
    pub expected: PlannerEvalExpectedDecision,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PlannerEvalExpectedDecision {
    pub action: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub skill_name: Option<String>,
    #[serde(default)]
    pub subagent_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerEvalActualDecision {
    pub action: String,
    pub tool_name: Option<String>,
    pub skill_name: Option<String>,
    pub subagent_name: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerEvalCaseResult {
    pub fixture: PlannerEvalFixture,
    pub actual: PlannerEvalActualDecision,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerEvalSuiteResult {
    pub fixture_dir: PathBuf,
    pub cases: Vec<PlannerEvalCaseResult>,
}

#[allow(dead_code)]
pub fn parse_eval_fixture(_contents: &str) -> io::Result<PlannerEvalFixture> {
    let fixture: PlannerEvalFixture = serde_json::from_str(_contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    validate_fixture(&fixture)?;
    Ok(fixture)
}

#[allow(dead_code)]
pub fn load_eval_fixtures(_dir: &Path) -> io::Result<Vec<PlannerEvalFixture>> {
    if !_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut paths = fs::read_dir(_dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    paths.sort();

    paths.into_iter()
        .map(|path| {
            let contents = fs::read_to_string(&path)?;
            parse_eval_fixture(&contents)
        })
        .collect()
}

fn validate_fixture(fixture: &PlannerEvalFixture) -> io::Result<()> {
    if fixture.name.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Eval fixture `name` must not be empty.",
        ));
    }
    if fixture.user_input.trim().is_empty() && fixture.expected.action != "stop" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Eval fixture `user_input` must not be empty.",
        ));
    }

    match fixture.expected.action.as_str() {
        "tool" => require_target(&fixture.expected.tool_name, "tool_name"),
        "skill" => require_target(&fixture.expected.skill_name, "skill_name"),
        "delegate" => require_target(&fixture.expected.subagent_name, "subagent_name"),
        "finish" | "retry" | "stop" => Ok(()),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Eval fixture `expected.action` must be one of tool, skill, delegate, finish, retry, stop; got `{other}`."
            ),
        )),
    }
}

fn require_target(value: &Option<String>, field_name: &str) -> io::Result<()> {
    match value.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Eval fixture action requires non-empty `{field_name}`."),
        )),
    }
}

impl PlannerEvalExpectedDecision {
    pub fn matches(&self, actual: &PlannerEvalActualDecision) -> bool {
        if self.action != actual.action {
            return false;
        }

        match self.action.as_str() {
            "tool" => self.tool_name == actual.tool_name,
            "skill" => self.skill_name == actual.skill_name,
            "delegate" => self.subagent_name == actual.subagent_name,
            "finish" | "retry" | "stop" => true,
            _ => false,
        }
    }

    pub fn summary(&self) -> String {
        match self.action.as_str() {
            "tool" => format!(
                "tool:{}",
                self.tool_name.as_deref().unwrap_or("<missing>")
            ),
            "skill" => format!(
                "skill:{}",
                self.skill_name.as_deref().unwrap_or("<missing>")
            ),
            "delegate" => format!(
                "delegate:{}",
                self.subagent_name.as_deref().unwrap_or("<missing>")
            ),
            other => other.to_string(),
        }
    }
}

impl PlannerEvalActualDecision {
    pub fn summary(&self) -> String {
        match self.action.as_str() {
            "tool" => format!(
                "tool:{}",
                self.tool_name.as_deref().unwrap_or("<missing>")
            ),
            "skill" => format!(
                "skill:{}",
                self.skill_name.as_deref().unwrap_or("<missing>")
            ),
            "delegate" => format!(
                "delegate:{}",
                self.subagent_name.as_deref().unwrap_or("<missing>")
            ),
            other => other.to_string(),
        }
    }
}

impl PlannerEvalSuiteResult {
    pub fn passed(&self) -> usize {
        self.cases.iter().filter(|case| case.passed).count()
    }

    pub fn failed(&self) -> usize {
        self.cases.len().saturating_sub(self.passed())
    }

    pub fn render(&self) -> String {
        let mut lines = vec![format!(
            "Planner evals: {} passed, {} failed ({})",
            self.passed(),
            self.failed(),
            self.fixture_dir.display()
        )];

        for case in &self.cases {
            if case.passed {
                lines.push(format!(
                    "[PASS] {} expected: {} actual: {}",
                    case.fixture.name,
                    case.fixture.expected.summary(),
                    case.actual.summary()
                ));
            } else {
                lines.push(format!(
                    "[FAIL] {} expected: {} actual: {} reason: {}",
                    case.fixture.name,
                    case.fixture.expected.summary(),
                    case.actual.summary(),
                    case.actual.reason
                ));
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        fs::create_dir_all(&path).expect("temp dir should exist");
        path
    }

    #[test]
    fn parse_eval_fixture_supports_user_input_observations_and_expected_action() {
        let fixture = parse_eval_fixture(
            r#"{
                "name": "tool routing",
                "user_input": "search the web for the latest rust release",
                "observations": ["No observations recorded yet."],
                "expected": {
                    "action": "tool",
                    "tool_name": "web_search"
                }
            }"#,
        )
        .expect("fixture should parse");

        assert_eq!(fixture.name, "tool routing");
        assert_eq!(
            fixture.user_input,
            "search the web for the latest rust release"
        );
        assert_eq!(fixture.observations.len(), 1);
        assert_eq!(fixture.expected.action, "tool");
        assert_eq!(fixture.expected.tool_name.as_deref(), Some("web_search"));
    }

    #[test]
    fn parse_eval_fixture_rejects_missing_target_for_targeted_actions() {
        let error = parse_eval_fixture(
            r#"{
                "name": "invalid tool fixture",
                "user_input": "search the web",
                "expected": {
                    "action": "tool"
                }
            }"#,
        )
        .expect_err("fixture should fail validation");

        assert!(error.to_string().contains("tool_name"));
    }

    #[test]
    fn parse_eval_fixture_allows_empty_user_input_for_stop_action() {
        let fixture = parse_eval_fixture(
            r#"{
                "name": "empty input stops",
                "user_input": "",
                "expected": {
                    "action": "stop"
                }
            }"#,
        )
        .expect("stop fixture should allow empty input");

        assert_eq!(fixture.user_input, "");
        assert_eq!(fixture.expected.action, "stop");
    }

    #[test]
    fn load_eval_fixtures_reads_sorted_json_files() {
        let root = temp_root("eval-fixtures");
        fs::write(
            root.join("b.json"),
            r#"{
                "name": "second",
                "user_input": "use skill ship-small",
                "expected": {
                    "action": "skill",
                    "skill_name": "ship-small"
                }
            }"#,
        )
        .expect("fixture should write");
        fs::write(
            root.join("a.json"),
            r#"{
                "name": "first",
                "user_input": "search the web",
                "expected": {
                    "action": "tool",
                    "tool_name": "web_search"
                }
            }"#,
        )
        .expect("fixture should write");

        let fixtures = load_eval_fixtures(&root).expect("fixtures should load");

        assert_eq!(fixtures.len(), 2);
        assert_eq!(fixtures[0].name, "first");
        assert_eq!(fixtures[1].name, "second");
    }

    #[test]
    fn expected_decision_matches_tool_route() {
        let expected = PlannerEvalExpectedDecision {
            action: "tool".to_string(),
            tool_name: Some("web_search".to_string()),
            skill_name: None,
            subagent_name: None,
        };
        let actual = PlannerEvalActualDecision {
            action: "tool".to_string(),
            tool_name: Some("web_search".to_string()),
            skill_name: None,
            subagent_name: None,
            reason: "explicit tool request".to_string(),
        };

        assert!(expected.matches(&actual));
    }

    #[test]
    fn eval_suite_render_includes_pass_fail_summary() {
        let fixture = PlannerEvalFixture {
            name: "tool routing".to_string(),
            user_input: "search the web".to_string(),
            observations: Vec::new(),
            expected: PlannerEvalExpectedDecision {
                action: "tool".to_string(),
                tool_name: Some("web_search".to_string()),
                skill_name: None,
                subagent_name: None,
            },
        };
        let passing = PlannerEvalCaseResult {
            fixture: fixture.clone(),
            actual: PlannerEvalActualDecision {
                action: "tool".to_string(),
                tool_name: Some("web_search".to_string()),
                skill_name: None,
                subagent_name: None,
                reason: "explicit tool request".to_string(),
            },
            passed: true,
        };
        let failing = PlannerEvalCaseResult {
            fixture,
            actual: PlannerEvalActualDecision {
                action: "delegate".to_string(),
                tool_name: None,
                skill_name: None,
                subagent_name: Some("general-purpose".to_string()),
                reason: "default delegation".to_string(),
            },
            passed: false,
        };
        let suite = PlannerEvalSuiteResult {
            fixture_dir: PathBuf::from("evals/fixtures"),
            cases: vec![passing, failing],
        };
        let report = suite.render();

        assert!(report.contains("Planner evals: 1 passed, 1 failed"));
        assert!(report.contains("[PASS] tool routing"));
        assert!(report.contains("[FAIL] tool routing"));
        assert!(report.contains("expected: tool:web_search"));
        assert!(report.contains("actual: delegate:general-purpose"));
    }
}
