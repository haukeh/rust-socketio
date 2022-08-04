use std::convert::TryFrom;
use std::convert::TryInto;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytes::Bytes;
use native_tls::TlsConnector;
use url::Url;

use crate::callback::OptionalCallback;
use crate::ENGINE_IO_VERSION;
use crate::error::{Error, Result};
use crate::header::HeaderMap;
use crate::packet::{HandshakePacket, Packet, PacketId};
use crate::socket::{Socket as EngineSocket, V4Socket};
use crate::transport::Transport;
use crate::transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport};

pub struct Client {
    pub(crate) socket: Arc<dyn EngineSocket + Sync + Send>,
}

impl Client {
    pub fn close(&self) -> Result<()> {
        self.socket.disconnect()
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to tr igger the Ping-cycle.
    pub fn connect(&self) -> Result<()> {
        self.socket.connect()
    }

    /// Disconnects the connection.
    pub fn disconnect(&self) -> Result<()> {
        self.socket.disconnect()
    }

    /// Sends a packet to the server.
    pub fn emit(&self, packet: Packet) -> Result<()> {
        self.socket.emit(packet)
    }

    /// Polls for next payload
    #[doc(hidden)]
    pub fn poll(&self) -> Result<Option<Packet>> {
        let packet = self.socket.poll()?;
        if let Some(packet) = packet {
            // check for the appropriate action or callback
            self.socket.handle_packet(packet.clone());
            match packet.packet_id {
                PacketId::MessageBinary => {
                    self.socket.handle_data(packet.data.clone());
                }
                PacketId::Message => {
                    self.socket.handle_data(packet.data.clone());
                }
                PacketId::Close => {
                    self.socket.close_connection()?;
                }
                PacketId::Open => {
                    unreachable!("Won't happen as we open the connection beforehand");
                }
                PacketId::Upgrade => {
                    // this is already checked during the handshake, so just do nothing here
                }
                PacketId::Ping => {
                    self.socket.handle_ping()?;
                }
                PacketId::Pong => {
                    self.socket.handle_pong()?;
                }
                PacketId::Noop => (),
            }
            Ok(Some(packet))
        } else {
            Ok(None)
        }
    }

    /// Check if the underlying transport client is connected.
    pub fn is_connected(&self) -> Result<bool> {
        self.socket.is_connected()
    }

    pub fn iter(&self) -> Iter {
        Iter { socket: self }
    }
}

#[derive(Clone)]
pub struct Iter<'a> {
    socket: &'a Client,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<Packet>;
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        match self.socket.poll() {
            Ok(Some(packet)) => Some(Ok(packet)),
            Ok(None) => None,
            Err(err) => Some(Err(err)),
        }
    }
}
