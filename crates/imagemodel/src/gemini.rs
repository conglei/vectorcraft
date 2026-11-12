//! Google's Gemini API as a photo preparation provider.
//!
//! One `models/{model}:generateContent` call (https://aistudio.google.com/api-reference/generate-content):
//! the photo as inline PNG data and [`FAITHFUL_PROMPT`], asking for an image back
//! (`generationConfig.responseModalities: ["IMAGE"]`). Models and their codes:
//! https://aistudio.google.com/docs/models; image editing: https://aistudio.google.com/docs/image-generation;
//! request limits (20 MB inline): https://aistudio.google.com/docs/image-understanding.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_imageprep::{ImageProvider, PrepareOptions, Prepared, Raster, check, decode_capped, downscale, encode_png, flatten, resize_exact};

use crate::transport::{HttpReply, Transport, TransportError, UreqTransport};

/// The provider's name in commands.
pub const GEMINI: &str = "gemini";
/// The environment variable the API key is read from, at each call; it is never stored, written
/// or logged.
pub const KEY_VAR: &str = "GEMINI_API_KEY";
/// The environment variable that names another model, if set.
pub const MODEL_VAR: &str = "VECTORCRAFT_GEMINI_MODEL";
/// The model used unless another is named: Nano Banana 2, listed as stable.
pub const DEFAULT_MODEL: &str = "gemini-3.1-flash-image";
/// The API's base address.
pub const DEFAULT_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta";
/// The longest side of the photo sent, in pixels: larger photos are scaled down first.
pub const MAX_SEND_SIDE: u32 = 1536;
/// The largest request sent (18 MB, under the API's 20 MB for inline data).
pub const MAX_REQUEST_BYTES: usize = 18 << 20;
/// The most pixels a returned image may have (4096 × 4096); a larger one is refused undecoded.
pub const MAX_RESULT_PIXELS: u64 = 4096 * 4096;
/// The error when there is no key.
pub const MISSING_KEY: &str = "set GEMINI_API_KEY to use the Gemini provider; the no-model path still works";
/// What the model is asked to do in faithful mode.
pub const FAITHFUL_PROMPT: &str = "Edit this photo for embroidery digitizing. Remove the background completely and replace it with one plain, flat, \
pure white colour (#FFFFFF): no checkerboard or transparency pattern, no shadow, no gradient, no texture. Keep the main subject exactly as it is: the same pose, proportions, markings, \
colours, expression and framing, at the same position and scale in the image. Do not restyle, stylize, cartoonize, beautify, add or remove \
anything on the subject. Lightly reduce noise and blur only. Return only the edited image.";

/// The longest error text kept from the API.
const MAX_MESSAGE: usize = 300;

type KeySource = Box<dyn Fn() -> Option<String> + Send + Sync>;

/// The Gemini provider: a model, an endpoint, how requests are sent and where the key comes from.
pub struct Gemini {
    model: String,
    endpoint: String,
    transport: Arc<dyn Transport>,
    key: KeySource,
}

impl Gemini {
    /// The provider the apps install: the key from [`KEY_VAR`], the model from [`MODEL_VAR`] or
    /// [`DEFAULT_MODEL`], HTTPS through [`UreqTransport`].
    pub fn from_env(user_agent: &str) -> Self {
        let model =
            std::env::var(MODEL_VAR).ok().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| DEFAULT_MODEL.to_string());
        Self::new(&model, DEFAULT_ENDPOINT, Arc::new(UreqTransport::new(user_agent)), Box::new(|| std::env::var(KEY_VAR).ok()))
    }

    /// A provider with its parts given (tests: a recorded transport and a made-up key).
    pub fn new(model: &str, endpoint: &str, transport: Arc<dyn Transport>, key: KeySource) -> Self {
        Self { model: model.to_string(), endpoint: endpoint.trim_end_matches('/').to_string(), transport, key }
    }

    /// The model asked.
    pub fn model(&self) -> &str {
        &self.model
    }

    fn key(&self) -> Result<String, String> {
        (self.key)().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).ok_or_else(|| MISSING_KEY.to_string())
    }

    fn url(&self) -> Result<String, String> {
        let ok = !self.model.is_empty() && self.model.len() <= 100 && self.model.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b));
        if !ok {
            return Err(format!("“{}” is not a Gemini model name", self.model));
        }
        Ok(format!("{}/models/{}:generateContent", self.endpoint, self.model))
    }
}

/// The request body for `png`.
fn request(png: &[u8]) -> Value {
    json!({
        "contents": [{
            "role": "user",
            "parts": [
                {"text": FAITHFUL_PROMPT},
                {"inline_data": {"mime_type": "image/png", "data": vectorcraft_format::base64_encode(png)}}
            ]
        }],
        "generationConfig": {"responseModalities": ["IMAGE"]}
    })
}

/// `text` without the key and cut to [`MAX_MESSAGE`] characters.
fn scrub(text: &str, key: &str) -> String {
    let text = if key.is_empty() { text.to_string() } else { text.replace(key, "[key]") };
    let mut out: String = text.chars().take(MAX_MESSAGE).collect();
    if text.chars().count() > MAX_MESSAGE {
        out.push('…');
    }
    out
}

/// The error for an answer with status `reply.status` other than 200.
fn http_error(reply: &HttpReply, key: &str) -> String {
    let message = serde_json::from_slice::<Value>(&reply.body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(|m| scrub(m, key)))
        .unwrap_or_default();
    let detail = if message.is_empty() { String::new() } else { format!(": {message}") };
    match reply.status {
        400 => format!("Gemini refused the request (HTTP 400){detail}"),
        401 | 403 => format!("Gemini refused the API key (HTTP {}): check GEMINI_API_KEY{detail}", reply.status),
        404 => format!("Gemini has no such model (HTTP 404); set {MODEL_VAR} to a current image model{detail}"),
        429 => format!("Gemini's quota or rate limit was reached (HTTP 429): try again later, or use the no-model path{detail}"),
        s if s >= 500 => format!("Gemini is unavailable (HTTP {s}): try again later, or use the no-model path{detail}"),
        s => format!("Gemini answered HTTP {s}{detail}"),
    }
}

/// The image bytes in a `generateContent` answer, and any text the model added.
fn image_in(body: &[u8], key: &str) -> Result<(Vec<u8>, Vec<String>), String> {
    let v: Value = serde_json::from_slice(body).map_err(|_| "Gemini's answer could not be read (not JSON)".to_string())?;
    if let Some(reason) = v.pointer("/promptFeedback/blockReason").and_then(Value::as_str) {
        return Err(format!("Gemini refused the photo ({}); the no-model path still works", scrub(reason, key)));
    }
    let candidate = v.pointer("/candidates/0").ok_or("Gemini returned no result")?;
    let parts = candidate.pointer("/content/parts").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut texts = Vec::new();
    for part in &parts {
        let inline = part.get("inlineData").or_else(|| part.get("inline_data"));
        if let Some(data) = inline.and_then(|i| i.get("data")).and_then(Value::as_str) {
            let bytes = vectorcraft_format::base64_decode(data).ok_or("Gemini's image could not be read (bad base64)")?;
            return Ok((bytes, texts));
        }
        if let Some(t) = part.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
            texts.push(scrub(t, key));
        }
    }
    let reason = candidate.get("finishReason").and_then(Value::as_str).unwrap_or("no reason given");
    let said = texts.first().map(|t| format!(": “{t}”")).unwrap_or_default();
    Err(format!("Gemini returned no image ({}){said}", scrub(reason, key)))
}

impl ImageProvider for Gemini {
    fn name(&self) -> &str {
        GEMINI
    }

    fn sends_off_machine(&self) -> bool {
        true
    }

    fn ready(&self) -> Result<(), String> {
        self.key().map(|_| ())
    }

    fn prepare(&self, image: &Raster, options: &PrepareOptions) -> Result<Prepared, String> {
        let key = self.key()?;
        let url = self.url()?;
        check(image)?;
        let mut notes = Vec::new();
        let sent = downscale(image, options.max_side.min(MAX_SEND_SIDE))?;
        if (sent.width, sent.height) != (image.width, image.height) {
            notes.push(format!("scaled from {} × {} to {} × {} pixels before sending", image.width, image.height, sent.width, sent.height));
        }
        let png = encode_png(&sent)?;
        let body = serde_json::to_vec(&request(&png)).map_err(|e| format!("couldn't build the request: {e}"))?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(format!("the photo is too large to send ({} MB, at most {} MB): lower maxSide", body.len() >> 20, MAX_REQUEST_BYTES >> 20));
        }
        let headers = [("content-type", "application/json"), ("x-goog-api-key", key.as_str())];
        let reply = self.transport.post(&url, &headers, body).map_err(|e| match e {
            TransportError::Timeout => {
                format!("Gemini took too long to answer (more than {} s); the no-model path still works", crate::TIMEOUT.as_secs())
            }
            TransportError::TooLarge => format!("Gemini's answer is larger than {} MB and was not read", crate::MAX_RESPONSE_BYTES >> 20),
            TransportError::Network(m) => format!("couldn't reach Gemini ({}); the no-model path still works", scrub(&m, &key)),
        })?;
        if reply.status != 200 {
            return Err(http_error(&reply, &key));
        }
        let (bytes, texts) = image_in(&reply.body, &key)?;
        let mut out = decode_capped(&bytes, MAX_RESULT_PIXELS).map_err(|e| format!("Gemini's image was refused: {e}"))?;
        let aspect = |w: u32, h: u32| f64::from(w) / f64::from(h.max(1));
        if (aspect(out.width, out.height) / aspect(sent.width, sent.height) - 1.0).abs() <= 0.02 {
            if (out.width, out.height) != (sent.width, sent.height) {
                out = resize_exact(&out, sent.width, sent.height)?;
            }
        } else {
            notes.push(format!(
                "Gemini returned a {} × {} image for a {} × {} photo: its framing may differ",
                out.width, out.height, sent.width, sent.height
            ));
            out = downscale(&out, options.max_side)?;
        }
        let transparent = out.rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 128);
        notes.push(if transparent {
            "background removed (transparent)".to_string()
        } else {
            "background replaced by a flat colour (no transparency returned): check the subject's edges".to_string()
        });
        if !options.keep_alpha {
            out = flatten(&out, [255, 255, 255]);
        }
        notes.extend(texts.into_iter().map(|t| format!("Gemini said: {t}")));
        notes.push(format!("prepared by Gemini ({}); the photo was sent to Google's Gemini API", self.model));
        Ok(Prepared { image: out, provider: GEMINI.to_string(), notes })
    }
}

#[cfg(test)]
mod tests;
