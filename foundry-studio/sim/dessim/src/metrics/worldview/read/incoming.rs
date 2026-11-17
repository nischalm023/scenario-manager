//! Within `read`, `incoming` is A RECORDING READ AS ITS BYTES COME IN: the
//! bytes a recording is written as, handed over in whatever pieces they come
//! in, taken into the run model a frame at a time, as a file's frames are.
//!
//! **What it is for.** What watches a run reads the recording being written,
//! not a second account of the run: the same bytes the file gets, decoded the
//! way a file is read, so a run watched while it is written and the same run
//! played back from its file are one reading of one recording.
//!
//! **What it holds.** The run model, and of the bytes only those of the frame
//! not yet whole: a frame is taken once its last byte is in and its bytes are
//! let go, so the intake does not hold a long run twice, as its recording's
//! bytes and as the run model they state. A reader that keeps a series of its
//! own beside the intake's, as desvis's window does, holds those slices twice.

use std::mem;

use desunits::SimDuration;
use worldview::container::index::OrderCheck;
use worldview::frames::scenario::Scenario;
use worldview::{Error, Frame};

use super::{FrameIntake, Recording};
use crate::metrics::profile::{DynamicGrid, RobotIdentity, SliceRecord};

/// A recording read from its bytes, in whatever pieces they come in: each
/// frame is decoded once it is whole and taken into the run model it states.
///
/// The first damage ends the reading, and every call after it answers with
/// that damage: a frame read past it would be read from a place in the
/// recording that is no longer known.
#[derive(Default)]
pub struct IncomingRecording {
    /// The run model the frames taken rebuild, from the scenario block on.
    intake: Option<FrameIntake>,
    /// The bytes of the next frame, which is not whole yet.
    held: Vec<u8>,
    /// Where the next frame begins in the recording.
    offset: u64,
    order: OrderCheck,
    damage: Option<String>,
}

impl IncomingRecording {
    /// Take the recording's next bytes, which carry on from the last ones
    /// taken and may end anywhere, inside a frame or on a frame's last byte.
    /// A frame that does not decode, or that breaks the order a recording's
    /// frames run in, is damage. What follows the end frame is the
    /// container's trailer, not a frame, and is not read.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        if let Some(damage) = &self.damage {
            return Err(damage.clone());
        }
        let read = match self.held.is_empty() {
            // Bytes that begin a frame are read where they lie, and only the
            // ones after the last whole frame among them are copied to hold.
            true => self.take_whole_frames(bytes).map(|taken| self.held.extend_from_slice(&bytes[taken..])),
            false => {
                let mut held = mem::take(&mut self.held);
                held.extend_from_slice(bytes);
                self.take_whole_frames(&held).map(|taken| {
                    held.drain(..taken);
                    self.held = held;
                })
            }
        };
        if self.has_ended() {
            self.held = Vec::new();
        }
        read.inspect_err(|damage| self.damage = Some(damage.clone()))
    }

    /// Take every whole frame at the front of `bytes`, which begin where the
    /// next frame does, up to the end frame, and say how many bytes they span.
    fn take_whole_frames(&mut self, bytes: &[u8]) -> Result<usize, String> {
        let mut taken = 0;
        while !self.has_ended() {
            let (frame, len) = match Frame::decode_at(&bytes[taken..], self.offset) {
                Ok(whole) => whole,
                // The rest of the frame is not in yet.
                Err(Error::ShortHeader { .. } | Error::ShortPayload { .. }) => break,
                Err(damage) => return Err(damage.to_string()),
            };
            self.order.admit(frame.header.tag, frame.header.t, self.offset).map_err(|e| e.to_string())?;
            match self.intake.as_mut() {
                Some(intake) => intake.take(self.offset, &frame)?,
                None => {
                    let scenario = Scenario::from_frame(&frame).map_err(|e| e.to_string())?;
                    self.intake = Some(FrameIntake::open(scenario)?);
                }
            }
            taken += len;
            self.offset += len as u64;
        }
        Ok(taken)
    }

    /// The width of every slice, once the scenario block, which states it, is
    /// in.
    pub fn slice_width(&self) -> Option<SimDuration> {
        self.intake.as_ref().map(FrameIntake::slice_width)
    }

    /// The slices of the deltas taken, in order.
    pub fn slices(&self) -> &[SliceRecord] {
        match &self.intake {
            Some(intake) => intake.slices(),
            None => &[],
        }
    }

    /// The robots the run holds, once the scenario block, which states them,
    /// is in.
    pub fn robots(&self) -> Option<Vec<RobotIdentity>> {
        self.intake.as_ref().map(FrameIntake::robots)
    }

    /// How many orders the run's scenario resolved to, once the scenario
    /// block, which states them, is in.
    pub fn orders(&self) -> Option<usize> {
        self.intake.as_ref().map(FrameIntake::orders)
    }

    /// What each cell has been counted for, summed over the slices taken,
    /// once the scenario block is in: once every slice is in, it is the run
    /// model's `cumulative_grid`.
    pub fn cumulative_grid(&self) -> Option<&DynamicGrid> {
        self.intake.as_ref().map(FrameIntake::cumulative_grid)
    }

    /// Whether the end block is in: the run's totals, summary and result are
    /// then whole, and no frame follows.
    pub fn has_ended(&self) -> bool {
        self.intake.as_ref().is_some_and(FrameIntake::has_ended)
    }

    /// The run model the frames taken state, truncated when the end block is
    /// not among them; the bytes of a frame not yet whole are let go, as they
    /// are when a file cut inside a frame is read.
    pub fn into_recording(self) -> Result<Recording, String> {
        if let Some(damage) = self.damage {
            return Err(damage);
        }
        self.intake.ok_or("the recording's scenario block is not whole, so it states no run")?.into_recording()
    }
}
