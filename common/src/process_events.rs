//! Common code for event processing.

use nanomsg::{Socket, NanoError, NanoErrorKind};
use rustc_serialize;
use std::convert::AsRef;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Process all available events from a channel.
// TODO: This could probably be an iterator.
pub fn process_receiver<T, Apply>(chan: &Receiver<T>, mut apply: Apply) -> bool
  where
    T: Send + 'static,
    Apply: FnMut(T) -> bool,
{
  loop {
    match chan.try_recv() {
      Err(TryRecvError::Empty) => break,
      Err(e) => panic!("Error receiving from channel: {:?}", e),
      Ok(update) => {
        if !apply(update) {
          return false
        }
      },
    }
  }

  true
}

/// Process all available events from a socket.
// TODO: This could probably be an iterator.
pub fn process_socket<T, Apply>(socket: &mut Socket, mut apply: Apply) -> bool
  where
    T: rustc_serialize::Decodable,
    Apply: FnMut(T) -> bool,
{
  loop {
    let mut s = Vec::new();
    match socket.nb_read_to_end(&mut s) {
      Err(NanoError { kind: NanoErrorKind::TryAgain, ..}) => {
        // No more messages.
        return true
      }
      Err(e) => panic!("Error getting message from socket: {:?}", e),
      Ok(_) => {
        let s = String::from_utf8(s).unwrap();
        let update = rustc_serialize::json::decode(s.as_ref()).unwrap();
        if !apply(update) {
          return false
        }
      },
    }
  }
}