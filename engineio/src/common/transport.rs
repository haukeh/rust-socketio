use std::time::SystemTime;

use adler32::adler32;
use bytes::Bytes;
use url::Url;

// use crate::v4::transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport};
use crate::error::Result;

pub static APPLICATION_OCTET_STREAM: http::HeaderValue =
    http::HeaderValue::from_static("application/octet-stream");

#[derive(Debug)]
pub enum ContentType {
    Binary,
    String,
}

#[derive(Debug)]
pub struct PollingResponse {
    pub content_type: ContentType,
    pub data: Bytes,
}

pub trait Transport {
    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()>;

    /// Performs the server long polling procedure as long as the client is
    /// connected. This should run separately at all time to ensure proper
    /// response handling from the server.
    fn poll(&self) -> Result<PollingResponse>;

    /// Returns start of the url. ex. http://localhost:2998/engine.io/?EIO=4&transport=polling
    /// Must have EIO and transport already set.
    fn base_url(&self) -> Result<Url>;

    /// Used to update the base path, like when adding the sid.
    fn set_base_url(&self, base_url: Url) -> Result<()>;

    /// Full query address
    fn address(&self) -> Result<Url> {
        let reader = format!("{:#?}", SystemTime::now());
        let hash = adler32(reader.as_bytes()).unwrap();
        let mut url = self.base_url()?;
        url.query_pairs_mut().append_pair("t", &hash.to_string());
        Ok(url)
    }
}

impl std::fmt::Debug for dyn Transport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("Transport(base_url: {:?})", self.base_url(), ))
    }
}
