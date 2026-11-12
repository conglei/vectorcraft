//! The HTTPS call, behind a trait so tests answer with recorded responses and make no network
//! calls.

use std::time::Duration;

/// How long a whole request may take, sending the photo and reading the image back: image models
/// take tens of seconds.
pub const TIMEOUT: Duration = Duration::from_secs(120);
/// How long connecting may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The largest response read (40 MB): a 4096 × 4096 PNG in base64 fits; more is refused, not read.
pub const MAX_RESPONSE_BYTES: u64 = 40 << 20;

/// An HTTP answer: its status and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Why a request got no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// It took longer than allowed.
    Timeout,
    /// The answer was larger than [`MAX_RESPONSE_BYTES`].
    TooLarge,
    /// No connection, TLS or protocol failure; the text says which.
    Network(String),
}

/// Sends a POST and returns the answer, whatever its status.
pub trait Transport: Send + Sync {
    /// POST `body` to `url` with `headers`. Implementations never log the headers (one is the API
    /// key) or the body (the photo).
    fn post(&self, url: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Result<HttpReply, TransportError>;
}

/// [`Transport`] over HTTPS with ureq: HTTPS only, certificates checked by the operating system,
/// the proxy from the environment, [`TIMEOUT`] and [`CONNECT_TIMEOUT`].
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    /// `user_agent` names the product and its version and nothing about the person.
    pub fn new(user_agent: &str) -> Self {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            // The API answers directly; a redirect would resend the key elsewhere.
            .max_redirects(0)
            .user_agent(user_agent)
            .build()
            .new_agent();
        Self { agent }
    }
}

impl Transport for UreqTransport {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Result<HttpReply, TransportError> {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let failed = |e: ureq::Error| match e {
            ureq::Error::Timeout(_) => TransportError::Timeout,
            ureq::Error::BodyExceedsLimit(_) => TransportError::TooLarge,
            other => TransportError::Network(other.to_string()),
        };
        let mut resp = req.send(&body[..]).map_err(failed)?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(MAX_RESPONSE_BYTES).read_to_vec().map_err(failed)?;
        Ok(HttpReply { status, body })
    }
}
