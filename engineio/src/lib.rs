#![allow(clippy::rc_buffer)]
#![warn(clippy::complexity)]
#![warn(clippy::style)]
#![warn(clippy::perf)]
#![warn(clippy::correctness)]
/// A small macro that spawns a scoped thread. Used for calling the callback
/// functions.
macro_rules! spawn_scoped {
    ($e:expr) => {
        crossbeam_utils::thread::scope(|s| {
            s.spawn(|_| $e);
        })
        .unwrap();
    };
}

mod common;

#[cfg(feature = "v4")]
mod v4;
#[cfg(feature = "v4")]
pub use v4::client::{Client, ClientBuilder};
#[cfg(feature = "v4")]
pub use v4::packet::{Packet, PacketId};
#[cfg(feature = "v4")]
pub use common::header::{HeaderMap, HeaderValue};

/// Contains the error type which will be returned with every result in this
/// crate. Handles all kinds of errors.
pub mod error;

pub use error::Error;