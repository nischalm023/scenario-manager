//! The provider's interleavings, decided by the test rather than by timing: a store whose every ranged read waits for
//! a permit the test hands out, and fails or panics from a chosen read on, holds attempt 1's uploaded picture; readers
//! open the cold attempt and follow its one fetch as each object lands, see a failed fetch as a fault, find a finished
//! fetch's file, never keep a copy cut short in the store, outlive a fetch whose task died, and keep the sweeper off a
//! folder they hold; an open attempt is read from memory, then live until it ends; a burst leaves no reader behind;
//! and a disk copy keeps its own word on whether it is whole: live segments whose uploads never finished are an
//! incomplete copy, one fetched while the uploads went on gives way to the whole run once they finished, and one its
//! server never vouched for gives way to the final recording.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use runstore::RunStorage;
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, live_scenario_key, live_segment_key, recording_key};
use studio_protocol::RunOutcome;
use tempfile::TempDir;
use tokio::time::timeout;
use worldview::{End, Frame, Ms, Payload, Tag, Writer};

use super::{Fault, Provider, Reading, Unreached, ended_outcome};
use crate::cache::Known;
use crate::frames::tests::{frame, key};
use crate::frames::{FrameStore, LIVE_FILE};
use crate::upload::Uploader;

#[path = "../../tests/support/gated.rs"]
mod gated;

use gated::Gated;

/// Segment `n`: a keyframe and three deltas, 30 s of simtime from `n × 30 s`.
fn segment(n: u64) -> Vec<Vec<u8>> {
    let start = n * 30_000;
    let mut frames = vec![frame(Tag::Keyframe, start, 64)];
    frames.extend((0..3).map(|slice| frame(Tag::Delta, start + slice * 10_000, 64)));
    frames
}

/// A provider over a scratch cache, its store gated and holding attempt 1's scenario block and `segments` segments,
/// each object one ranged read, the last segment closed by the end block; and where the tables say they are, the
/// uploads finished.
async fn cold(dir: &TempDir, segments: u32) -> (Provider, Arc<Gated>, Known) {
    let store = Arc::new(Gated::over(&dir.path().join("objects")));
    let (run, attempt) = (RunId::new(key(1).run.as_str()).unwrap(), StoredAttempt::FIRST);
    store.inner.put(&live_scenario_key(&run, attempt), frame(Tag::Scenario, 0, 0).into()).await.unwrap();
    for n in 0..segments {
        let mut object = segment(u64::from(n)).concat();
        if n + 1 == segments {
            object.extend(frame(Tag::End, u64::from(n) * 30_000 + 20_000, 64));
        }
        store.inner.put(&live_segment_key(&run, attempt, n), object.into()).await.unwrap();
    }
    let frames = FrameStore::new(dir.path().join("cache"), u64::MAX, Uploader::new(store.clone(), None));
    let known = Known {
        recording_key: None,
        scenario_key: Some(live_scenario_key(&run, attempt)),
        segments_uploaded: segments,
        uploads_finished: true,
        ended: true,
    };
    (Provider::new(frames), store, known)
}

/// The reader's next frame, or that the copy is whole (`None`), within a bound that only a broken test reaches.
async fn next(reading: &mut Reading) -> Result<Option<Tag>, Fault> {
    timeout(Duration::from_secs(5), reading.next())
        .await
        .expect("the reading went on")
        .map(|frame| frame.map(|f| f.tag))
}

/// Every frame the reader takes from here to the copy's end.
async fn rest(reading: &mut Reading) -> Result<Vec<Tag>, Fault> {
    let mut tags = Vec::new();
    while let Some(tag) = next(reading).await? {
        tags.push(tag);
    }
    Ok(tags)
}

fn whole_copy(segments: u64) -> Vec<Tag> {
    let mut tags = vec![Tag::Scenario];
    for _ in 0..segments {
        tags.extend([Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::Delta]);
    }
    tags.push(Tag::End);
    tags
}

/// The first reader's request starts the fetch, a second joins it, and both follow the one part file as each object
/// lands, the first not waiting for the whole download; the store is read once per object.
#[tokio::test]
async fn every_reader_of_a_cold_attempt_follows_its_one_fetch_as_it_lands() {
    let dir = TempDir::new().unwrap();
    let (provider, store, known) = cold(&dir, 2).await;
    let mut first = provider.open(&key(1), Ok(Some(known.clone()))).await.expect("a fetch starts");
    let mut second = provider.open(&key(1), Ok(Some(known))).await.expect("the fetch is joined");
    assert!(first.landing() && second.landing() && first.history().is_empty());
    store.release(1);
    assert_eq!(next(&mut first).await, Ok(Some(Tag::Scenario)), "the scenario block, before the rest landed");
    assert_eq!(next(&mut second).await, Ok(Some(Tag::Scenario)));
    store.release(2);
    assert_eq!(rest(&mut first).await.unwrap(), whole_copy(2)[1..]);
    assert_eq!(rest(&mut second).await.unwrap(), whole_copy(2)[1..]);
    assert_eq!(store.reads.load(Ordering::SeqCst), 3, "one read of each object, however many readers");
    let folder = dir.path().join("cache").join(key(1).run.as_str()).join("1");
    assert!(folder.join(LIVE_FILE).exists() && !folder.join("live.worldview.part").exists(), "renamed into place");
    // A reader opening the attempt once the fetch has landed and renamed its file reads that file, and no object again.
    let mut late = provider.open(&key(1), Ok(None)).await.expect("the disk copy");
    assert!(!late.landing() && !late.live());
    assert_eq!(late.history().iter().map(|frame| frame.tag).collect::<Vec<_>>(), whole_copy(2));
    assert_eq!(next(&mut late).await, Ok(None), "nothing after a whole copy");
    assert_eq!(store.reads.load(Ordering::SeqCst), 3);
}

/// A reader opening while the fetch lands takes what has landed as its history, the rest as it comes.
#[tokio::test]
async fn a_reader_opening_mid_fetch_holds_what_landed_and_follows_the_rest() {
    let dir = TempDir::new().unwrap();
    let (provider, store, known) = cold(&dir, 2).await;
    let mut first = provider.open(&key(1), Ok(Some(known))).await.unwrap();
    store.release(2);
    for expected in &whole_copy(2)[..5] {
        assert_eq!(next(&mut first).await, Ok(Some(*expected)), "the scenario block and segment 0");
    }
    let mut joining = provider.open(&key(1), Ok(None)).await.expect("the fetch in progress, known or not");
    let landed: Vec<Tag> = joining.history().iter().map(|frame| frame.tag).collect();
    assert_eq!(landed, whole_copy(2)[..5], "the history is what had landed");
    store.release(1);
    assert_eq!(rest(&mut joining).await.unwrap(), whole_copy(2)[5..]);
}

/// A fetch that fails after the scenario block landed reaches each reader as that failure, never as a copy that
/// ended, its part file gone; the next reader fetches again, and reads the copy whole.
#[tokio::test]
async fn a_fetch_failing_midway_fails_every_reader_and_the_next_fetches_again() {
    let dir = TempDir::new().unwrap();
    let (provider, store, known) = cold(&dir, 2).await;
    store.fail_from.store(1, Ordering::SeqCst);
    let mut first = provider.open(&key(1), Ok(Some(known.clone()))).await.unwrap();
    let mut second = provider.open(&key(1), Ok(Some(known.clone()))).await.unwrap();
    store.release(2);
    assert_eq!(next(&mut first).await, Ok(Some(Tag::Scenario)));
    let failed = rest(&mut first).await.expect_err("the fetch failed");
    assert!(matches!(&failed, Fault::Fetch(why) if why.contains("refused read 1")), "{failed:?}");
    assert_eq!(rest(&mut second).await.map_err(|fault| matches!(fault, Fault::Fetch(_))), Err(true));
    let folder = dir.path().join("cache").join(key(1).run.as_str()).join("1");
    assert!(!folder.join("live.worldview.part").exists() && !folder.join(LIVE_FILE).exists());
    store.fail_from.store(usize::MAX, Ordering::SeqCst);
    let mut again = provider.open(&key(1), Ok(Some(known))).await.expect("a fetch starts again");
    store.release(3);
    assert_eq!(rest(&mut again).await.unwrap(), whole_copy(2));
}

/// A copy the store holds cut short inside a frame fails its fetch as a damaged recording, which another fetch would
/// copy again and a viewer is refused for good, checked before the part file would be renamed into place, so no copy
/// is kept: the next reader fetches again and fails alike, and once the store holds the segment whole, a reader reads
/// the run whole.
#[tokio::test]
async fn a_copy_cut_short_in_the_store_fails_its_fetch_and_is_never_kept() {
    let dir = TempDir::new().unwrap();
    let (provider, store, known) = cold(&dir, 1).await;
    let (run, attempt) = (RunId::new(key(1).run.as_str()).unwrap(), StoredAttempt::FIRST);
    let segment_key = live_segment_key(&run, attempt, 0);
    let whole = store.inner.get(&segment_key).await.unwrap();
    store.inner.put(&segment_key, whole.slice(..whole.len() - 5)).await.unwrap();
    let folder = dir.path().join("cache").join(key(1).run.as_str()).join("1");
    for read in [2, 4] {
        let mut reading = provider.open(&key(1), Ok(Some(known.clone()))).await.expect("a fetch starts");
        store.release(2);
        let cut = rest(&mut reading).await.expect_err("the copy ends inside a frame");
        assert!(matches!(&cut, Fault::Unreadable(why) if why.contains("ends inside a frame")), "{cut:?}");
        assert_eq!(store.reads.load(Ordering::SeqCst), read, "fetched again, nothing kept from the last");
        assert!(!folder.join(LIVE_FILE).exists() && !folder.join("live.worldview.part").exists());
    }
    store.inner.put(&segment_key, whole).await.unwrap();
    let mut reading = provider.open(&key(1), Ok(Some(known))).await.expect("a fetch starts");
    store.release(2);
    assert_eq!(rest(&mut reading).await.unwrap(), whole_copy(1));
}

/// A fetch whose task dies, here of a panic in the store's client, holds the folder no longer: its reader is told the
/// fetch stopped, the sweeper may take the folder, and the next reader fetches again rather than follow the dead one.
#[tokio::test]
async fn a_fetch_whose_task_dies_holds_its_folder_no_longer() {
    let dir = TempDir::new().unwrap();
    let (provider, store, known) = cold(&dir, 1).await;
    store.panic_from.store(1, Ordering::SeqCst);
    let mut dying = provider.open(&key(1), Ok(Some(known.clone()))).await.expect("a fetch starts");
    store.release(2);
    assert_eq!(next(&mut dying).await, Ok(Some(Tag::Scenario)));
    let stopped = rest(&mut dying).await.expect_err("the task died");
    assert_eq!(stopped, Fault::Fetch("the fetch stopped before it finished".to_owned()));
    drop(dying);
    assert!(!provider.in_use(&key(1)), "the dead fetch holds nothing");
    store.panic_from.store(usize::MAX, Ordering::SeqCst);
    let mut again = provider.open(&key(1), Ok(Some(known))).await.expect("a new fetch starts");
    assert!(again.landing());
    store.release(2);
    assert_eq!(rest(&mut again).await.unwrap(), whole_copy(1));
}

/// A cold attempt the tables name nothing of in the store is not yet while it goes and nowhere once it has ended, or
/// when the tables know no such attempt; one whose lookup failed says so.
#[tokio::test]
async fn an_attempt_neither_here_nor_named_in_the_store_is_unreached() {
    let dir = TempDir::new().unwrap();
    let (provider, _store, _known) = cold(&dir, 1).await;
    let going =
        Known { recording_key: None, scenario_key: None, segments_uploaded: 0, uploads_finished: false, ended: false };
    assert_eq!(provider.open(&key(1), Ok(Some(going.clone()))).await.err(), Some(Unreached::NotYet));
    let ended = Known { ended: true, ..going };
    assert_eq!(provider.open(&key(1), Ok(Some(ended))).await.err(), Some(Unreached::Nowhere));
    assert_eq!(provider.open(&key(1), Ok(None)).await.err(), Some(Unreached::Nowhere));
    let down = provider.open(&key(1), Err("the database is down".to_owned())).await.err();
    assert_eq!(down, Some(Unreached::Database("the database is down".to_owned())));
}

/// The sweeper takes a folder only once no reading holds it, and the folders nobody holds are forgotten.
#[tokio::test]
async fn the_sweeper_spares_a_folder_a_reading_holds() {
    let dir = TempDir::new().unwrap();
    let (provider, store, known) = cold(&dir, 1).await;
    let mut fetching = provider.open(&key(1), Ok(Some(known))).await.unwrap();
    assert!(provider.in_use(&key(1)), "a fetch holds it");
    assert!(!provider.sweep(&key(1)).await.unwrap());
    store.release(2);
    rest(&mut fetching).await.unwrap();
    assert!(provider.in_use(&key(1)), "the reading that followed the fetch still holds it");
    assert!(!provider.sweep(&key(1)).await.unwrap());
    drop(fetching);
    assert!(!provider.in_use(&key(1)));
    assert!(provider.sweep(&key(1)).await.unwrap());
    assert!(!dir.path().join("cache").join(key(1).run.as_str()).join("1").exists());
    provider.forget_idle();
    assert_eq!(provider.open(&key(1), Ok(None)).await.err(), Some(Unreached::Nowhere), "swept, and named nowhere");
}

/// An open attempt is read from memory: the frames so far, then each one live, until its ending; before its scenario
/// block it has nothing to read yet.
#[tokio::test]
async fn an_open_attempt_is_read_from_memory_then_live_until_it_ends() {
    let dir = TempDir::new().unwrap();
    let (provider, _store, _known) = cold(&dir, 0).await;
    let frames = provider.inner.frames.open(&key(1));
    assert_eq!(provider.open(&key(1), Ok(None)).await.err(), Some(Unreached::NotYet));
    for (tag, at) in [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0)] {
        frames.push(frame(tag, at, 64)).await.unwrap();
    }
    let mut reading = provider.open(&key(1), Ok(None)).await.expect("in memory");
    assert!(reading.live());
    assert_eq!(
        reading.history().iter().map(|frame| frame.tag).collect::<Vec<_>>(),
        [Tag::Scenario, Tag::Keyframe, Tag::Delta]
    );
    let held = reading.bytes(reading.history()).await.unwrap();
    assert_eq!(held[2], Bytes::from(frame(Tag::Delta, 0, 64)), "the bytes the store holds");
    frames.push(frame(Tag::Delta, 10_000, 64)).await.unwrap();
    frames.push(frame(Tag::End, 15_000, 64)).await.unwrap();
    assert_eq!(next(&mut reading).await, Ok(Some(Tag::Delta)));
    assert_eq!(next(&mut reading).await, Ok(Some(Tag::End)));
    provider.inner.frames.close(&key(1));
    assert_eq!(next(&mut reading).await, Ok(None), "the attempt ended");
}

/// An open attempt's scenario block and first keyframe held, a reader opened, and then `deltas` slices and the end
/// block taken and the attempt ended without the reader taking a frame.
async fn left_behind(dir: &TempDir, deltas: u64) -> Reading {
    let (provider, _store, _known) = cold(dir, 0).await;
    let frames = provider.inner.frames.open(&key(1));
    frames.push(frame(Tag::Scenario, 0, 0)).await.unwrap();
    frames.push(frame(Tag::Keyframe, 0, 64)).await.unwrap();
    let reading = provider.open(&key(1), Ok(None)).await.expect("in memory");
    for slice in 0..deltas {
        frames.push(frame(Tag::Delta, slice * 10_000, 64)).await.unwrap();
    }
    frames.push(frame(Tag::End, deltas * 10_000, 64)).await.unwrap();
    provider.inner.frames.close(&key(1));
    reading
}

/// A burst, as a runner sends everything it wrote while its connection was lost, leaves a reader that takes none of it
/// as it comes three thousand slices behind. The reader still takes every frame of it in order, then the ending, never
/// a fault.
#[tokio::test]
async fn a_reader_left_behind_by_a_burst_takes_every_frame_of_it_in_order() {
    let dir = TempDir::new().unwrap();
    let mut reading = left_behind(&dir, 3_000).await;
    let mut taken = Vec::new();
    while let Some(frame) = timeout(Duration::from_secs(5), reading.next()).await.expect("no wait").expect("no fault") {
        taken.push((frame.tag, frame.at.as_ms()));
    }
    let mut burst: Vec<(Tag, u64)> = (0..3_000).map(|slice| (Tag::Delta, slice * 10_000)).collect();
    burst.push((Tag::End, 30_000_000));
    assert_eq!(taken, burst);
}

/// What a burst queued is handed over without a wait, at most as many frames as asked, and the queue's ending with
/// the last of them, so the reading then says the copy is whole.
#[tokio::test]
async fn ready_hands_over_what_a_burst_queued_and_its_ending() {
    let dir = TempDir::new().unwrap();
    let mut reading = left_behind(&dir, 40).await;
    assert_eq!(reading.ready(32).len(), 32);
    let rest: Vec<Tag> = reading.ready(32).iter().map(|frame| frame.tag).collect();
    assert_eq!(rest.len(), 9, "eight slices and the end block: {rest:?}");
    assert_eq!(rest.last(), Some(&Tag::End));
    assert_eq!(next(&mut reading).await, Ok(None), "the ending was taken with them");
}

/// Live segments whose uploads never finished are an incomplete copy, fetched from the store and then read off the
/// disk, and read to their end they say so whatever their last frame says; and a disk copy that cannot be vouched for
/// is not read at all while the database does not answer, since whether a whole copy is to be had is then unknown.
#[tokio::test]
async fn live_segments_whose_uploads_never_finished_are_an_incomplete_copy() {
    let dir = TempDir::new().unwrap();
    let (provider, store, finished) = cold(&dir, 2).await;
    let stopped = Known { uploads_finished: false, ..finished };
    let mut fetched = provider.open(&key(1), Ok(Some(stopped.clone()))).await.expect("a fetch starts");
    assert!(fetched.incomplete(), "fetched");
    store.release(3);
    assert_eq!(rest(&mut fetched).await.unwrap(), whole_copy(2), "served as far as it goes");
    assert_eq!(ended_outcome(&fetched, Some(RunOutcome::SimTimeLimit)), RunOutcome::RecordingIncomplete);
    drop(fetched);
    let on_disk = provider.open(&key(1), Ok(Some(stopped))).await.expect("the copy on disk");
    assert!(on_disk.incomplete() && !on_disk.landing(), "on disk, not fetched again");
    let unanswered = provider.open(&key(1), Err("the database is down".to_owned())).await.err();
    assert_eq!(unanswered, Some(Unreached::Database("the database is down".to_owned())));
}

/// A copy fetched while the attempt's own server was still uploading holds the segments uploaded so far, and keeps its
/// word that it stops short however the tables change: read once that server's uploads have all landed, it gives way
/// to a fetch of the whole run, never passing for it; and that copy says beside it that it is whole, so it reads whole
/// with the database not answering, ended as its end block says, or as carrying none.
#[tokio::test]
async fn a_copy_fetched_while_the_uploads_went_on_gives_way_to_the_whole_run_once_they_finished() {
    let dir = TempDir::new().unwrap();
    let (provider, store, finished) = cold(&dir, 2).await;
    let uploading = Known { segments_uploaded: 1, uploads_finished: false, ..finished.clone() };
    let mut early = provider.open(&key(1), Ok(Some(uploading.clone()))).await.expect("a fetch starts");
    store.release(2);
    assert_eq!(rest(&mut early).await.unwrap(), whole_copy(1)[..5], "the scenario block and segment 0");
    drop(early);
    let again = provider.open(&key(1), Ok(Some(uploading))).await.expect("the copy on disk");
    assert!(again.incomplete() && !again.landing(), "still short, and nothing whole to be had");
    drop(again);
    let mut late = provider.open(&key(1), Ok(Some(finished))).await.expect("a fetch of the whole run");
    assert!(late.landing() && !late.incomplete(), "fetched in place of the short copy");
    store.release(3);
    assert_eq!(rest(&mut late).await.unwrap(), whole_copy(2));
    drop(late);
    let vouched = provider.open(&key(1), Err("the database is down".to_owned())).await.expect("its own word");
    assert!(!vouched.incomplete() && !vouched.landing());
    assert_eq!(ended_outcome(&vouched, Some(RunOutcome::SimTimeLimit)), RunOutcome::SimTimeLimit);
    assert_eq!(ended_outcome(&vouched, None), RunOutcome::NoEndBlock, "a whole copy without an end block");
    assert_eq!(store.reads.load(Ordering::SeqCst), 5, "two objects early, three late");
}

/// The attempt's own server took it to its end and stopped before its last upload landed, so it never vouched for its
/// complete disk copy. Restarted over the same cache, it serves that copy as one that may stop short while the tables
/// name nothing whole, and once they name the final recording, fetches it and serves it whole, as its end block says.
#[tokio::test]
async fn a_disk_copy_its_server_never_vouched_for_gives_way_to_the_final_recording() {
    let dir = TempDir::new().unwrap();
    let (provider, store, _) = cold(&dir, 0).await;
    let stopped = FrameStore::new(dir.path().join("cache"), u64::MAX, Uploader::new(store.clone(), None));
    let frames = stopped.open(&key(1));
    for (tag, at) in [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::End, 5_000)] {
        frames.push(frame(tag, at, 64)).await.unwrap();
    }
    let (run, attempt) = (RunId::new(key(1).run.as_str()).unwrap(), StoredAttempt::FIRST);
    let ended = Known { scenario_key: Some(live_scenario_key(&run, attempt)), ended: true, ..Known::default() };
    let own = provider.open(&key(1), Ok(Some(ended.clone()))).await.expect("the copy on disk");
    assert!(own.incomplete() && !own.landing(), "never vouched for, and nothing whole to be had");
    drop(own);
    store.inner.put(&recording_key(&run, attempt), final_recording().into()).await.unwrap();
    let stored = Known { recording_key: Some(recording_key(&run, attempt)), ..ended };
    let mut fetched = provider.open(&key(1), Ok(Some(stored))).await.expect("a fetch of the final recording");
    assert!(fetched.landing() && !fetched.incomplete());
    store.release(1);
    assert_eq!(rest(&mut fetched).await.unwrap().last(), Some(&Tag::End));
    assert_eq!(ended_outcome(&fetched, Some(RunOutcome::SimTimeLimit)), RunOutcome::SimTimeLimit);
}

/// A finished recording: the scenario block, a keyframe, three deltas, the index block and the end block.
fn final_recording() -> Vec<u8> {
    let mut writer = Writer::new(Vec::new());
    let (scenario, _) = Frame::decode_at(&frame(Tag::Scenario, 0, 0), 0).unwrap();
    writer.append_scenario(Ms::ZERO, &scenario.contents(0).unwrap()).unwrap();
    writer.append_sections(Tag::Keyframe, Ms::ZERO, &[(1, b"all".as_slice())]).unwrap();
    for at in [0, 10_000, 20_000] {
        writer.append_sections(Tag::Delta, Ms(at), &[(1, b"moved".as_slice())]).unwrap();
    }
    let end = End { stop_reason: "SimTimeLimit".to_owned(), frames_written: 5, ..End::default() };
    writer.finish(Ms(20_000), &end.to_bytes().unwrap()).unwrap()
}

/// An attempt this server took to its end, its uploads landed and its memory given back, is whole on its own disk,
/// even to a reader whose tables were read a moment before the uploads finished.
#[tokio::test]
async fn an_attempt_this_server_took_to_its_end_reads_whole_from_its_own_disk() {
    let dir = TempDir::new().unwrap();
    let (provider, _store, finished) = cold(&dir, 0).await;
    let frames = provider.inner.frames.open(&key(1));
    for (tag, at) in [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::End, 10_000)] {
        frames.push(frame(tag, at, 64)).await.unwrap();
    }
    provider.inner.frames.close(&key(1));
    provider.inner.frames.uploader().drain(&key(1)).await;
    let read_before = Known { uploads_finished: false, ..finished };
    let reading = provider.open(&key(1), Ok(Some(read_before))).await.expect("its own disk copy");
    assert!(!reading.live() && !reading.incomplete());
}
