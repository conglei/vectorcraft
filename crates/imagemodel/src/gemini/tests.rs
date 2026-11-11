use std::sync::Mutex;

use vectorcraft_imageprep::{LIKENESS_THRESHOLD, likeness, synthetic_photo};

use super::*;

/// A made-up key: tests never see a real one.
const KEY: &str = "test-key-not-real-0123";

/// A request as sent: its URL, headers and JSON body.
type Sent = (String, Vec<(String, String)>, Value);

/// Answers every request with a recorded reply and keeps what was sent.
struct Recorded {
    reply: Result<HttpReply, TransportError>,
    sent: Mutex<Vec<Sent>>,
}

impl Recorded {
    fn new(reply: Result<HttpReply, TransportError>) -> Arc<Self> {
        Arc::new(Self { reply, sent: Mutex::new(vec![]) })
    }
}

impl Transport for Recorded {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Result<HttpReply, TransportError> {
        let h = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.sent.lock().unwrap().push((url.to_string(), h, serde_json::from_slice(&body).unwrap()));
        self.reply.clone()
    }
}

fn gemini(t: Arc<Recorded>, key: Option<&'static str>) -> Gemini {
    Gemini::new(DEFAULT_MODEL, DEFAULT_ENDPOINT, t, Box::new(move || key.map(str::to_string)))
}

fn ok(body: Value) -> Result<HttpReply, TransportError> {
    Ok(HttpReply { status: 200, body: serde_json::to_vec(&body).unwrap() })
}

/// A `generateContent` answer holding `img` as PNG, as the API returns it (camelCase).
fn answer_with(img: &Raster) -> Value {
    let png = encode_png(img).unwrap();
    json!({
        "candidates": [{
            "content": {"role": "model", "parts": [{"inlineData": {"mimeType": "image/png", "data": vectorcraft_format::base64_encode(&png)}}]},
            "finishReason": "STOP"
        }],
        "usageMetadata": {"promptTokenCount": 1290}
    })
}

/// The subject of `photo` cut out onto transparency.
fn cut_out(photo: &Raster) -> Raster {
    let (w, h) = (f64::from(photo.width), f64::from(photo.height));
    let r = w.min(h) * 0.3;
    Raster::from_fn(photo.width, photo.height, |x, y| {
        let (dx, dy) = ((f64::from(x) - w * 0.5) / r, (f64::from(y) - h * 0.55) / (r * 0.85));
        if dx * dx + dy * dy <= 0.95 { photo.pixel(x, y) } else { [0, 0, 0, 0] }
    })
}

#[test]
fn a_recorded_answer_is_decoded_checked_and_put_back_at_the_size_sent() {
    let photo = synthetic_photo(2000, 1500, 1);
    let sent = downscale(&photo, MAX_SEND_SIDE).unwrap();
    // The model answers at a size of its own with the same shape.
    let back = resize_exact(&cut_out(&sent), 1024, 768).unwrap();
    let t = Recorded::new(ok(answer_with(&back)));
    let out = gemini(t.clone(), Some(KEY)).prepare(&photo, &PrepareOptions { max_side: 4096, ..Default::default() }).unwrap();
    assert_eq!(out.provider, GEMINI);
    assert_eq!((out.image.width, out.image.height), (1536, 1152));
    assert!(out.notes.iter().any(|n| n.contains("transparent")), "{:?}", out.notes);
    assert!(out.notes.iter().any(|n| n.contains("sent to Google")), "{:?}", out.notes);
    assert!(likeness(&photo, &out.image).score >= LIKENESS_THRESHOLD);
    // What was sent: the key in a header only, the photo scaled to the cap, the faithful prompt.
    let sent_log = t.sent.lock().unwrap();
    let (url, headers, body) = sent_log.first().unwrap();
    assert_eq!(url, &format!("{DEFAULT_ENDPOINT}/models/{DEFAULT_MODEL}:generateContent"));
    assert!(!url.contains(KEY));
    assert!(headers.contains(&("x-goog-api-key".to_string(), KEY.to_string())));
    assert_eq!(body["contents"][0]["parts"][0]["text"], FAITHFUL_PROMPT);
    assert_eq!(body["generationConfig"]["responseModalities"], json!(["IMAGE"]));
    let png = vectorcraft_format::base64_decode(body["contents"][0]["parts"][1]["inline_data"]["data"].as_str().unwrap()).unwrap();
    let decoded = decode_capped(&png, 1 << 24).unwrap();
    assert_eq!((decoded.width, decoded.height), (1536, 1152));
}

#[test]
fn a_snake_case_answer_and_a_flat_background_are_taken() {
    let photo = synthetic_photo(300, 200, 2);
    let flat = flatten(&cut_out(&photo), [255, 255, 255]);
    let png = encode_png(&flat).unwrap();
    let body = json!({"candidates": [{"content": {"parts": [
        {"text": "Here is the edited image."},
        {"inline_data": {"mime_type": "image/png", "data": vectorcraft_format::base64_encode(&png)}}
    ]}}]});
    let out = gemini(Recorded::new(ok(body)), Some(KEY)).prepare(&photo, &PrepareOptions::default()).unwrap();
    assert_eq!((out.image.width, out.image.height), (300, 200));
    assert!(out.notes.iter().any(|n| n.contains("no transparency")), "{:?}", out.notes);
    assert!(out.notes.iter().any(|n| n.contains("Here is the edited image")), "{:?}", out.notes);
}

#[test]
fn a_reframed_answer_is_kept_with_a_note() {
    let photo = synthetic_photo(400, 300, 3);
    let square = synthetic_photo(300, 300, 3);
    let out = gemini(Recorded::new(ok(answer_with(&square))), Some(KEY)).prepare(&photo, &PrepareOptions::default()).unwrap();
    assert_eq!((out.image.width, out.image.height), (300, 300));
    assert!(out.notes.iter().any(|n| n.contains("framing may differ")), "{:?}", out.notes);
}

#[test]
fn a_missing_key_sends_nothing() {
    let t = Recorded::new(ok(json!({})));
    for key in [None, Some(""), Some("   ")] {
        let g = gemini(t.clone(), key);
        assert_eq!(g.ready().unwrap_err(), MISSING_KEY);
        assert_eq!(g.prepare(&synthetic_photo(64, 64, 0), &PrepareOptions::default()).unwrap_err(), MISSING_KEY);
    }
    assert!(t.sent.lock().unwrap().is_empty());
}

#[test]
fn http_errors_say_what_to_do_without_the_key() {
    let photo = synthetic_photo(64, 64, 4);
    let cases = [
        (400, "refused the request"),
        (403, "check GEMINI_API_KEY"),
        (404, MODEL_VAR),
        (429, "rate limit"),
        (503, "unavailable"),
        (418, "HTTP 418"),
    ];
    for (status, want) in cases {
        let body = json!({"error": {"code": status, "message": format!("bad thing with key {KEY}"), "status": "X"}});
        let t = Recorded::new(Ok(HttpReply { status, body: serde_json::to_vec(&body).unwrap() }));
        let e = gemini(t, Some(KEY)).prepare(&photo, &PrepareOptions::default()).unwrap_err();
        assert!(e.contains(want), "{status}: {e}");
        assert!(e.contains("bad thing"), "{e}");
        assert!(!e.contains(KEY), "{e}");
    }
}

#[test]
fn bad_answers_are_errors() {
    let photo = synthetic_photo(64, 64, 5);
    let run = |reply: Result<HttpReply, TransportError>| gemini(Recorded::new(reply), Some(KEY)).prepare(&photo, &PrepareOptions::default()).unwrap_err();
    let raw = |b: &[u8]| Ok(HttpReply { status: 200, body: b.to_vec() });
    assert!(run(raw(b"<html>")).contains("not JSON"));
    assert!(run(ok(json!({}))).contains("no result"));
    assert!(run(ok(json!({"promptFeedback": {"blockReason": "SAFETY"}}))).contains("refused the photo (SAFETY)"));
    let no_image = json!({"candidates": [{"content": {"parts": [{"text": "I can't do that."}]}, "finishReason": "IMAGE_SAFETY"}]});
    let e = run(ok(no_image));
    assert!(e.contains("no image (IMAGE_SAFETY)") && e.contains("I can't do that"), "{e}");
    let bad_b64 = json!({"candidates": [{"content": {"parts": [{"inlineData": {"mimeType": "image/png", "data": "!!!"}}]}}]});
    assert!(run(ok(bad_b64)).contains("bad base64"));
    let not_png = json!({"candidates": [{"content": {"parts": [{"inlineData": {"mimeType": "image/png", "data": vectorcraft_format::base64_encode(b"nope")}}]}}]});
    assert!(run(ok(not_png)).contains("image was refused"));
    assert!(run(Err(TransportError::Timeout)).contains("took too long"));
    assert!(run(Err(TransportError::TooLarge)).contains("was not read"));
    let e = run(Err(TransportError::Network(format!("dns failure {KEY}"))));
    assert!(e.contains("couldn't reach Gemini") && !e.contains(KEY), "{e}");
}

#[test]
fn an_oversize_answer_image_is_refused_before_decoding() {
    // A PNG header claiming 5000 × 5000 pixels: refused by its size alone.
    let big = image::RgbaImage::new(5000, 5000);
    let mut png = Vec::new();
    big.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let e = gemini(Recorded::new(ok(json!({"candidates": [{"content": {"parts": [{"inlineData": {"data": vectorcraft_format::base64_encode(&png)}}]}}]}))), Some(KEY))
        .prepare(&synthetic_photo(64, 64, 6), &PrepareOptions::default())
        .unwrap_err();
    assert!(e.contains("5000 × 5000"), "{e}");
}

#[test]
fn a_bad_model_name_is_refused_before_sending() {
    let t = Recorded::new(ok(json!({})));
    let g = Gemini::new("../x?key=1", DEFAULT_ENDPOINT, t.clone(), Box::new(|| Some(KEY.to_string())));
    assert!(g.prepare(&synthetic_photo(64, 64, 0), &PrepareOptions::default()).unwrap_err().contains("not a Gemini model name"));
    assert!(t.sent.lock().unwrap().is_empty());
}

/// One real call, run by hand: `GEMINI_API_KEY=… cargo test -p vectorcraft-imagemodel live -- --ignored --nocapture`.
/// It sends a made-up photo ([`synthetic_photo`]) and prints only sizes, likeness and timing.
#[test]
#[ignore = "calls the real Gemini API; needs GEMINI_API_KEY"]
fn live_gemini_call() {
    let photo = synthetic_photo(1024, 768, 42);
    let g = Gemini::from_env("VectorCraft-test");
    let start = std::time::Instant::now();
    let out = g.prepare(&photo, &PrepareOptions::default());
    let secs = start.elapsed().as_secs_f64();
    match out {
        Ok(p) => {
            let l = likeness(&photo, &p.image);
            println!("model {} width {} height {} likeness {:.3} subject {:.2} seconds {secs:.1}", g.model(), p.image.width, p.image.height, l.score, l.subject);
            println!("notes {:?} likeness notes {:?}", p.notes, l.notes);
        }
        Err(e) => panic!("after {secs:.1} s: {e}"),
    }
}
