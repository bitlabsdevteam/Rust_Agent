use serde::Deserialize;
use std::fs;
use std::io;
use std::path::Path;

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
    if fixture.user_input.trim().is_empty() {
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
}
