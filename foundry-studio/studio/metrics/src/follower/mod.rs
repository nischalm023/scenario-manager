//! Within `studio-metrics`, `follower` is ONE ATTEMPT'S RECORDING READ ONCE FOR EVERY METRIC: the server hands it the
//! scenario block, then every later frame in order, and it keeps what each family of metrics is computed from
//! (`Taken`): every slice's value of every time series, the robot state channel's windows, the journals, and what the
//! end block states. A socket's ask opens a view (`view`) that states every row final so far, then the rows each
//! frame makes final; one view serves every socket that asks alike. What a family alone reads, failing, fails that
//! family's views alone: the robot state channel, one journal's rows, the end block's summary, or one view's own
//! window; a frame or a slice's scalars that do not read fail them all. A frame a runner sends again after a lost
//! connection, at or before the newest slice taken, is passed over.

#[cfg(test)]
mod tests;
mod view;

use dessim::metrics::profile::SummaryStats;
use dessim::metrics::worldview::summary::read_document;
use desunits::SimTime;
use rustc_hash::FxHashMap;
use studio_protocol::{MetricDescriptor, MetricPoint, RunOutcome};
use worldview::frames::scalars::ScalarsDelta;
use worldview::frames::tables::TablesDelta;
use worldview::frames::{SCALARS, TABLES, instant};
use worldview::{Declare, Descriptors, End, Frame, Payload, Scenario, Sections, Tag};

use crate::ending::run_outcome;
use crate::reports::Journals;
use crate::series::{Slices, World};
use crate::states::Channel;

pub(crate) use view::View;
pub use view::{Ask, Unreadable, Unrecorded, Unservable};

/// What the follower has taken from the recording so far, kept from the run's start for every view.
pub(crate) struct Taken {
    pub(crate) slices: Slices,
    pub(crate) channel: Result<Channel, Unservable>,
    pub(crate) journals: Journals,
    /// How far the rows stated so far reach: the instant of the newest slice or end block taken, every slice before it
    /// stated; or, once a recording cut short is over, its last slice's end.
    pub(crate) reached: SimTime,
    pub(crate) ended: Option<Ended>,
}

/// What the end block states.
pub(crate) struct Ended {
    /// The end block's instant: the simtime the run reached.
    pub(crate) run_end: SimTime,
    pub(crate) outcome: RunOutcome,
    pub(crate) summary: Result<SummaryStats, Unservable>,
}

/// One attempt's recording, followed frame by frame for every view open on it.
pub struct Follower {
    descriptors: Descriptors,
    taken: Taken,
    /// The start of the newest slice taken.
    taken_through: Option<SimTime>,
    views: FxHashMap<Ask, View>,
}

/// A view as it opens: the metric as it will be served, and every row final so far.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    pub descriptor: MetricDescriptor,
    pub rows: Vec<MetricPoint>,
}

/// What a frame made known to one open view: the rows it made final, or why the view can go no further, after which
/// the view is closed.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    pub ask: Ask,
    pub rows: Result<Vec<MetricPoint>, Unservable>,
}

impl Follower {
    /// The follower of the recording whose first frame, its scenario block, is `scenario`. The block's station table,
    /// a row for every station the run's world compiled to, gives the station count presentations are divided by.
    pub fn from_scenario(scenario: &[u8]) -> Result<Self, Unreadable> {
        let (frame, _) = Frame::decode_at(scenario, 0).map_err(Unreadable::because)?;
        if frame.header.tag != Tag::Scenario {
            let opens = format!("the recording opens with a {} frame, not its scenario block", frame.header.tag);
            return Err(Unreadable::because(opens));
        }
        let block =
            Scenario::from_bytes(&frame.contents(0).map_err(Unreadable::because)?, 0).map_err(Unreadable::because)?;
        let descriptors = Descriptors::from_scenario(&block);
        let mut journals = Journals::new();
        journals.describe(&descriptors);
        let taken = Taken {
            slices: Slices::new(block.slice_width, World { stations: block.stations.len() }, &descriptors),
            channel: Channel::declared(&block),
            journals,
            reached: SimTime::ZERO,
            ended: None,
        };
        Ok(Self { descriptors, taken, taken_through: None, views: FxHashMap::default() })
    }

    /// One more frame, in the recording's order: what it made known to each open view.
    pub fn frame(&mut self, bytes: &[u8]) -> Result<Vec<Made>, Unreadable> {
        let (frame, _) = Frame::decode_at(bytes, 0).map_err(Unreadable::because)?;
        let at = instant(frame.header.t);
        let resent = self.taken_through.is_some_and(|through| at <= through);
        match frame.header.tag {
            Tag::Scenario => Err(Unreadable::because("a second scenario block")),
            // After the end block comes nothing but its segment sent again.
            _ if self.taken.ended.is_some() => Ok(Vec::new()),
            Tag::Index => Ok(Vec::new()),
            Tag::Declare | Tag::Delta if resent => Ok(Vec::new()),
            Tag::Declare => {
                let declared = Declare::from_bytes(&frame.contents(0).map_err(Unreadable::because)?, 0)
                    .map_err(Unreadable::because)?;
                self.descriptors.push(&declared);
                self.taken.slices.describe(&self.descriptors);
                self.taken.journals.describe(&self.descriptors);
                Ok(Vec::new())
            }
            Tag::Keyframe => {
                let sections = Sections::decode(frame.contents(0).map_err(Unreadable::because)?, 0)
                    .map_err(Unreadable::because)?;
                failing(&mut self.taken.channel, |channel| channel.keyframe(at, &sections));
                Ok(Vec::new())
            }
            Tag::Delta => {
                self.taken_through = Some(at);
                let sections = Sections::decode(frame.contents(0).map_err(Unreadable::because)?, 0)
                    .map_err(Unreadable::because)?;
                let scalars = match sections.get(SCALARS) {
                    Some(bytes) => ScalarsDelta::from_bytes(bytes, 0).map_err(Unreadable::because)?,
                    None => ScalarsDelta::default(),
                };
                self.taken.slices.take(at, scalars);
                self.taken.reached = at;
                if let Some(bytes) = sections.get(TABLES) {
                    match TablesDelta::from_bytes(bytes, 0) {
                        Ok(tables) => self.taken.journals.take(&tables.0),
                        Err(fault) => self.taken.journals.fail(&Unreadable::because(fault)),
                    }
                }
                failing(&mut self.taken.channel, |channel| channel.slice(at, &sections));
                Ok(self.advance())
            }
            Tag::End => {
                let end = End::from_bytes(&frame.contents(0).map_err(Unreadable::because)?, 0)
                    .map_err(Unreadable::because)?;
                let outcome = run_outcome(&end.stop_reason).ok_or_else(|| {
                    Unreadable::because(format!(
                        "the end block's stop reason `{}` is none this server knows",
                        end.stop_reason
                    ))
                })?;
                self.taken.slices.close(at);
                self.taken.reached = at;
                self.taken.journals.take(&end.tables);
                failing(&mut self.taken.channel, |channel| channel.end(at));
                let summary = read_document::<SummaryStats>(&end.summary, "summary statistics")
                    .map_err(|fault| Unservable::Unreadable(Unreadable::because(fault)));
                self.taken.ended = Some(Ended { run_end: at, outcome, summary });
                Ok(self.advance())
            }
        }
    }

    /// Open the view of `ask`, shared with every socket that asks alike: the metric as served, and every row final so
    /// far; or why it cannot be served.
    pub fn open(&mut self, ask: Ask) -> Result<Opened, Unservable> {
        let mut view = View::opened(ask, &self.taken)?;
        let rows = view.advance(&self.taken)?;
        let descriptor = view.descriptor();
        self.views.entry(ask).or_insert(view);
        Ok(Opened { descriptor, rows })
    }

    /// The last socket asking `ask` has gone.
    pub fn close(&mut self, ask: &Ask) {
        self.views.remove(ask);
    }

    /// How far the rows stated so far reach.
    pub fn reached(&self) -> SimTime {
        self.taken.reached
    }

    /// The recording is over without an end block, as one cut short is: its last slice is stated its whole width, as
    /// simrunner's stored series state it, and what that made known to each open view is told. A recording that took
    /// its end block is passed over.
    pub fn cut_short(&mut self) -> Vec<Made> {
        if self.taken.ended.is_some() {
            return Vec::new();
        }
        match self.taken.slices.close_whole() {
            Some(end) => {
                self.taken.reached = end;
                self.advance()
            }
            None => Vec::new(),
        }
    }

    /// How the run ended, once the end block has been taken.
    pub fn outcome(&self) -> Option<RunOutcome> {
        self.taken.ended.as_ref().map(|ended| ended.outcome)
    }

    /// Every open view advanced over what has been taken; a view that can go no further is closed.
    fn advance(&mut self) -> Vec<Made> {
        let mut made = Vec::new();
        self.views.retain(|ask, view| match view.advance(&self.taken) {
            Ok(rows) => {
                if !rows.is_empty() {
                    made.push(Made { ask: *ask, rows: Ok(rows) });
                }
                true
            }
            Err(fault) => {
                made.push(Made { ask: *ask, rows: Err(fault) });
                false
            }
        });
        made
    }
}

/// `take` applied to a family of what is taken that still reads; one that fails no longer reads, and its views fail.
fn failing<T>(family: &mut Result<T, Unservable>, take: impl FnOnce(&mut T) -> Result<(), Unreadable>) {
    let failed = match family {
        Ok(held) => take(held).err(),
        Err(_) => None,
    };
    if let Some(fault) = failed {
        *family = Err(fault.into());
    }
}
