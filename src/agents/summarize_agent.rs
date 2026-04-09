use crate::mainAgent::StepOutcome;

pub(crate) fn run(user_input: &str) -> StepOutcome {
    let summary = user_input
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");

    StepOutcome::Success(format!("Summary skill output: {summary}"))
}
