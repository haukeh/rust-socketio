use std::{fmt::Debug, sync::atomic::Ordering};
use std::{
    sync::{Arc, atomic::AtomicBool, Mutex},
    time::Instant,
};
use std::convert::TryFrom;
use std::fmt::Formatter;
use std::sync::RwLock;

use bytes::Bytes;

use crate::callback::OptionalCallback;
use crate::error::{Error, Result};
use crate::packet::{HandshakePacket, Packet, PacketId, Payload};
use crate::transport::TransportType;

pub(crate) trait Socket {
    fn connect(&self) -> Result<()>;
    fn is_connected(&self) -> Result<bool>;
    fn emit(&self, packet: Packet) -> Result<()>;
    fn poll(&self) -> Result<Option<Packet>>;
    fn close_connection(&self) -> Result<()>;
    fn handle_ping(&self) -> Result<()>;
    fn handle_pong(&self) -> Result<()>;

    fn on_close(&self) -> OptionalCallback<()> { OptionalCallback::default() }
    fn on_data(&self) -> OptionalCallback<Bytes> { OptionalCallback::default() }
    fn on_error(&self) -> OptionalCallback<String> { OptionalCallback::default() }
    fn on_open(&self) -> OptionalCallback<()> { OptionalCallback::default() }
    fn on_packet(&self) -> OptionalCallback<Packet> { OptionalCallback::default() }

    fn disconnect(&self) -> Result<()> {
        if let Some(on_close) = self.on_close().as_ref() {
            spawn_scoped!(on_close(()));
        }

        self.emit(Packet::new(PacketId::Close, Bytes::new()))?;

        return self.close_connection();
    }

    /// Calls the error callback with a given message.
    #[inline]
    fn call_error_callback(&self, text: String) {
        if let Some(function) = self.on_error().as_ref() {
            spawn_scoped!(function(text));
        }
    }

    fn handle_packet(&self, packet: Packet) {
        if let Some(on_packet) = self.on_packet().as_ref() {
            spawn_scoped!(on_packet(packet));
        }
    }

    fn handle_data(&self, data: Bytes) {
        if let Some(on_data) = self.on_data().as_ref() {
            spawn_scoped!(on_data(data));
        }
    }
}

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
pub struct V4Socket {
    transport: Arc<TransportType>,
    on_close: OptionalCallback<()>,
    on_data: OptionalCallback<Bytes>,
    on_error: OptionalCallback<String>,
    on_open: OptionalCallback<()>,
    on_packet: OptionalCallback<Packet>,
    connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    connection_data: Arc<HandshakePacket>,
    /// Since we get packets in payloads it's possible to have a state where only some of the packets have been consumed.
    remaining_packets: Arc<RwLock<Option<crate::packet::IntoIter>>>,
}

impl V4Socket {
    pub(crate) fn new(
        transport: TransportType,
        handshake: HandshakePacket,
        on_close: OptionalCallback<()>,
        on_data: OptionalCallback<Bytes>,
        on_error: OptionalCallback<String>,
        on_open: OptionalCallback<()>,
        on_packet: OptionalCallback<Packet>,
    ) -> Self {
        V4Socket {
            on_close,
            on_data,
            on_error,
            on_open,
            on_packet,
            transport: Arc::new(transport),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            connection_data: Arc::new(handshake),
            remaining_packets: Arc::new(RwLock::new(None)),
        }
    }
}

impl Socket for V4Socket {
    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    fn connect(&self) -> Result<()> {
        // SAFETY: Has valid handshake due to type
        self.connected.store(true, Ordering::Release);

        if let Some(on_open) = self.on_open.as_ref() {
            spawn_scoped!(on_open(()));
        }

        // set the last ping to now and set the connected state
        *self.last_ping.lock()? = Instant::now();

        // emit a pong packet to keep trigger the ping cycle on the server
        self.emit(Packet::new(PacketId::Pong, Bytes::new()))?;

        Ok(())
    }

    fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }

    /// Sends a packet to the server.
    fn emit(&self, packet: Packet) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
            self.call_error_callback(format!("{}", error));
            return Err(error);
        }

        let is_binary = packet.packet_id == PacketId::MessageBinary;

        // send a post request with the encoded payload as body
        // if this is a binary attachment, then send the raw bytes
        let data: Bytes = if is_binary {
            packet.data
        } else {
            packet.into()
        };

        if let Err(error) = self.transport.as_transport().emit(data, is_binary) {
            self.call_error_callback(error.to_string());
            return Err(error);
        }

        Ok(())
    }

    /// Polls for next payload
    fn poll(&self) -> Result<Option<Packet>> {
        loop {
            if self.connected.load(Ordering::Acquire) {
                if self.remaining_packets.read()?.is_some() {
                    // SAFETY: checked is some above
                    let mut iter = self.remaining_packets.write()?;
                    let iter = iter.as_mut().unwrap();
                    if let Some(packet) = iter.next() {
                        return Ok(Some(packet));
                    }
                }

                // Iterator has run out of packets, get a new payload
                // TODO: 0.3.X timeout?
                let data = self.transport.as_transport().poll()?;

                if data.is_empty() {
                    continue;
                }

                let payload = Payload::try_from(data)?;
                let mut iter = payload.into_iter();

                if let Some(packet) = iter.next() {
                    *self.remaining_packets.write()? = Some(iter);
                    return Ok(Some(packet));
                }
            } else {
                return Ok(None);
            }
        }
    }

    fn close_connection(&self) -> Result<()> {
        if let Some(on_close) = self.on_close().as_ref() {
            spawn_scoped!(on_close);
        }

        self.connected.store(false, Ordering::Release);

        Ok(())
    }

    fn handle_ping(&self) -> Result<()> {
        *self.last_ping.lock()? = Instant::now();
        self.emit(Packet::new(PacketId::Pong, Bytes::new()))
    }

    fn handle_pong(&self) -> Result<()> {
        unreachable!("Should never be called, because in engine.io protocol v4 pong packets are only sent by the client")
    }

    fn on_close(&self) -> OptionalCallback<()> {
        self.on_close.clone()
    }

    fn on_data(&self) -> OptionalCallback<Bytes> {
        self.on_data.clone()
    }

    fn on_error(&self) -> OptionalCallback<String> {
        self.on_error.clone()
    }

    fn on_open(&self) -> OptionalCallback<()> {
        self.on_open.clone()
    }

    fn on_packet(&self) -> OptionalCallback<Packet> {
        self.on_packet.clone()
    }
}

#[cfg_attr(tarpaulin, ignore)]
impl Debug for V4Socket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "EngineSocket(transport: {:?}, on_error: {:?}, on_open: {:?}, on_close: {:?}, on_packet: {:?}, on_data: {:?}, connected: {:?}, last_ping: {:?}, last_pong: {:?}, connection_data: {:?})",
            self.transport,
            self.on_error,
            self.on_open,
            self.on_close,
            self.on_packet,
            self.on_data,
            self.connected,
            self.last_ping,
            self.last_pong,
            self.connection_data,
        ))
    }
}
