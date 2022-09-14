use crate::common::transport::Transport;
use crate::v3::transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport};

#[derive(Debug)]
pub enum TransportType {
    Polling(PollingTransport),
    WebsocketSecure(WebsocketSecureTransport),
    Websocket(WebsocketTransport),
}

impl From<PollingTransport> for TransportType {
    fn from(transport: PollingTransport) -> Self {
        TransportType::Polling(transport)
    }
}

impl From<WebsocketSecureTransport> for TransportType {
    fn from(transport: WebsocketSecureTransport) -> Self {
        TransportType::WebsocketSecure(transport)
    }
}

impl From<WebsocketTransport> for TransportType {
    fn from(transport: WebsocketTransport) -> Self {
        TransportType::Websocket(transport)
    }
}

impl TransportType {
    pub fn as_transport(&self) -> &dyn Transport {
        match self {
            TransportType::Polling(transport) => transport,
            TransportType::Websocket(transport) => transport,
            TransportType::WebsocketSecure(transport) => transport,
        }
    }
}
