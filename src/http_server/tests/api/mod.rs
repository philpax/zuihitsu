//! HTTP integration tests for the agent's control and platform surfaces, grouped by API surface: the
//! control endpoints, the platform endpoints, the attachment routes, and the SSE event and message
//! streams.

mod blobs;
mod control;
mod platform;
mod read_only;
mod stream;
