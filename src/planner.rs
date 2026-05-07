use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    CallTool {
        tool_name: String,
        arguments: serde_json::Value,
        reason: String,
    },
    UseSkill {
        skill_name: String,
        reason: String,
    },
    DelegateSubagent {
        subagent_name: String,
        reason: String,
    },
    Finish {
        answer: String,
        reason: String,
    },
    Retry(String),
    Stop(String),
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CallTool {
                tool_name, reason, ..
            } => write!(f, "call tool `{tool_name}` ({reason})"),
            Self::UseSkill { skill_name, reason } => {
                write!(f, "use skill `{skill_name}` ({reason})")
            }
            Self::DelegateSubagent {
                subagent_name,
                reason,
            } => write!(f, "delegate to subagent `{subagent_name}` ({reason})"),
            Self::Finish { answer, reason } => write!(f, "finish ({reason}; {answer})"),
            Self::Retry(reason) => write!(f, "retry ({reason})"),
            Self::Stop(reason) => write!(f, "stop ({reason})"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerFailureKind {
    Recoverable,
    Terminal,
}

impl PlannerFailureKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recoverable => "recoverable",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannerRun<Usage = ()> {
    pub decision: Decision,
    pub reasoning: Vec<String>,
    pub trace: Vec<String>,
    pub usage: Option<Usage>,
    pub failure_kind: Option<PlannerFailureKind>,
}

impl<Usage> PlannerRun<Usage> {
    pub fn success(
        decision: Decision,
        reasoning: Vec<String>,
        usage: Option<Usage>,
    ) -> Self {
        Self {
            decision,
            reasoning,
            trace: Vec::new(),
            usage,
            failure_kind: None,
        }
    }

    pub fn failure(
        decision: Decision,
        reasoning: Vec<String>,
        failure_kind: PlannerFailureKind,
    ) -> Self {
        Self {
            decision,
            reasoning,
            trace: Vec::new(),
            usage: None,
            failure_kind: Some(failure_kind),
        }
    }

    pub fn push_trace(&mut self, entry: impl Into<String>) {
        self.trace.push(entry.into());
    }
}

#[derive(Debug, Clone)]
pub struct PlannerRouter {
    backend_label: String,
    default_model: String,
    fallback_model: String,
    allow_heuristic_planner: bool,
}

impl PlannerRouter {
    pub fn new(
        backend_label: impl Into<String>,
        default_model: impl Into<String>,
        fallback_model: impl Into<String>,
        allow_heuristic_planner: bool,
    ) -> Self {
        Self {
            backend_label: backend_label.into(),
            default_model: default_model.into(),
            fallback_model: fallback_model.into(),
            allow_heuristic_planner,
        }
    }

    pub fn decide<Usage, Primary, Fallback, Heuristic>(
        &self,
        primary: Option<Primary>,
        fallback: Option<Fallback>,
        heuristic: Heuristic,
    ) -> PlannerRun<Usage>
    where
        Primary: FnOnce() -> Result<PlannerRun<Usage>, String>,
        Fallback: FnOnce() -> Result<PlannerRun<Usage>, String>,
        Heuristic: FnOnce() -> PlannerRun<Usage>,
    {
        if let Some(primary) = primary {
            match primary() {
                Ok(mut plan) => {
                    plan.push_trace(success_trace("primary", &self.default_model));
                    if fallback.is_some() {
                        plan.push_trace(
                            "planner fallback outcome = not used; primary planner succeeded."
                                .to_string(),
                        );
                    } else {
                        plan.push_trace(
                            "planner fallback outcome = not available; no configured fallback planner."
                                .to_string(),
                        );
                    }
                    return plan;
                }
                Err(reason) => {
                    let primary_failure_kind = classify_planner_failure_kind(&reason);
                    if let Some(fallback) = fallback {
                        match fallback() {
                            Ok(mut plan) => {
                                plan.reasoning.push(format!(
                                    "Primary planner failed and the harness fell back to `{}`: {}",
                                    self.fallback_model, reason
                                ));
                                plan.push_trace(failure_trace(
                                    "primary",
                                    &self.default_model,
                                    primary_failure_kind,
                                    &reason,
                                ));
                                plan.push_trace(success_trace("fallback", &self.fallback_model));
                                plan.push_trace(
                                    "planner fallback outcome = configured fallback planner succeeded after primary failure."
                                        .to_string(),
                                );
                                return plan;
                            }
                            Err(fallback_reason) => {
                                let fallback_failure_kind =
                                    classify_planner_failure_kind(&fallback_reason);
                                let failure_kind = if primary_failure_kind
                                    == PlannerFailureKind::Recoverable
                                    || fallback_failure_kind
                                        == PlannerFailureKind::Recoverable
                                {
                                    PlannerFailureKind::Recoverable
                                } else {
                                    PlannerFailureKind::Terminal
                                };
                                let mut run = planner_failure_run(
                                    self.backend_label.clone(),
                                    format!(
                                        "Primary planner failed: {reason}; fallback planner failed: {fallback_reason}"
                                    ),
                                    failure_kind,
                                    Some(format!(
                                        "The harness attempted the configured fallback planner `{}` after `{}` failed.",
                                        self.fallback_model, self.default_model
                                    )),
                                );
                                run.push_trace(failure_trace(
                                    "primary",
                                    &self.default_model,
                                    primary_failure_kind,
                                    &reason,
                                ));
                                run.push_trace(failure_trace(
                                    "fallback",
                                    &self.fallback_model,
                                    fallback_failure_kind,
                                    &fallback_reason,
                                ));
                                run.push_trace(
                                    "planner fallback outcome = configured fallback planner failed after primary failure."
                                        .to_string(),
                                );
                                return run;
                            }
                        }
                    }

                    if self.allow_heuristic_planner {
                        let mut fallback = heuristic();
                        fallback.reasoning.push(format!(
                            "{} failed; fell back to the local heuristic router: {reason}",
                            self.default_model
                        ));
                        fallback.push_trace(failure_trace(
                            "primary",
                            &self.default_model,
                            primary_failure_kind,
                            &reason,
                        ));
                        fallback.push_trace(
                            "planner fallback outcome = local heuristic router succeeded after primary planner failure."
                                .to_string(),
                        );
                        return fallback;
                    }

                    let mut run = planner_failure_run(
                        self.backend_label.clone(),
                        format!("OpenAI planner `{}` failed: {reason}", self.default_model),
                        primary_failure_kind,
                        None,
                    );
                    run.push_trace(failure_trace(
                        "primary",
                        &self.default_model,
                        primary_failure_kind,
                        &reason,
                    ));
                    run.push_trace(
                        "planner fallback outcome = not available; no configured fallback planner."
                            .to_string(),
                    );
                    return run;
                }
            }
        }

        if let Some(fallback) = fallback {
            match fallback() {
                Ok(mut plan) => {
                    plan.push_trace(success_trace("fallback", &self.fallback_model));
                    plan.push_trace(
                        "planner fallback outcome = configured fallback planner was selected directly."
                            .to_string(),
                    );
                    return plan;
                }
                Err(reason) => {
                    let fallback_failure_kind = classify_planner_failure_kind(&reason);
                    if self.allow_heuristic_planner {
                        let mut fallback = heuristic();
                        fallback.reasoning.push(format!(
                            "Fallback planner `{}` failed; fell back to the local heuristic router: {reason}",
                            self.fallback_model
                        ));
                        fallback.push_trace(failure_trace(
                            "fallback",
                            &self.fallback_model,
                            fallback_failure_kind,
                            &reason,
                        ));
                        fallback.push_trace(
                            "planner fallback outcome = local heuristic router succeeded after fallback planner failure."
                                .to_string(),
                        );
                        return fallback;
                    }

                    let mut run = planner_failure_run(
                        self.backend_label.clone(),
                        format!("Fallback planner `{}` failed: {reason}", self.fallback_model),
                        fallback_failure_kind,
                        None,
                    );
                    run.push_trace(failure_trace(
                        "fallback",
                        &self.fallback_model,
                        fallback_failure_kind,
                        &reason,
                    ));
                    run.push_trace(
                        "planner fallback outcome = configured fallback planner failed."
                            .to_string(),
                    );
                    return run;
                }
            }
        }

        if self.allow_heuristic_planner {
            let mut plan = heuristic();
            plan.push_trace(
                "planner attempt = heuristic backend `local heuristic router`; outcome = success; failure class = none."
                    .to_string(),
            );
            plan.push_trace("planner fallback outcome = not applicable.".to_string());
            return plan;
        }

        let mut run = planner_failure_run(
            "no planner configured",
            "No model-backed planner is configured for this agent.".to_string(),
            PlannerFailureKind::Terminal,
            None,
        );
        run.push_trace(
            "planner attempt = none; outcome = failure; failure class = terminal; reason = No model-backed planner is configured for this agent."
                .to_string(),
        );
        run.push_trace("planner fallback outcome = not applicable.".to_string());
        run
    }
}

fn success_trace(stage: &str, backend: &str) -> String {
    format!(
        "planner attempt = {stage} backend `{backend}`; outcome = success; failure class = none."
    )
}

fn failure_trace(
    stage: &str,
    backend: &str,
    failure_kind: PlannerFailureKind,
    reason: &str,
) -> String {
    format!(
        "planner attempt = {stage} backend `{backend}`; outcome = failure; failure class = {}; reason = {reason}",
        failure_kind.label()
    )
}

pub fn classify_planner_failure_kind(reason: &str) -> PlannerFailureKind {
    let reason = reason.to_ascii_lowercase();
    if reason.contains("timeout")
        || reason.contains("timed out")
        || reason.contains("503")
        || reason.contains("502")
        || reason.contains("429")
        || reason.contains("connection reset")
        || reason.contains("temporarily unavailable")
        || reason.contains("rate limit")
    {
        PlannerFailureKind::Recoverable
    } else {
        PlannerFailureKind::Terminal
    }
}

pub fn planner_failure_run<Usage>(
    backend_label: impl Into<String>,
    reason: impl Into<String>,
    failure_kind: PlannerFailureKind,
    fallback_reasoning: Option<String>,
) -> PlannerRun<Usage> {
    let backend_label = backend_label.into();
    let reason = reason.into();
    let mut reasoning = vec![format!(
        "Planner backend `{backend_label}` classified as {}.",
        failure_kind.label()
    )];
    if let Some(fallback_reasoning) = fallback_reasoning {
        reasoning.push(fallback_reasoning);
    }
    reasoning.push(reason.clone());

    let decision = match failure_kind {
        PlannerFailureKind::Recoverable => Decision::Retry(reason),
        PlannerFailureKind::Terminal => Decision::Stop(reason),
    };

    PlannerRun::failure(decision, reasoning, failure_kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish_run(label: &str) -> PlannerRun<()> {
        PlannerRun::success(
            Decision::Finish {
                answer: label.to_string(),
                reason: label.to_string(),
            },
            vec![label.to_string()],
            None,
        )
    }

    #[test]
    fn router_prefers_primary_success() {
        let router = PlannerRouter::new("primary -> fallback", "gpt-5.4", "Opus 4.6", true);

        let run = router.decide(
            Some(|| Ok(finish_run("primary"))),
            Some(|| Ok(finish_run("fallback"))),
            || finish_run("heuristic"),
        );

        assert_eq!(run.decision, finish_run("primary").decision);
        assert_eq!(run.reasoning, vec!["primary".to_string()]);
    }

    #[test]
    fn router_uses_fallback_after_primary_failure() {
        let router = PlannerRouter::new("primary -> fallback", "gpt-5.4", "Opus 4.6", true);

        let run = router.decide(
            Some(|| Err("HTTP 503 Service Unavailable".to_string())),
            Some(|| Ok(finish_run("fallback"))),
            || finish_run("heuristic"),
        );

        assert_eq!(run.decision, finish_run("fallback").decision);
        assert!(run
            .reasoning
            .iter()
            .any(|entry| entry.contains("Primary planner failed and the harness fell back")));
    }

    #[test]
    fn router_classifies_dual_failure_as_recoverable() {
        let router = PlannerRouter::new("primary -> fallback", "gpt-5.4", "Opus 4.6", false);

        let run = router.decide::<(), _, _, _>(
            Some(|| Err("HTTP 503 Service Unavailable".to_string())),
            Some(|| Err("Planner JSON parse failed: missing action".to_string())),
            || finish_run("heuristic"),
        );

        assert_eq!(run.failure_kind, Some(PlannerFailureKind::Recoverable));
        assert!(matches!(run.decision, Decision::Retry(_)));
    }

    #[test]
    fn failure_classifier_distinguishes_transient_and_terminal_errors() {
        assert_eq!(
            classify_planner_failure_kind("HTTP 503 Service Unavailable"),
            PlannerFailureKind::Recoverable
        );
        assert_eq!(
            classify_planner_failure_kind("Planner JSON parse failed: missing action"),
            PlannerFailureKind::Terminal
        );
    }
}
