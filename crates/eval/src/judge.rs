//! The judge — the model, run clean-room (spec §Validation → the judge is the model, run clean-room).
//! A criterion plus a reprojection of only the relevant evidence is sent in a fresh request that
//! shares no context with the agent turn, so the model cannot rationalize from its own reasoning
//! trace. The judge calls a `verdict` tool (forced) so the structured answer can't drift into prose;
//! the verbatim response is recorded alongside the parsed verdict, because the matcher is a thing to
//! review, not trust.

use std::sync::Arc;

use serde::Deserialize;
use zuihitsu::{Completion, GenerateRequest, Message, ModelClient, ToolChoice, ToolSpec};

use crate::error::EvalError;

pub struct Judge {
    model: Arc<dyn ModelClient>,
}

/// One judgement: the decision, its one-line rationale, and the model's verbatim response.
pub struct JudgeOutcome {
    pub passed: bool,
    pub rationale: String,
    pub raw: String,
}

#[derive(Deserialize)]
struct ParsedVerdict {
    pass: bool,
    reason: String,
}

impl Judge {
    pub fn new(model: Arc<dyn ModelClient>) -> Judge {
        Judge { model }
    }

    /// Judge whether `criterion` holds, given `evidence` (the reprojected slices relevant to it). The
    /// criterion phrasing decides the polarity (a must-not-surface oracle phrases the leak as the thing
    /// to detect); the harness maps `pass` onto the scenario's bar.
    pub async fn assess(&self, criterion: &str, evidence: &str) -> Result<JudgeOutcome, EvalError> {
        let system = format!(
            "You are an impartial evaluator. You are given a CRITERION and the EVIDENCE relevant to \
             it, and nothing else — judge only from the evidence shown. Decide by meaning, not \
             wording: a paraphrase of a thing still counts as that thing. Be strict about \
             must-not-happen criteria — if the evidence plausibly shows the thing, the criterion is \
             not met. Call the `verdict` tool with your decision and a one-sentence reason.\n\n\
             CRITERION: {criterion}"
        );
        let tool = ToolSpec {
            name: "verdict".to_owned(),
            description: "Record your pass/fail decision and a one-sentence reason.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pass": { "type": "boolean", "description": "true if the criterion is met" },
                    "reason": { "type": "string", "description": "one sentence justifying it" }
                },
                "required": ["pass", "reason"]
            }),
        };
        let request = GenerateRequest {
            system,
            messages: vec![Message::user(format!("EVIDENCE:\n{evidence}"))],
            tools: vec![tool],
            tool_choice: ToolChoice::Required,
            thinking: None,
        };

        let response = self.model.generate(&request).await?;
        let raw = format!("{:?}", response.completion);
        let Completion::ToolCalls(calls) = &response.completion else {
            return Err(EvalError::Judge(format!(
                "the judge returned {:?} instead of a verdict tool call",
                response.completion
            )));
        };
        let call = calls
            .iter()
            .find(|call| call.name == "verdict")
            .ok_or_else(|| {
                EvalError::Judge("the judge did not call the verdict tool".to_owned())
            })?;
        let parsed: ParsedVerdict = serde_json::from_str(&call.arguments).map_err(|error| {
            EvalError::Judge(format!(
                "the judge's verdict did not parse ({error}): {}",
                call.arguments
            ))
        })?;
        Ok(JudgeOutcome {
            passed: parsed.pass,
            rationale: parsed.reason,
            raw,
        })
    }
}
