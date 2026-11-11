//! Hosted image models as photo preparation providers ([`vectorcraft_imageprep::ImageProvider`]).
//!
//! This crate is the only one that sends a photo off the computer, and only when a command asks
//! for its provider by name (`photo.prepare {provider: gemini}`) or the `photoProvider` preference
//! chooses it. It sits above the engine (layer 6) and is never built for the web. The apps install
//! [`Gemini::from_env`] into a session; the engine knows only the trait.
//!
//! Spec: `docs/superpowers/specs/2026-11-11-embroidery-image-provider-design.md`.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod gemini;
mod transport;

pub use gemini::{
    DEFAULT_ENDPOINT, DEFAULT_MODEL, FAITHFUL_PROMPT, GEMINI, Gemini, KEY_VAR, MAX_REQUEST_BYTES, MAX_RESULT_PIXELS, MAX_SEND_SIDE, MISSING_KEY, MODEL_VAR,
};
pub use transport::{CONNECT_TIMEOUT, HttpReply, MAX_RESPONSE_BYTES, TIMEOUT, Transport, TransportError, UreqTransport};
