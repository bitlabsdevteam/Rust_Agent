use crate::mainAgent::StepOutcome;

pub(crate) fn run(retries: u8) -> StepOutcome {
    if retries == 0 {
        StepOutcome::Retry("Simulated transient skill failure.".to_string())
    } else {
        StepOutcome::Success("Retry-once skill recovered successfully.".to_string())
    }
}
