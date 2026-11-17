//! Within `write`, `tap` is how a reader in the same process takes the
//! recording as it is written: a sink that hands it every byte the
//! recording's file takes, in the order the file takes them. A viewer
//! watching a run reads the recording itself this way, never a channel of
//! its own.

use std::io::{Result as IoResult, Write};

use worldview::container::writer::AppendSink;

/// The recording's file with a tap on it. Each write lands in the file first,
/// then in the tap, which is handed the bytes the file took and no others, so
/// the tap never holds a byte the file lacks. A tap that refuses fails the
/// write as the file refusing would: the writer stops for good, and the
/// recording fails, which stops the run.
pub(super) struct Tapped<F> {
    file: F,
    tap: Box<dyn Write + Send>,
}

impl<F> Tapped<F> {
    pub(super) fn new(file: F, tap: Box<dyn Write + Send>) -> Self {
        Self { file, tap }
    }
}

impl<F: Write> Write for Tapped<F> {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        let taken = self.file.write(bytes)?;
        self.tap.write_all(&bytes[..taken])?;
        Ok(taken)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.file.flush()?;
        self.tap.flush()
    }
}

/// Only the file is made durable: what the tap holds does not outlive the
/// process.
impl<F: AppendSink> AppendSink for Tapped<F> {
    fn durable(&mut self) -> IoResult<()> {
        self.file.durable()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind};
    use std::sync::{Arc, Mutex};

    use worldview::container::Ms;
    use worldview::container::writer::Writer;

    use super::*;

    /// A sink that keeps what it takes where the test can read it, until it
    /// has taken `room` bytes: the write that would pass them takes what fits
    /// and the next is refused, as a disk that fills up does.
    #[derive(Clone)]
    struct Fills {
        held: Arc<Mutex<Vec<u8>>>,
        room: usize,
    }

    impl Fills {
        fn with_room(room: usize) -> Self {
            Self { held: Arc::default(), room }
        }

        fn held(&self) -> Vec<u8> {
            self.held.lock().unwrap().clone()
        }
    }

    impl Write for Fills {
        fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
            let mut held = self.held.lock().unwrap();
            let fits = bytes.len().min(self.room - held.len());
            if fits == 0 && !bytes.is_empty() {
                return Err(IoError::new(ErrorKind::StorageFull, "the sink is full"));
            }
            held.extend_from_slice(&bytes[..fits]);
            Ok(fits)
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    impl AppendSink for Fills {}

    /// A file that fills part way through a frame holds the frame's first
    /// bytes, and the tap holds exactly those: it is never handed what the
    /// file refused.
    #[test]
    fn the_tap_holds_what_the_file_took_and_nothing_it_refused() {
        let (file, tap) = (Fills::with_room(10), Fills::with_room(usize::MAX));
        let mut writer = Writer::new(Tapped::new(file.clone(), Box::new(tap.clone())));
        writer.append_scenario(Ms::ZERO, b"a run longer than the file has room for").expect_err("the file fills");
        assert_eq!(file.held().len(), 10, "the file took what fitted");
        assert_eq!(tap.held(), file.held());
    }

    /// A tap that refuses fails the write that handed it the bytes, as the
    /// file refusing them would, and the writer appends nothing more.
    #[test]
    fn a_tap_that_refuses_fails_the_write_and_stops_the_writer() {
        let (file, tap) = (Fills::with_room(usize::MAX), Fills::with_room(0));
        let mut writer = Writer::new(Tapped::new(file, Box::new(tap)));
        let refused = writer.append_scenario(Ms::ZERO, b"the run").expect_err("the tap refuses the frame");
        assert!(refused.to_string().contains("the sink is full"), "the tap's own words: {refused}");
        let after = writer.append_scenario(Ms::ZERO, b"the run").expect_err("the writer stopped for good");
        assert!(after.to_string().contains("appends nothing more"), "{after}");
    }
}
