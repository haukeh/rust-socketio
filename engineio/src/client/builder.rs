use std::convert::TryFrom;
use std::convert::TryInto;
use std::fmt::Debug;
use std::sync::Arc;

use bytes::Bytes;
use native_tls::TlsConnector;
use url::Url;

use crate::{Client, ENGINE_IO_VERSION};
use crate::callback::OptionalCallback;
use crate::error::{Error, Result};
use crate::header::HeaderMap;
use crate::packet::{HandshakePacket, Packet};
use crate::socket::{Socket as EngineSocket, V4Socket};
use crate::transport::Transport;
use crate::transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport};

#[derive(Clone, Debug)]
pub struct ClientBuilder {
    url: Url,
    tls_config: Option<TlsConnector>,
    headers: Option<HeaderMap>,
    handshake: Option<HandshakePacket>,
    on_error: OptionalCallback<String>,
    on_open: OptionalCallback<()>,
    on_close: OptionalCallback<()>,
    on_data: OptionalCallback<Bytes>,
    on_packet: OptionalCallback<Packet>,
}

impl ClientBuilder {
    pub fn new(url: Url) -> Self {
        let mut url = url;
        url.query_pairs_mut()
            .append_pair("EIO", &ENGINE_IO_VERSION.to_string());

        // No path add engine.io
        if url.path() == "/" {
            url.set_path("/engine.io/");
        }

        ClientBuilder {
            url,
            headers: None,
            tls_config: None,
            handshake: None,
            on_close: OptionalCallback::default(),
            on_data: OptionalCallback::default(),
            on_error: OptionalCallback::default(),
            on_open: OptionalCallback::default(),
            on_packet: OptionalCallback::default(),
        }
    }

    /// Specify transport's tls config
    pub fn tls_config(mut self, tls_config: TlsConnector) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    /// Specify transport's HTTP headers
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Registers the `on_close` callback.
    pub fn on_close<T>(mut self, callback: T) -> Self
        where
            T: Fn(()) + 'static + Sync + Send,
    {
        self.on_close = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_data` callback.
    pub fn on_data<T>(mut self, callback: T) -> Self
        where
            T: Fn(Bytes) + 'static + Sync + Send,
    {
        self.on_data = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_error` callback.
    pub fn on_error<T>(mut self, callback: T) -> Self
        where
            T: Fn(String) + 'static + Sync + Send,
    {
        self.on_error = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_open` callback.
    pub fn on_open<T>(mut self, callback: T) -> Self
        where
            T: Fn(()) + 'static + Sync + Send,
    {
        self.on_open = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_packet` callback.
    pub fn on_packet<T>(mut self, callback: T) -> Self
        where
            T: Fn(Packet) + 'static + Sync + Send,
    {
        self.on_packet = OptionalCallback::new(callback);
        self
    }

    /// Performs the handshake
    fn handshake_with_transport<T: Transport>(&mut self, transport: &T) -> Result<()> {
        // No need to handshake twice
        if self.handshake.is_some() {
            return Ok(());
        }

        let mut url = self.url.clone();

        let handshake: HandshakePacket = Packet::try_from(transport.poll()?)?.try_into()?;

        // update the base_url with the new sid
        url.query_pairs_mut().append_pair("sid", &handshake.sid[..]);

        self.handshake = Some(handshake);

        self.url = url;

        Ok(())
    }

    fn handshake(&mut self) -> Result<()> {
        if self.handshake.is_some() {
            return Ok(());
        }

        // Start with polling transport
        let transport = PollingTransport::new(
            self.url.clone(),
            self.tls_config.clone(),
            self.headers.clone().map(|v| v.try_into().unwrap()),
        );

        self.handshake_with_transport(&transport)
    }

    /// Build websocket if allowed, if not fall back to polling
    pub fn build(mut self) -> Result<Client> {
        self.handshake()?;

        if self.websocket_upgrade()? {
            self.build_websocket_with_upgrade()
        } else {
            self.build_polling()
        }
    }

    /// Build socket with polling transport
    pub fn build_polling(mut self) -> Result<Client> {
        self.handshake()?;

        // Make a polling transport with new sid
        let transport = PollingTransport::new(
            self.url,
            self.tls_config,
            self.headers.map(|v| v.try_into().unwrap()),
        );

        // SAFETY: handshake function called previously.
        Ok(Client {
            socket: Arc::new(V4Socket::new(
                transport.into(),
                self.handshake.unwrap(),
                self.on_close,
                self.on_data,
                self.on_error,
                self.on_open,
                self.on_packet,
            )
            ),
        })
    }

    /// Build socket with a polling transport then upgrade to websocket transport
    pub fn build_websocket_with_upgrade(mut self) -> Result<Client> {
        self.handshake()?;

        if self.websocket_upgrade()? {
            self.build_websocket()
        } else {
            Err(Error::IllegalWebsocketUpgrade())
        }
    }

    /// Build socket with only a websocket transport
    pub fn build_websocket(mut self) -> Result<Client> {
        // SAFETY: Already a Url
        let url = url::Url::parse(&self.url.to_string())?;

        let headers: Option<http::HeaderMap> = if let Some(map) = self.headers.clone() {
            Some(map.try_into()?)
        } else {
            None
        };

        match url.scheme() {
            "http" | "ws" => {
                let transport = WebsocketTransport::new(url, headers)?;
                if self.handshake.is_some() {
                    transport.upgrade()?;
                } else {
                    self.handshake_with_transport(&transport)?;
                }
                // NOTE: Although self.url contains the sid, it does not propagate to the transport
                // SAFETY: handshake function called previously.
                Ok(Client {
                    socket: Arc::new(V4Socket::new(
                        transport.into(),
                        self.handshake.unwrap(),
                        self.on_close,
                        self.on_data,
                        self.on_error,
                        self.on_open,
                        self.on_packet,
                    )),
                })
            }
            "https" | "wss" => {
                let transport =
                    WebsocketSecureTransport::new(url, self.tls_config.clone(), headers)?;
                if self.handshake.is_some() {
                    transport.upgrade()?;
                } else {
                    self.handshake_with_transport(&transport)?;
                }
                // NOTE: Although self.url contains the sid, it does not propagate to the transport
                // SAFETY: handshake function called previously.
                Ok(Client {
                    socket: Arc::new(V4Socket::new(
                        transport.into(),
                        self.handshake.unwrap(),
                        self.on_close,
                        self.on_data,
                        self.on_error,
                        self.on_open,
                        self.on_packet,
                    )),
                })
            }
            _ => Err(Error::InvalidUrlScheme(url.scheme().to_string())),
        }
    }

    /// Build websocket if allowed, if not allowed or errored fall back to polling.
    /// WARNING: websocket errors suppressed, no indication of websocket success or failure.
    pub fn build_with_fallback(self) -> Result<Client> {
        let result = self.clone().build();
        if result.is_err() {
            self.build_polling()
        } else {
            result
        }
    }

    /// Checks the handshake to see if websocket upgrades are allowed
    fn websocket_upgrade(&mut self) -> Result<bool> {
        // SAFETY: handshake set by above function.
        Ok(self
            .handshake
            .as_ref()
            .unwrap()
            .upgrades
            .iter()
            .any(|upgrade| upgrade.to_lowercase() == *"websocket"))
    }
}

#[cfg(test)]
mod test {
    use reqwest::header::HOST;

    use crate::packet::Packet;
    use crate::packet::PacketId;

    use super::*;

    #[test]
    fn test_illegal_actions() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let sut = builder(url.clone()).build()?;

        assert!(sut
            .emit(Packet::new(PacketId::Close, Bytes::new()))
            .is_err());

        sut.connect()?;

        assert!(sut.poll().is_ok());

        assert!(builder(Url::parse("fake://fake.fake").unwrap())
            .build_websocket()
            .is_err());

        Ok(())
    }

    fn builder(url: Url) -> ClientBuilder {
        ClientBuilder::new(url)
            .on_open(|_| {
                println!("Open event!");
            })
            .on_packet(|packet| {
                println!("Received packet: {:?}", packet);
            })
            .on_data(|data| {
                println!("Received data: {:?}", std::str::from_utf8(&data));
            })
            .on_close(|_| {
                println!("Close event!");
            })
            .on_error(|error| {
                println!("Error {}", error);
            })
    }


    fn test_connection(socket: Client) -> Result<()> {
        let socket = socket;

        socket.connect().unwrap();

        // TODO: 0.3.X better tests

        let mut iter = socket
            .iter()
            .map(|packet| packet.unwrap())
            .filter(|packet| packet.packet_id != PacketId::Ping);

        assert_eq!(
            iter.next(),
            Some(Packet::new(PacketId::Message, "hello client"))
        );

        socket.emit(Packet::new(PacketId::Message, "respond"))?;

        assert_eq!(
            iter.next(),
            Some(Packet::new(PacketId::Message, "Roger Roger"))
        );

        socket.close()
    }

    #[test]
    fn test_connection_long() -> Result<()> {
        // Long lived socket to receive pings
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build()?;

        socket.connect()?;

        let mut iter = socket.iter();
        // hello client
        iter.next();
        // Ping
        iter.next();

        socket.disconnect()?;

        assert!(!socket.is_connected()?);

        Ok(())
    }

    #[test]
    fn test_connection_dynamic() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build()?;
        test_connection(socket)?;

        let url = crate::test::engine_io_polling_server()?;
        let socket = builder(url).build()?;
        test_connection(socket)
    }

    #[test]
    fn test_connection_fallback() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build_with_fallback()?;
        test_connection(socket)?;

        let url = crate::test::engine_io_polling_server()?;
        let socket = builder(url).build_with_fallback()?;
        test_connection(socket)
    }

    #[test]
    fn test_connection_dynamic_secure() -> Result<()> {
        let url = crate::test::engine_io_server_secure()?;
        let mut builder = builder(url);
        builder = builder.tls_config(crate::test::tls_connector()?);
        let socket = builder.build()?;
        test_connection(socket)
    }

    #[test]
    fn test_connection_polling() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build_polling()?;
        test_connection(socket)
    }

    #[test]
    fn test_connection_wss() -> Result<()> {
        let url = crate::test::engine_io_polling_server()?;
        assert!(builder(url).build_websocket_with_upgrade().is_err());

        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        let mut url = crate::test::engine_io_server_secure()?;

        let mut headers = HeaderMap::default();
        headers.insert(HOST, host);
        let mut builder = builder(url.clone());

        builder = builder.tls_config(crate::test::tls_connector()?);
        builder = builder.headers(headers.clone());
        let socket = builder.clone().build_websocket_with_upgrade()?;

        test_connection(socket)?;

        let socket = builder.build_websocket()?;

        test_connection(socket)?;

        url.set_scheme("wss").unwrap();

        let builder = self::builder(url)
            .tls_config(crate::test::tls_connector()?)
            .headers(headers);
        let socket = builder.clone().build_websocket()?;

        test_connection(socket)?;

        assert!(builder.build_websocket_with_upgrade().is_err());

        Ok(())
    }

    #[test]
    fn test_connection_ws() -> Result<()> {
        let url = crate::test::engine_io_polling_server()?;
        assert!(builder(url.clone()).build_websocket().is_err());
        assert!(builder(url).build_websocket_with_upgrade().is_err());

        let mut url = crate::test::engine_io_server()?;

        let builder = builder(url.clone());
        let socket = builder.clone().build_websocket()?;
        test_connection(socket)?;

        let socket = builder.build_websocket_with_upgrade()?;
        test_connection(socket)?;

        url.set_scheme("ws").unwrap();

        let builder = self::builder(url);
        let socket = builder.clone().build_websocket()?;

        test_connection(socket)?;

        assert!(builder.build_websocket_with_upgrade().is_err());

        Ok(())
    }

    #[test]
    fn test_open_invariants() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let illegal_url = "this is illegal";

        assert!(Url::parse(illegal_url).is_err());

        let invalid_protocol = "file:///tmp/foo";
        assert!(builder(Url::parse(invalid_protocol).unwrap())
            .build()
            .is_err());

        let sut = builder(url.clone()).build()?;
        let _error = sut
            .emit(Packet::new(PacketId::Close, Bytes::new()))
            .expect_err("error");
        assert!(matches!(Error::IllegalActionBeforeOpen(), _error));

        // test missing match arm in socket constructor
        let mut headers = HeaderMap::default();
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        headers.insert(HOST, host);

        let _ = builder(url.clone())
            .tls_config(
                TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .build()
                    .unwrap(),
            )
            .build()?;
        let _ = builder(url).headers(headers).build()?;
        Ok(())
    }
}