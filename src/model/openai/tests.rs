//! The custom byot response type must deserialize the serving layer's `reasoning_content` and
//! the full token usage that the standard `async-openai` type drops — the deliberation the
//! model-interaction record captures (spec §Observability) — and the backend-error
//! classification must mark exactly the transport/overload failures transient.
use async_openai::error::{ApiError, ApiErrorResponse, OpenAIError};
use reqwest::StatusCode;

use super::{ChatResponse, into_response, is_transient};
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
