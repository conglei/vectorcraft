# Embroidery image provider: design

> **Status:** first pass implemented · **Date:** 2026-11-11 · **Covers:** gap E12 in [embroidery-parity.md](../../embroidery-parity.md) (Linear STI-13): the provider interface, the no-model path, the Gemini adapter, the likeness check, `photo.providers` and `photo.prepare`

## Goal

Get a pet photo ready for the stitch pipeline (E11) faithfully: the subject cut out from its
background and cleaned up, never restyled. A hosted image model does the cut-out. Without a
model the pipeline still works: the photo is scaled down and cleaned up on this computer, and the
background stays.

Owner decisions (2026-11-11): the provider is **Gemini**, its key comes from the `GEMINI_API_KEY`
environment variable, faithful preparation is the default, restyles are opt-in and not in this
pass, and everything must work with no model.

## Placement and layering

| Piece | Where | Layer | Why |
|---|---|---|---|
| `ImageProvider` trait, `PrepareOptions`, `Prepared`, `NoModel`, `likeness` | `crates/imageprep` (`vectorcraft-imageprep`) | L3 | Offline, builds for wasm. Needs `vectorcraft-trace` (L2) for `Raster`, capped decoding and the edge-preserving smoothing, and `image` for scaling and PNG. Kept out of the engine so the adapter can implement the trait without depending on the engine |
| Gemini adapter, `Transport`, `UreqTransport` | `crates/imagemodel` (`vectorcraft-imagemodel`) | L6 | The only crate that talks to the network. Above the engine, like `ui-egui`, which already reaches the network through `craft-update`. Not built for the web |
| `photo.providers`, `photo.prepare`, `Session::photo` (`Providers`), the `photoProvider` preference | `crates/engine/src/cmd/photo.rs`, `prefs.rs`, `prefscmds.rs` | L5 | Everything user-visible is a command. The engine sees only `Arc<dyn ImageProvider>` |
| Installing Gemini into a session | `apps/vectorcraft/src/prefs_dir.rs` (`attach_image_providers`, shared with `vectorcraft-cli`); called by the desktop app, `vectorcraft-cli run` and `vectorcraft-cli mcp` (headless, also when confined to automation roots) | L7 | The apps choose what is installed. The web installs nothing, so there `photo.prepare` offers `nomodel` only |

`cargo xtask layers` registers both crates (`imageprep` L3, `imagemodel` L6).

**HTTP crate.** `ureq` 3 (MIT OR Apache-2.0), with the features `craft-update` already builds it
with (`rustls-no-provider`, `_ring`, `platform-verifier`): rustls on ring, certificates checked by
the operating system, no new crate in `Cargo.lock`. Its `rustls` feature was not used because it
would add `webpki-roots` (CDLA-Permissive-2.0, not on the licence allowlist). The Google SDK is not
used: one HTTPS JSON call is enough.

The suite's `craft-ai-core` (craft-libs) is an interim, much larger contract (connections, roles,
consent ledgers, jobs). It is not adopted here; see Deferred.

## Provider interface

```rust
pub trait ImageProvider: Send + Sync {
    fn name(&self) -> &str;                          // "nomodel", "gemini"
    fn sends_off_machine(&self) -> bool { false }   // the photo leaves this computer
    fn ready(&self) -> Result<(), String> { Ok(()) } // e.g. the key is missing
    fn prepare(&self, image: &Raster, options: &PrepareOptions) -> Result<Prepared, String>;
}
pub struct PrepareOptions { pub mode: Mode /* Faithful only */, pub max_side: u32, pub keep_alpha: bool, pub clean_up: bool }
pub struct Prepared { pub image: Raster, pub provider: String, pub notes: Vec<String> }
```

`Raster` is Image Trace's RGBA8 image. `max_side` defaults to 1024 (64–4096); a smaller photo is
never scaled up. Errors are sentences for the person.

## The no-model path

`NoModel` scales the photo down to `max_side` (box filter, aspect kept), flattens it onto white
when `keep_alpha` is off, and with `clean_up` (default) applies Image Trace's bilateral smoothing
(`vectorcraft_trace::smoothed`, now public), which removes noise within tones and keeps edges where
they are. It cannot cut the subject out and says so in its notes: "no model: the background was
not removed; the subject is as in the photo".

## Gemini adapter

- **Call.** `POST {endpoint}/models/{model}:generateContent` with the key in the
  `x-goog-api-key` header (never in the URL). Body:
  `{"contents": [{"role": "user", "parts": [{"text": PROMPT}, {"inline_data": {"mime_type": "image/png", "data": BASE64}}]}], "generationConfig": {"responseModalities": ["IMAGE"]}}`.
  The answer's image is the first part with `inlineData` (or `inline_data`) `{mimeType, data}` in
  `candidates[0].content.parts`; text parts become notes.
- **Defaults.** Endpoint `https://generativelanguage.googleapis.com/v1beta`; model
  `gemini-3.1-flash-image` (Nano Banana 2, listed as stable). `VECTORCRAFT_GEMINI_MODEL` names
  another; `Gemini::new` takes model and endpoint as parameters. A model name with anything but
  letters, digits, `-`, `.` and `_` is refused before sending.
- **Prompt** (fixed, faithful): remove the background completely and make it one plain flat
  white (`#FFFFFF`), with no checkerboard or transparency pattern, shadow, gradient or texture;
  keep the subject exactly as it is (pose, proportions, markings, colours, expression, framing,
  position and scale); no restyling, stylizing, cartoonizing, beautifying, adding or removing;
  lightly reduce noise and blur only; return only the edited image.
- **Size limits.** The photo is scaled to at most 1536 px on the long side before sending (and to
  `max_side` if smaller); a request over 18 MB is refused (the API takes 20 MB of inline data). The
  answer is read up to 40 MB; its image is decoded only when its header says at most 4096 × 4096
  pixels. An answer of the shape sent (within 2 %) is scaled back to the size sent; another shape is
  kept, scaled to `max_side`, with a note that its framing may differ. `keep_alpha: false` flattens
  it onto white. A note says whether the background came back transparent or as a flat colour.
- **Time limits.** Connect 15 s, the whole call 120 s; no redirects (a redirect would resend the
  key); HTTPS only.
- **Key.** Read from `GEMINI_API_KEY` at each call (it is not stored in the provider); blank counts
  as missing.
- **Errors.** Missing key: "set GEMINI_API_KEY to use the Gemini provider; the no-model path still
  works". HTTP 400: "Gemini refused the request"; 401/403: "Gemini refused the API key … check
  GEMINI_API_KEY"; 404: no such model, set `VECTORCRAFT_GEMINI_MODEL`; 429: quota or rate limit;
  5xx: unavailable; each with the API's `error.message`. Blocked prompt
  (`promptFeedback.blockReason`): "Gemini refused the photo (REASON)". No image part: "Gemini
  returned no image (finishReason)" with the model's text. Timeout, network failure, oversize
  answer, bad base64, undecodable image: their own messages. Every message from the API or the
  network is cut to 300 characters and has the key replaced by `[key]`.

API references: <https://aistudio.google.com/api-reference/generate-content> (request, response,
`finishReason` and `blockReason` values, `responseModalities`),
<https://aistudio.google.com/docs/image-generation> (image editing),
<https://aistudio.google.com/docs/models> (model codes),
<https://aistudio.google.com/docs/image-understanding> (20 MB inline limit, input types). The
image-generation guide now shows the newer Interactions API (`/v1beta/interactions`); the adapter
uses `generateContent`, which the API reference lists without a deprecation notice.

## Likeness check

`likeness(original, prepared)` returns a 0–1 score. Both images are scaled to 96 px on the long
side (the original's aspect) and compared only on the prepared image's subject: pixels at least
half opaque that are not its flat background (a colour 75 % of its border is within 30 levels of),
less the subject's outermost pixel ring, which blends into either background. The score is 0.8 ×
the mean SSIM of luminance over 8 × 8 windows four pixels apart (windows at least half subject)
plus 0.2 × a colour term (1 at no mean RGB difference, 0 at 120 levels). Fewer than 1 % subject
pixels scores 0 ("no subject left"). A different shape than the photo's is noted.

`photo.prepare` warns below `likenessThreshold` (default 0.6). Measured in tests: identical images
score 1.0, a blank result about 0, the subject cut out onto transparency or white above 0.85, the
no-model result above 0.6, stripes or an inverted subject below 0.6. The live Gemini runs below
scored 0.95.

Limits: fine detail does not count at 96 px; a different animal in the same pose and colours can
score well; a subject the same colour as a flat background is taken for background; a reframed or
zoomed result scores low although nothing changed. It asks a person to look; it does not judge.

## Commands

- `photo.providers {}` → `{providers: [{name, ready, sendsOffMachine, note?}], active, preference}`.
- `photo.prepare {file, path?, provider?, maxSide?, keepAlpha?, cleanUp?, likenessThreshold?}` →
  `{provider, width, height, likeness, notes, warnings, path}` or, without `path`,
  `{…, pngBase64}` (at most 2048 × 2048 pixels inline). `file` is the photo; `path` is where the result
  is written, the key `stitch.preview` and the exports use for their output (and the one the command sweep
  redirects into a temp folder). The photo file is read through the
  engine's file access (automation roots apply), at most 64 MB, decoded at most 192 megapixels and
  scaled to 64 if larger.

Both are queries (not journaled, no undo step, no document needed). MCP reaches them through
`run_command`. Plug-ins may not call them: `photo.prepare` names files, `photo.providers` is on
the denied list because it says how the computer is set up.

## Opt-in and privacy

- **Nothing leaves the computer** unless `photo.prepare` names `provider: "gemini"` or the
  `photoProvider` preference (Preferences › File Handling › Photo Preparation › Image Model,
  values `nomodel` (default) and `gemini`) chooses it. That differs from the first plan ("gemini
  when a key is present"): a key in the environment is not consent to send a photo, so the key's
  presence only adds a warning that names the opt-in.
- Named in the params without a key: an error with the missing-key sentence. Chosen in the
  preference without a key (or where the app has no Gemini, the web): `nomodel`, with a warning.
- Installing the provider sends nothing. The key is never stored, written, logged or put in a URL,
  error or test fixture; tests use a made-up key. Image bytes and request bodies are never logged.
  The notes of a Gemini result say the photo was sent to Google's Gemini API.
- The call blocks the thread that runs the command (up to 120 s). The CLI and MCP are fine; the
  desktop app has no menu item or dialog for it yet, and its dialog must run it off the UI thread.

## Tests

- `vectorcraft-imageprep`: no-model end to end on a photo drawn in code (`synthetic_photo`: a
  brown blob with a darker patch and a light spot on a noisy blue-grey background), alpha
  flattening, malformed rasters, `fit_side`, capped decoding, likeness (identical 1.0, blank low,
  different low, cut-out on transparency and on white high, restyle low, reshaped noted).
- `vectorcraft-imagemodel`: the adapter against recorded answers through a recording `Transport`
  (no network in CI): request shape, key only in the header, photo scaled to 1536, answer scaled
  back, snake_case answers, flat backgrounds, reframed answers, missing key sends nothing, HTTP
  errors without the key, blocked and imageless answers, bad base64, undecodable and oversize
  images, timeout, oversize body, network errors scrubbed, bad model names.
- Engine (`photo` tests): providers listing and the preference, no-model end to end to base64 and
  to a file, the opt-in rules with a fake hosted provider (counted calls), the missing-key error and
  fallback, the likeness warning, bad params and files, the inline size cap. `command_sweep` runs
  both commands with empty and junk params; the plug-in review list places them.
- **Manual runs** (2026-11-11, with the developer's key; photos drawn in code, no personal photo):
  - `cargo test -p vectorcraft-imagemodel live -- --ignored --nocapture` on a 1024 × 768
    `synthetic_photo`: model `gemini-3.1-flash-image`, 1024 × 768 back, likeness 0.951, 6.8 s.
  - `vectorcraft-cli run --cmd photo.prepare` on an 800 × 600 PNG drawn by a script: 800 × 600,
    likeness 0.945, flat white background, subject unchanged (checked by eye); the same photo
    through `nomodel`: likeness 0.999.
  - Finding: asked for "transparent, or flat white if not possible", the model returned an opaque
    image with a checkerboard *painted* where the background was (likeness 0.79). The model does not
    return an alpha channel, so the prompt now asks for flat white only.

## Deferred

- Restyle presets (opt-in), with the likeness check guarding them.
- The E11 stitch pipeline that consumes the prepared image.
- A dialog in the app (beyond the preference), run off the UI thread, with a first-use disclosure.
- The web build: no network provider there yet.
- A cut-out mask returned as its own channel when the model gives only a flat background (a local
  matte from the flat colour), and a second hosted adapter.
- Adopting `craft-ai-core` (connections, consent ledger, history) once the suite settles its AI core.
- Keys from the system keychain instead of the environment.
