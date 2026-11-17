//! Within `stream`, `recording` is the run's recording as the stream reads it: the tap the harness hands every byte
//! the recording's file takes, which tells the stream how far the file has grown, and the reader that takes the
//! file's frames from where it last stopped, each whole one to go out as one binary message, the scenario block first
//! and the end block last, so a page that watched the run gets the end-of-run figures a later reader of the file gets.
//! The index block, the container's bookkeeping, is passed over. The frames a viewer is sent are the recording's own
//! bytes, and a server that is away for a while costs the stream nothing but the wait: the file holds every frame
//! until the connection reads it.

use std::fmt::Display;
use std::future::pending;
use std::io::{Result as IoResult, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Notify;
use worldview::container::frame::HEADER_LEN;
use worldview::{FrameHeader, Tag};

/// How far the recording's file has grown, as the tap tells it, and the wake-up the reader waits on for more.
#[derive(Default)]
struct Growth {
    written: AtomicU64,
    grew: Notify,
}

/// The harness's tap on the run's recording (`RecordingTarget::tap`). The harness hands it each run of bytes after the
/// file took them, and it tells the stream the file's new length and nothing else: the file has the bytes. It never
/// refuses and never waits, so the stream neither stops nor slows the run.
#[derive(Clone)]
pub struct RecordingTap(Arc<Growth>);

impl Write for RecordingTap {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.0.written.fetch_add(bytes.len() as u64, Ordering::Release);
        self.0.grew.notify_one();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

/// The recording at `path` as the connection reads it: where the next frame to send begins, and what a new connection
/// says again, the scenario block (the file's first frame) and the open segment from its keyframe.
pub(super) struct RecordingReader {
    path: PathBuf,
    growth: Arc<Growth>,
    file: Option<File>,
    next: u64,
    /// The file length the next read needs: the next frame's header, then the whole frame once its header is read, and
    /// never once the end block is taken, since only the trailer, which is no frame, follows it.
    wanted: u64,
    scenario_len: Option<u64>,
    keyframe_at: Option<u64>,
    /// The file could not be read, which was said once: nothing more is read from it.
    failed: bool,
}

impl RecordingReader {
    /// The reader of the recording at `path`, and the tap that tells it the file grew.
    pub(super) fn new(path: PathBuf) -> (RecordingTap, Self) {
        let growth = Arc::new(Growth::default());
        let reader = Self {
            path,
            growth: growth.clone(),
            file: None,
            next: 0,
            wanted: HEADER_LEN as u64,
            scenario_len: None,
            keyframe_at: None,
            failed: false,
        };
        (RecordingTap(growth), reader)
    }

    /// Ready once the file has grown as far as the next read needs; never, past the end block or a read that failed.
    /// Waiting can be given up and taken again without missing a write.
    pub(super) async fn readable(&self) {
        if self.failed {
            return pending().await;
        }
        while self.growth.written.load(Ordering::Acquire) < self.wanted {
            self.growth.grew.notified().await;
        }
    }

    /// The next frame the file holds whole past the last one taken, read from the file, or none: it is not whole yet,
    /// the end block went, or the file could not be read. The index block is passed over unread. The end block of a run
    /// stopped before its first slice goes too, with no keyframe ahead of it.
    pub(super) async fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            let written = self.growth.written.load(Ordering::Acquire);
            if self.failed || written < self.wanted {
                return None;
            }
            let header = self.read(self.next, HEADER_LEN).await?;
            let header =
                match FrameHeader::decode_at(header.as_slice().try_into().expect("a header's bytes"), self.next) {
                    Ok(header) => header,
                    Err(fault) => return self.give_up(&fault),
                };
            let len = header.frame_len();
            if header.tag == Tag::Index {
                self.next += len;
                self.wanted = self.next + HEADER_LEN as u64;
                continue;
            }
            if written < self.next + len {
                self.wanted = self.next + len;
                return None;
            }
            let frame = self.read(self.next, len as usize).await?;
            match header.tag {
                Tag::Scenario => self.scenario_len = Some(len),
                Tag::Keyframe => self.keyframe_at = Some(self.next),
                _ => {}
            }
            self.next += len;
            self.wanted = if header.tag == Tag::End { u64::MAX } else { self.next + HEADER_LEN as u64 };
            return Some(frame);
        }
    }

    /// A new connection's beginning, once the old one may have lost what it was sent: the scenario block again, when it
    /// went out before, and the reader set back to the open segment's keyframe, so that segment goes out again and the
    /// server starts it over (`run_stream`'s binary rule). A recording with no keyframe yet, a run stopped before its
    /// first slice among them, is set back to just past its scenario block, so its end block goes out again whether or
    /// not the lost connection delivered it; the server passes over one it holds.
    pub(super) async fn rewound(&mut self) -> Option<Vec<u8>> {
        let len = self.scenario_len.filter(|_| !self.failed)?;
        self.next = self.keyframe_at.unwrap_or(len);
        self.wanted = self.next + HEADER_LEN as u64;
        self.read(0, len as usize).await
    }

    async fn read(&mut self, at: u64, len: usize) -> Option<Vec<u8>> {
        let mut bytes = vec![0; len];
        let read = async {
            if self.file.is_none() {
                self.file = Some(File::open(&self.path).await?);
            }
            let file = self.file.as_mut().expect("opened above");
            file.seek(SeekFrom::Start(at)).await?;
            file.read_exact(&mut bytes).await
        };
        match read.await {
            Ok(_) => Some(bytes),
            Err(fault) => self.give_up(&fault),
        }
    }

    fn give_up<T>(&mut self, fault: &dyn Display) -> Option<T> {
        eprintln!(
            "simrunner: the stream stops reading the recording at {}: {fault}; the run and its recording go on",
            self.path.display()
        );
        self.failed = true;
        None
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::ops::Range;
    use std::path::Path;
    use std::slice::from_ref;
    use std::time::Duration;

    use tempfile::tempdir;
    use tokio::time::timeout;
    use worldview::{Frame, Ms, Tag, Writer};

    use super::{RecordingReader, RecordingTap};

    fn frame(tag: Tag, t: u64) -> Vec<u8> {
        Frame::encode(tag, Ms(t), format!("{tag:?} at {t}").as_bytes(), true).expect("a frame")
    }

    /// `frames` closed as dessim's writer closes a recording, the index block, the end block and the trailer after
    /// them: the whole file, and where its end block lies in it.
    fn finished(frames: &[Vec<u8>], t_last: u64) -> (Vec<u8>, Range<usize>) {
        let mut writer = Writer::new(Vec::new());
        for bytes in frames {
            writer.append(&Frame::decode_at(bytes, 0).expect("a frame").0).expect("appended");
        }
        let file = writer.finish(Ms(t_last), b"the run's end").expect("closed");
        let index_at = frames.iter().map(Vec::len).sum::<usize>();
        let (index, index_len) = Frame::decode_at(&file[index_at..], 0).expect("the index block");
        let (end, end_len) = Frame::decode_at(&file[index_at + index_len..], 0).expect("the end block");
        assert_eq!((index.header.tag, end.header.tag), (Tag::Index, Tag::End), "the frames that close a file");
        let end_at = index_at + index_len;
        (file, end_at..end_at + end_len)
    }

    /// `bytes` appended to the file at `path` and handed to the tap, as the harness writes a recording.
    fn written(path: &Path, tap: &mut RecordingTap, bytes: &[u8]) {
        let mut file = OpenOptions::new().create(true).append(true).open(path).expect("the file");
        file.write_all(bytes).expect("appended");
        tap.write_all(bytes).expect("the tap never refuses");
    }

    async fn frames(reader: &mut RecordingReader) -> Vec<Vec<u8>> {
        let mut taken = Vec::new();
        while let Some(frame) = reader.next_frame().await {
            taken.push(frame);
        }
        taken
    }

    /// A frame is taken once its last byte is told, not before; the index block is passed over, the end block taken
    /// last, and the trailer after it never read as a frame.
    #[tokio::test]
    async fn a_frame_is_read_once_the_tap_told_its_last_byte_the_index_block_never_and_the_end_block_last() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recording.worldview");
        let (mut tap, mut reader) = RecordingReader::new(path.clone());
        let (scenario, keyframe, delta) = (frame(Tag::Scenario, 0), frame(Tag::Keyframe, 0), frame(Tag::Delta, 0));
        let (file, end) = finished(&[scenario.clone(), keyframe.clone(), delta.clone()], 0);
        written(&path, &mut tap, &scenario[..10]);
        assert!(frames(&mut reader).await.is_empty(), "not even its header is whole");
        written(&path, &mut tap, &scenario[10..scenario.len() - 1]);
        assert!(frames(&mut reader).await.is_empty(), "its last byte is not in");
        written(&path, &mut tap, &scenario[scenario.len() - 1..]);
        timeout(Duration::from_secs(1), reader.readable()).await.expect("the reader is woken");
        assert_eq!(frames(&mut reader).await, from_ref(&scenario));
        written(&path, &mut tap, &file[scenario.len()..end.end - 1]);
        assert_eq!(frames(&mut reader).await, [keyframe, delta], "the index passed over, the end block not whole");
        written(&path, &mut tap, &file[end.end - 1..]);
        assert!(file.len() > end.end, "a trailer follows the end block");
        assert_eq!(frames(&mut reader).await, [&file[end]], "the end block last");
        assert!(timeout(Duration::from_millis(50), reader.readable()).await.is_err(), "the trailer is no frame");
    }

    /// A run stopped before its first slice leaves no keyframe, and its end block goes after the scenario block all
    /// the same, so a page that watched the run gets its end-of-run figures and its stop reason.
    #[tokio::test]
    async fn the_end_block_of_a_run_stopped_before_its_first_slice_goes_after_the_scenario_block() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recording.worldview");
        let (mut tap, mut reader) = RecordingReader::new(path.clone());
        let scenario = frame(Tag::Scenario, 0);
        let (file, end) = finished(from_ref(&scenario), 0);
        written(&path, &mut tap, &file);
        assert_eq!(frames(&mut reader).await, [scenario, file[end].to_vec()]);
        assert!(timeout(Duration::from_millis(50), reader.readable()).await.is_err(), "nothing is left to wait for");
    }

    /// After a new connection the scenario block comes again and the reading goes back to the open segment's keyframe,
    /// so the frames since it come again, then the ones not yet sent.
    #[tokio::test]
    async fn a_rewind_says_the_scenario_block_again_and_goes_back_to_the_open_segments_keyframe() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recording.worldview");
        let (mut tap, mut reader) = RecordingReader::new(path.clone());
        let segment_0 = [frame(Tag::Keyframe, 0), frame(Tag::Delta, 0), frame(Tag::Delta, 1_000)];
        let segment_1 = [frame(Tag::Keyframe, 2_000), frame(Tag::Delta, 2_000), frame(Tag::Declare, 3_000)];
        let scenario = frame(Tag::Scenario, 0);
        written(&path, &mut tap, &[&scenario[..], &segment_0.concat(), &segment_1.concat()].concat());
        assert_eq!(frames(&mut reader).await.len(), 7, "all of them sent");
        let later = frame(Tag::Delta, 3_000);
        written(&path, &mut tap, &later);
        assert_eq!(reader.rewound().await, Some(scenario));
        assert_eq!(frames(&mut reader).await, [&segment_1[..], &[later]].concat());
    }

    /// A connection lost after the end block went: the open segment comes again from its keyframe, its end block with
    /// it, which the server takes as that segment started over.
    #[tokio::test]
    async fn a_rewind_after_the_end_block_says_the_open_segment_again_with_its_end_block() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recording.worldview");
        let (mut tap, mut reader) = RecordingReader::new(path.clone());
        let scenario = frame(Tag::Scenario, 0);
        let segment_0 = [frame(Tag::Keyframe, 0), frame(Tag::Delta, 0)];
        let segment_1 = [frame(Tag::Keyframe, 1_000), frame(Tag::Delta, 1_000)];
        let recorded = [&[scenario.clone()][..], &segment_0, &segment_1].concat();
        let (file, end) = finished(&recorded, 1_500);
        written(&path, &mut tap, &file);
        let end = file[end].to_vec();
        assert_eq!(frames(&mut reader).await, [&recorded[..], from_ref(&end)].concat());
        assert_eq!(reader.rewound().await, Some(scenario));
        assert_eq!(frames(&mut reader).await, [&segment_1[..], &[end]].concat());
    }

    /// A run stopped before its first slice: a connection lost as its end block went, delivered or not, sends the
    /// scenario block and the end block again.
    #[tokio::test]
    async fn a_rewind_of_a_run_stopped_before_its_first_slice_says_its_end_block_again() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recording.worldview");
        let (mut tap, mut reader) = RecordingReader::new(path.clone());
        let scenario = frame(Tag::Scenario, 0);
        let (file, end) = finished(from_ref(&scenario), 0);
        written(&path, &mut tap, &file);
        let end = file[end].to_vec();
        assert_eq!(frames(&mut reader).await, [scenario.clone(), end.clone()]);
        assert_eq!(reader.rewound().await, Some(scenario));
        timeout(Duration::from_secs(1), reader.readable()).await.expect("the end block is to be read again");
        assert_eq!(frames(&mut reader).await, [end], "the end block again, and only it");
    }

    /// Nothing went out yet, so a new connection has nothing to say again.
    #[tokio::test]
    async fn a_rewind_before_the_scenario_block_went_out_says_nothing() {
        let dir = tempdir().unwrap();
        let (_tap, mut reader) = RecordingReader::new(dir.path().join("recording.worldview"));
        assert_eq!(reader.rewound().await, None);
        assert!(reader.next_frame().await.is_none(), "and nothing is read from a file the run has not written");
    }
}
