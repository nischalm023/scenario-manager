//! Within `live_run`, `tap` is HOW A HEADED RUN REACHES THE WINDOW: the
//! recording's bytes, as the file takes them, and how the run ended, sent from
//! the run's thread in that order.

use std::io::{Error as IoError, ErrorKind, Result as IoResult, Write};
use std::sync::mpsc::Sender;

/// What a headed run's thread sends the window, in order: the recording's
/// bytes as the file takes them, then, once `execute` has returned or
/// panicked, how the run ended (`Ok`, or the failure's words).
pub(crate) enum FromRun {
    Bytes(Vec<u8>),
    Ended(Result<(), String>),
}

/// The tap a headed run's recording is written through: each write's bytes go
/// to the window as they go to the file.
pub(crate) struct RunTap(pub(crate) Sender<FromRun>);

impl Write for RunTap {
    /// A window that has stopped watching refuses the bytes, which stops the
    /// run as a full disk does: nothing would read what it went on to write.
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.0
            .send(FromRun::Bytes(bytes.to_vec()))
            .map_err(|_| IoError::new(ErrorKind::BrokenPipe, "the window stopped watching the run"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}
