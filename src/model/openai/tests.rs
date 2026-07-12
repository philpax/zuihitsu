//! The custom byot response type must deserialize the serving layer's `reasoning_content` and
//! the full token usage that the standard `async-openai` type drops — the deliberation the
//! model-interaction record captures (spec §Observability) — and the backend-error
//! classification must mark exactly the transport/overload failures transient.
use async_openai::error::{ApiError, ApiErrorResponse, OpenAIError};
use reqwest::StatusCode;

use super::{
    is_transient,
    response::{ChatResponse, into_response},
};
use crate::model::{Completion, Usage};

fn api_error(status: StatusCode) -> OpenAIError {
    OpenAIError::ApiError(ApiErrorResponse {
        status_code: status,
        api_error: ApiError {
            message: "backend says no".to_owned(),
            r#type: None,
            param: None,
            code: None,
        },
    })
}

#[test]
fn classifies_api_statuses() {
    // Overload and server-side statuses are transient; request-side statuses are not.
    for (status, expected) in [
        (StatusCode::REQUEST_TIMEOUT, true),
        (StatusCode::TOO_MANY_REQUESTS, true),
        (StatusCode::INTERNAL_SERVER_ERROR, true),
        (StatusCode::BAD_GATEWAY, true),
        (StatusCode::SERVICE_UNAVAILABLE, true),
        (StatusCode::BAD_REQUEST, false),
        (StatusCode::UNAUTHORIZED, false),
        (StatusCode::NOT_FOUND, false),
        (StatusCode::UNPROCESSABLE_ENTITY, false),
    ] {
        assert_eq!(
            is_transient(&api_error(status)),
            expected,
            "misclassified {status}"
        );
    }
}

#[test]
fn builder_and_serde_failures_are_not_transient() {
    assert!(!is_transient(&OpenAIError::InvalidArgument(
        "missing model".to_owned()
    )));
    let serde_error = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
    assert!(!is_transient(&OpenAIError::JSONDeserialize(
        serde_error,
        "not json".to_owned()
    )));
}

#[test]
fn deserializes_reasoning_and_full_usage_from_a_reply() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "  Hello there.\n",
                "reasoning_content": "The user greeted me, so I greet back."
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17 }
    });
    let response: ChatResponse = serde_json::from_value(body).expect("body deserializes");
    let generated = into_response(response).expect("maps to a response");

    assert_eq!(
        generated.completion,
        Completion::Reply("Hello there.".to_owned())
    );
    assert_eq!(
        generated.reasoning.as_deref(),
        Some("The user greeted me, so I greet back.")
    );
    assert_eq!(generated.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        generated.usage,
        Usage {
            prompt_tokens: Some(12),
            completion_tokens: Some(5),
            total_tokens: Some(17),
            ..Usage::default()
        }
    );
}

#[test]
fn parses_cached_tokens_into_the_cache_read_field() {
    // A server that reports `prompt_tokens_details.cached_tokens` (vLLM, OpenAI) surfaces it as
    // `cache_read_tokens`; no OpenAI-compatible server reports a write signal, so writes stay `None`.
    let body = serde_json::json!({
        "choices": [{ "message": { "role": "assistant", "content": "ok" } }],
        "usage": {
            "prompt_tokens": 3112,
            "completion_tokens": 4,
            "total_tokens": 3116,
            "prompt_tokens_details": { "cached_tokens": 2048 }
        }
    });
    let response: ChatResponse = serde_json::from_value(body).expect("body deserializes");
    let generated = into_response(response).expect("maps to a response");
    assert_eq!(generated.usage.cache_read_tokens, Some(2048));
    assert_eq!(generated.usage.cache_write_tokens, None);
}

#[test]
fn absent_cache_details_stay_unknown_not_zero() {
    // llama.cpp's compatibility endpoint reports usage without `prompt_tokens_details`; the cache
    // fields must stay `None` (unknown), never a fabricated zero. The same holds when the details
    // object is present but empty.
    for usage in [
        serde_json::json!({ "prompt_tokens": 10 }),
        serde_json::json!({ "prompt_tokens": 10, "prompt_tokens_details": {} }),
    ] {
        let body = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "ok" } }],
            "usage": usage,
        });
        let response: ChatResponse = serde_json::from_value(body).expect("body deserializes");
        let generated = into_response(response).expect("maps to a response");
        assert_eq!(generated.usage.cache_read_tokens, None);
        assert_eq!(generated.usage.cache_write_tokens, None);
    }
}

#[test]
fn deserializes_a_tool_call_and_tolerates_absent_reasoning() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "run_lua", "arguments": "{\"script\":\"return 1\"}" }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });
    let response: ChatResponse = serde_json::from_value(body).expect("body deserializes");
    let generated = into_response(response).expect("maps to a response");

    match generated.completion {
        Completion::ToolCalls(calls) => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "run_lua");
        }
        other => panic!("expected a tool call, got {other:?}"),
    }
    // No `reasoning_content` and no `usage` block — both degrade to `None`, never an error.
    assert_eq!(generated.reasoning, None);
    assert_eq!(generated.usage, Usage::default());
    assert_eq!(generated.finish_reason.as_deref(), Some("tool_calls"));
}

/// The streaming assembler must reproduce the whole-body mapping exactly: folding the chunked form
/// of a response and finishing yields the same `GenerateResponse` as mapping the equivalent
/// non-streaming body — the contract the recorded `ModelCalled` relies on.
#[test]
fn assembled_chunks_equal_the_whole_body_mapping() {
    use super::response::{ChatChunk, StreamAssembler};
    use crate::model::GenerateDelta;

    let chunks = [
        serde_json::json!({"choices": [{"delta": {"reasoning_content": "Thinking "}}]}),
        serde_json::json!({"choices": [{"delta": {"reasoning_content": "hard."}}]}),
        serde_json::json!({"choices": [{"delta": {"content": "Hello "}}]}),
        serde_json::json!({"choices": [{"delta": {"content": "there."}, "finish_reason": "stop"}]}),
        serde_json::json!({"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14}}),
    ];
    let mut assembler = StreamAssembler::default();
    let mut fragments = Vec::new();
    for chunk in chunks {
        let chunk: ChatChunk = serde_json::from_value(chunk).expect("chunk deserializes");
        fragments.extend(assembler.fold(chunk));
    }
    assert_eq!(
        fragments,
        vec![
            GenerateDelta::Reasoning("Thinking ".to_owned()),
            GenerateDelta::Reasoning("hard.".to_owned()),
            GenerateDelta::Reply("Hello ".to_owned()),
            GenerateDelta::Reply("there.".to_owned()),
        ]
    );
    let streamed = assembler.finish().expect("assembles");

    let body = serde_json::json!({
        "choices": [{
            "message": {"content": "Hello there.", "reasoning_content": "Thinking hard."},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14}
    });
    let whole: ChatResponse = serde_json::from_value(body).expect("body deserializes");
    assert_eq!(streamed, into_response(whole).expect("maps"));
}

/// Tool-call argument fragments concatenate across chunks by their shared index, and `id`/`name`
/// arrive once on the first fragment — the OpenAI streaming shape for tool calls.
#[test]
fn fragmented_tool_call_arguments_reassemble() {
    use super::response::{ChatChunk, StreamAssembler};

    let chunks = [
        serde_json::json!({"choices": [{"delta": {"tool_calls": [
            {"index": 0, "id": "call_1", "function": {"name": "run_lua", "arguments": "{\"scr"}}
        ]}}]}),
        serde_json::json!({"choices": [{"delta": {"tool_calls": [
            {"index": 0, "function": {"arguments": "ipt\":\"return 1\"}"}}
        ]}}]}),
        serde_json::json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
    ];
    let mut assembler = StreamAssembler::default();
    for chunk in chunks {
        let chunk: ChatChunk = serde_json::from_value(chunk).expect("chunk deserializes");
        // Tool-call fragments are not display text, so folding them yields no fragments.
        assert!(assembler.fold(chunk).is_empty());
    }
    let response = assembler.finish().expect("assembles");
    match response.completion {
        Completion::ToolCalls(calls) => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "call_1");
            assert_eq!(calls[0].name, "run_lua");
            assert_eq!(calls[0].arguments, "{\"script\":\"return 1\"}");
        }
        other => panic!("expected a tool call, got {other:?}"),
    }
    assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
}

/// A body that closes without a single chunk is the backend producing nothing — surfaced as a
/// transient error rather than assembled into an empty reply the turn would record.
#[test]
fn an_empty_stream_is_an_error_not_an_empty_reply() {
    use super::response::StreamAssembler;

    let error = StreamAssembler::default().finish().expect_err("no chunks");
    assert!(error.is_transient());
    assert!(error.to_string().contains("without a single chunk"));
}

/// The truncation ladder: a generous first clip, a 0.9× trim per length-overflow rejection, a
/// floor at a quarter of the window, and immediate surfacing of unrelated failures.
mod embed_truncation {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use parking_lot::Mutex;

    use super::super::{embed_truncated, is_length_overflow, truncate_chars};
    use crate::model::ModelError;

    fn overflow() -> ModelError {
        ModelError::Backend {
            model: "embedder".to_owned(),
            message: "the input exceeds the maximum context length of 512 tokens".to_owned(),
            transient: false,
        }
    }

    #[test]
    fn truncation_cuts_whole_characters_only() {
        assert_eq!(truncate_chars("hello", 3), "hel");
        assert_eq!(truncate_chars("hello", 10), "hello");
        // Multi-byte characters count as one each; the cut can never split one.
        assert_eq!(truncate_chars("日本語のテキスト", 3), "日本語");
        assert_eq!(truncate_chars("", 4), "");
    }

    #[tokio::test]
    async fn the_first_attempt_clips_to_two_and_a_half_chars_per_token() {
        let seen = Mutex::new(Vec::new());
        let long = "x".repeat(4_000);
        embed_truncated(&[long], 512, |inputs| {
            seen.lock().push(inputs[0].chars().count());
            async { Ok(vec![vec![0.0]]) }
        })
        .await
        .unwrap();
        assert_eq!(*seen.lock(), vec![1_280]);
    }

    #[tokio::test]
    async fn a_length_overflow_trims_the_budget_by_a_tenth_and_retries() {
        let seen = Mutex::new(Vec::new());
        let long = "x".repeat(4_000);
        let result = embed_truncated(&[long], 512, |inputs| {
            let mut seen = seen.lock();
            seen.push(inputs[0].chars().count());
            let fail = seen.len() == 1;
            async move {
                if fail {
                    Err(overflow())
                } else {
                    Ok(vec![vec![0.0]])
                }
            }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(*seen.lock(), vec![1_280, 1_152]);
    }

    #[tokio::test]
    async fn persistent_overflows_stop_at_the_floor_and_surface_the_error() {
        let attempts = AtomicUsize::new(0);
        let long = "x".repeat(4_000);
        let result = embed_truncated(&[long], 512, |_| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(overflow()) }
        })
        .await;
        assert!(matches!(result, Err(ModelError::Backend { .. })));
        // 1280 shrinking by 0.9x per attempt stays above the 128-char floor for 22 attempts;
        // the 23rd step would fall below it.
        assert_eq!(attempts.load(Ordering::SeqCst), 22);
    }

    #[tokio::test]
    async fn an_unrelated_failure_surfaces_without_a_retry() {
        let attempts = AtomicUsize::new(0);
        let result = embed_truncated(&["hi".to_owned()], 512, |_| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async {
                Err(ModelError::Backend {
                    model: "embedder".to_owned(),
                    message: "invalid api key".to_owned(),
                    transient: false,
                })
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_overflow_heuristic_matches_the_backends_vocabularies() {
        let backend = |message: &str| ModelError::Backend {
            model: "embedder".to_owned(),
            message: message.to_owned(),
            transient: false,
        };
        // The recurring wordings across llama.cpp, vLLM, and TEI.
        assert!(is_length_overflow(&backend(
            "input is too large to process"
        )));
        assert!(is_length_overflow(&backend(
            "This model's maximum context length is 512 tokens"
        )));
        assert!(is_length_overflow(&backend(
            "`inputs` must have less than 512 tokens"
        )));
        assert!(!is_length_overflow(&backend("invalid api key")));
        assert!(!is_length_overflow(&ModelError::Exhausted));
    }
}
