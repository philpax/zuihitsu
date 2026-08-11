//! The custom chat request type plus the message/tool converters, carrying the serving layer's
//! sampling extensions that the standard `async-openai` schema does not model.

use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
    ChatCompletionRequestUserMessageContentPart, ChatCompletionTool, ChatCompletionTools,
    CreateChatCompletionRequest, FunctionCall, FunctionObject, ImageUrl,
};
use serde::Serialize;

use crate::model::{
    GenerateRequest, Message, ModelError, Role, ToolCall,
    openai::{ChatTemplateKwargs, backend},
};

/// The one custom type: the standard chat request plus the serving layer's sampling extensions,
/// which the standard schema does not model. Sent through `create_byot`.
#[derive(Serialize)]
pub(crate) struct ChatRequest {
    #[serde(flatten)]
    pub(crate) base: CreateChatCompletionRequest,
    /// An OpenAI `response_format` object, built as raw JSON for the byot path — a json-schema
    /// constraint on the whole reply (see [`crate::model::ResponseSchema`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) min_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) chat_template_kwargs: Option<ChatTemplateKwargs>,
}

pub(crate) fn to_messages(
    request: &GenerateRequest,
) -> Result<Vec<ChatCompletionRequestMessage>, ModelError> {
    fn system_message(content: &str) -> Result<ChatCompletionRequestMessage, ModelError> {
        Ok(ChatCompletionRequestSystemMessageArgs::default()
            .content(content.to_owned())
            .build()
            .map_err(backend)?
            .into())
    }

    fn message_tool_call(call: &ToolCall) -> ChatCompletionMessageToolCalls {
        ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
            id: call.id.clone(),
            function: FunctionCall {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            },
        })
    }

    let mut messages = Vec::with_capacity(request.messages.len() + 1);
    if !request.system.is_empty() {
        messages.push(system_message(&request.system)?);
    }
    for message in &request.messages {
        messages.push(match message.role {
            Role::System => system_message(&message.content)?,
            Role::User => ChatCompletionRequestUserMessageArgs::default()
                .content(user_content(message))
                .build()
                .map_err(backend)?
                .into(),
            Role::Assistant => {
                let mut args = ChatCompletionRequestAssistantMessageArgs::default();
                if !message.content.is_empty() {
                    args.content(message.content.clone());
                }
                if !message.tool_calls.is_empty() {
                    args.tool_calls(
                        message
                            .tool_calls
                            .iter()
                            .map(message_tool_call)
                            .collect::<Vec<_>>(),
                    );
                }
                args.build().map_err(backend)?.into()
            }
            Role::Tool => ChatCompletionRequestToolMessageArgs::default()
                .content(message.content.clone())
                .tool_call_id(message.tool_call_id.clone().unwrap_or_default())
                .build()
                .map_err(backend)?
                .into(),
        });
    }
    Ok(messages)
}

/// The content of a user message: the plain string it has always been, or — when the message shows
/// images — a content-part array of the text followed by one `image_url` part per image.
///
/// A text-only message keeps the string form, so no request that carried no image changes shape. The
/// images ride as `data:` URIs rather than as links back to this server, since nothing guarantees the
/// inference host can reach it.
fn user_content(message: &Message) -> ChatCompletionRequestUserMessageContent {
    if message.images.is_empty() {
        return ChatCompletionRequestUserMessageContent::Text(message.content.clone());
    }
    let mut parts = Vec::with_capacity(message.images.len() + 1);
    parts.push(ChatCompletionRequestUserMessageContentPart::Text(
        ChatCompletionRequestMessageContentPartText {
            text: message.content.clone(),
        },
    ));
    for image in &message.images {
        parts.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(
            ChatCompletionRequestMessageContentPartImage {
                image_url: ImageUrl {
                    url: format!("data:{};base64,{}", image.mime, image.data),
                    detail: None,
                },
            },
        ));
    }
    ChatCompletionRequestUserMessageContent::Array(parts)
}

pub(crate) fn to_tools(request: &GenerateRequest) -> Vec<ChatCompletionTools> {
    request
        .tools
        .iter()
        .map(|tool| {
            ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    parameters: Some(tool.parameters.clone()),
                    strict: None,
                },
            })
        })
        .collect()
}
