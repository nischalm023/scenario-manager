//! Within `studio-metrics`, `states` is THE SERIES OF TIME IN A STATE, READ FROM THE ROBOT STATE CHANNEL: desvis's D30
//! state, D30 waiting, DA50 state and DA50 waiting tabs, each a share of time in trailing windows and over the whole
//! run. The span engine (`dessim::metrics::reporting`'s spans) reads the channel once for all four, holding each robot's
//! state, and makes each window one slice wide final once the follower has taken the slice past its end; those windows
//! are kept from the run's start, so a series is cut into any trailing windows at any time, and nothing sent is ever
//! revised. A category the channel never states, as the DA50 reasons a cycle's commit explains after the fact, has no
//! value.

use dessim::metrics::reporting::{
    D30State, D30StateDefinition, D30WaitCategory, D30WaitDefinition, Da50State, Da50StateDefinition,
    Da50WaitDefinition, Da50WaitReason, Ratio, SpanDefinition, SpanError, SpanEvent, SpanReader, SpanWindow,
    SpanWindows,
};
use desunits::{RobotId, SimDuration, SimTime};
use studio_protocol::{
    Aggregation, Bucket, Dimension, MetricDescriptor, MetricPoint, MetricSource, MetricUnit, TrailingSpec,
};
use worldview::frames::ROBOT_STATE;
use worldview::frames::playback::RobotClass;
use worldview::frames::robot_state::{RobotStateDelta, RobotStateKeyframe};
use worldview::{Payload, Scenario, Sections};

use crate::follower::{Taken, Unreadable, Unrecorded, Unservable};
use crate::reports::default_trailing;

/// One series of time in a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateSeries {
    D30State,
    D30Wait,
    Da50State,
    Da50Wait,
}

/// What a series' shares are of: the fleet's time, or the time it spent waiting.
#[derive(Clone, Copy)]
enum Whole {
    FleetTime,
    WaitingTime,
}

impl StateSeries {
    pub const ALL: [StateSeries; 4] = [Self::Da50State, Self::Da50Wait, Self::D30State, Self::D30Wait];

    pub fn name(self) -> &'static str {
        match self {
            Self::D30State => "d30_state_share",
            Self::D30Wait => "d30_wait_share",
            Self::Da50State => "da50_state_share",
            Self::Da50Wait => "da50_wait_share",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::D30State => "D30 state, % of fleet time",
            Self::D30Wait => "D30 waiting, % by category",
            Self::Da50State => "DA50 state, % of fleet time",
            Self::Da50Wait => "DA50 waiting, % by reason",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::D30State => {
                "Time the D30s spent in each state, as a percentage of the D30 fleet's time in the window; read from the robot state channel, each window stated once the server has read the slice past its end."
            }
            Self::D30Wait => {
                "Time the D30s spent waiting, by category, as a percentage of their waiting time in the window; read from the robot state channel."
            }
            Self::Da50State => {
                "Time the DA50s spent in each state, as a percentage of the DA50 fleet's time in the window; read from the robot state channel, each window stated once the server has read the slice past its end."
            }
            Self::Da50Wait => {
                "Time the DA50s spent waiting, by reason, as a percentage of their waiting time in the window; read from the robot state channel, which states a DA50's wait before its cycle as no selected task, so the reasons the cycle's commit gives have no value."
            }
        }
    }

    fn dimension(self) -> Dimension {
        let (name, description) = match self {
            Self::D30State => ("state", "The D30 state, as desvis names it."),
            Self::D30Wait => ("category", "Why the D30 waited."),
            Self::Da50State => ("state", "The DA50 state, as desvis names it."),
            Self::Da50Wait => ("reason", "Why the DA50 waited."),
        };
        Dimension { name: name.to_owned(), description: description.to_owned() }
    }

    fn whole(self) -> Whole {
        match self {
            Self::D30State | Self::Da50State => Whole::FleetTime,
            Self::D30Wait | Self::Da50Wait => Whole::WaitingTime,
        }
    }
}

/// The series' descriptor, as served in `bucket`.
pub fn descriptor(series: StateSeries, bucket: Bucket) -> MetricDescriptor {
    let denominator = match series.whole() {
        Whole::FleetTime => "percent of the fleet's time in the window",
        Whole::WaitingTime => "percent of the waiting time in the window",
    };
    MetricDescriptor {
        metric: series.name().to_owned(),
        label: series.label().to_owned(),
        description: series.description().to_owned(),
        unit: MetricUnit::Ratio,
        clock: None,
        aggregation: Aggregation::Ratio,
        denominator: Some(denominator.to_owned()),
        source: MetricSource::RobotState,
        bucket,
        dimensions: vec![series.dimension()],
    }
}

/// One window one slice wide, as kept: the fleet's time in it and each category's, in milliseconds, in the
/// definition's order.
struct Window {
    fleet_ms: u64,
    times_ms: Box<[u64]>,
}

/// One definition's windows: the ones the engine still holds open, and every one gone out since the reader began.
struct Windows<D: SpanDefinition> {
    open: SpanWindows<D>,
    out: Vec<Window>,
}

impl<D: SpanDefinition> Windows<D> {
    fn new(width: SimDuration, from: SimTime) -> Result<Self, SpanError> {
        Ok(Self { open: SpanWindows::new(width, from)?, out: Vec::new() })
    }

    fn take(&mut self, events: &[SpanEvent]) -> Result<(), SpanError> {
        let kept = |window: SpanWindow<D::Category>| Window {
            fleet_ms: window.fleet_time.as_ms(),
            times_ms: window.times.iter().map(|(_, time)| time.as_ms()).collect(),
        };
        self.out.extend(self.open.take(events)?.into_iter().map(kept));
        Ok(())
    }
}

/// The channel from its first keyframe on: the engine's reader, each definition's windows, and the run's end.
struct Seeded {
    reader: SpanReader,
    /// The grid index of the first window, the one holding the keyframe the reader was seeded at.
    first: u64,
    d30_states: Windows<D30StateDefinition>,
    d30_waits: Windows<D30WaitDefinition>,
    da50_states: Windows<Da50StateDefinition>,
    da50_waits: Windows<Da50WaitDefinition>,
    run_end: Option<SimTime>,
}

/// The robot state channel of a recording that records it, read once for every series of time in a state.
pub(crate) struct Channel {
    roster: Vec<(RobotId, RobotClass)>,
    width: SimDuration,
    /// Seeded at the recording's first keyframe.
    seeded: Option<Seeded>,
}

impl Channel {
    /// The channel a scenario block declares, or the fact that it does not.
    pub(crate) fn declared(scenario: &Scenario) -> Result<Self, Unservable> {
        if !scenario.sections.iter().any(|(id, _)| *id == ROBOT_STATE) {
            return Err(Unservable::NotRecorded(Unrecorded::RobotState));
        }
        let roster = scenario.robots.iter().map(|robot| (robot.robot, robot.class)).collect();
        Ok(Self { roster, width: scenario.slice_width, seeded: None })
    }

    /// A keyframe at `at`, its `sections`: the first seeds the reader with every robot's state; a later one says
    /// nothing new.
    pub(crate) fn keyframe(&mut self, at: SimTime, sections: &Sections) -> Result<(), Unreadable> {
        if self.seeded.is_some() {
            return Ok(());
        }
        let stated = sections
            .get(ROBOT_STATE)
            .ok_or_else(|| Unreadable::because(format!("the keyframe at {at} states no robot")))?;
        let states = RobotStateKeyframe::from_bytes(stated, 0).map_err(Unreadable::because)?;
        let refused = |fault: SpanError| Unreadable::because(fault);
        self.seeded = Some(Seeded {
            reader: SpanReader::seeded(self.roster.iter().copied(), self.width, at, &states).map_err(refused)?,
            first: at.as_ms() / self.width.as_ms(),
            d30_states: Windows::new(self.width, at).map_err(refused)?,
            d30_waits: Windows::new(self.width, at).map_err(refused)?,
            da50_states: Windows::new(self.width, at).map_err(refused)?,
            da50_waits: Windows::new(self.width, at).map_err(refused)?,
            run_end: None,
        });
        Ok(())
    }

    /// The slice beginning at `at`, its delta's `sections`: the slice before it told, and every window ending by `at`
    /// made final.
    pub(crate) fn slice(&mut self, at: SimTime, sections: &Sections) -> Result<(), Unreadable> {
        let seeded =
            self.seeded.as_mut().ok_or_else(|| Unreadable::because(format!("a slice at {at} before a keyframe")))?;
        let stated = sections
            .get(ROBOT_STATE)
            .ok_or_else(|| Unreadable::because(format!("the slice at {at} states no robot")))?;
        let spans = RobotStateDelta::from_bytes(stated, 0).map_err(Unreadable::because)?;
        let events = seeded.reader.slice(at, spans).map_err(Unreadable::because)?;
        seeded.take(&events).map_err(Unreadable::because)
    }

    /// The run's stop at `run_end`, where the end block is stamped: the last slice told up to there, and every window
    /// made final. A run that stopped before its first slice stated nothing to end.
    pub(crate) fn end(&mut self, run_end: SimTime) -> Result<(), Unreadable> {
        let Some(seeded) = self.seeded.as_mut() else { return Ok(()) };
        let events = seeded.reader.end(run_end).map_err(Unreadable::because)?;
        seeded.take(&events).map_err(Unreadable::because)?;
        seeded.run_end = Some(run_end);
        Ok(())
    }
}

impl Seeded {
    fn take(&mut self, events: &[SpanEvent]) -> Result<(), SpanError> {
        self.d30_states.take(events)?;
        self.d30_waits.take(events)?;
        self.da50_states.take(events)?;
        self.da50_waits.take(events)
    }
}

/// One series of time in a state as one ask cuts it: trailing windows, then the whole run.
pub(crate) struct StateView {
    series: StateSeries,
    spec: TrailingSpec,
    /// The end of the next trailing window to state, in milliseconds of simtime.
    next_end: u64,
}

impl StateView {
    /// `series` in the trailing windows `asked`, or desvis's report window, which must be whole slices.
    pub(crate) fn opened(series: StateSeries, asked: Option<TrailingSpec>, taken: &Taken) -> Result<Self, Unservable> {
        let channel = taken.channel.as_ref().map_err(Clone::clone)?;
        let spec = asked.unwrap_or_else(default_trailing);
        if !spec.fits(channel.width) {
            return Err(Unservable::UnsupportedTrailing(spec));
        }
        let (window, step) = (spec.window_sim_ms.as_ms(), spec.step_sim_ms.as_ms());
        Ok(Self { series, spec, next_end: window.div_ceil(step) * step })
    }

    pub(crate) fn descriptor(&self) -> MetricDescriptor {
        descriptor(self.series, Bucket::Trailing(self.spec))
    }

    /// Every trailing window whose slices have all gone out since the view last advanced, then, once the end block
    /// has been taken, the whole run.
    pub(crate) fn advance(&mut self, taken: &Taken) -> Result<Vec<MetricPoint>, Unservable> {
        let channel = taken.channel.as_ref().map_err(Clone::clone)?;
        let Some(seeded) = &channel.seeded else { return Ok(Vec::new()) };
        let width = channel.width.as_ms();
        Ok(match self.series {
            StateSeries::D30State => self.windows(seeded, &seeded.d30_states, width, D30State::label),
            StateSeries::D30Wait => self.windows(seeded, &seeded.d30_waits, width, D30WaitCategory::label),
            StateSeries::Da50State => self.windows(seeded, &seeded.da50_states, width, Da50State::label),
            StateSeries::Da50Wait => self.windows(seeded, &seeded.da50_waits, width, Da50WaitReason::label),
        })
    }

    fn windows<D: SpanDefinition>(
        &mut self,
        seeded: &Seeded,
        kept: &Windows<D>,
        width: u64,
        label: fn(D::Category) -> &'static str,
    ) -> Vec<MetricPoint> {
        let out = &kept.out;
        let first_ms = seeded.first * width;
        let gone_out_ms = first_ms + out.len() as u64 * width;
        let reached = seeded.run_end.map_or(gone_out_ms, |run_end| run_end.as_ms().min(gone_out_ms));
        let whole = self.series.whole();
        let mut rows = Vec::new();
        while self.next_end <= reached {
            let from = self.next_end - self.spec.window_sim_ms.as_ms();
            // A window reaching back before the keyframe the reader began at has time no reader saw: it is left out.
            if from >= first_ms {
                let held = &out[((from - first_ms) / width) as usize..((self.next_end - first_ms) / width) as usize];
                rows.extend(shares::<D>(held, whole, label, from, self.next_end, false));
            }
            self.next_end += self.spec.step_sim_ms.as_ms();
        }
        if let Some(run_end) = seeded.run_end {
            rows.extend(shares::<D>(out, whole, label, first_ms, run_end.as_ms(), true));
        }
        rows
    }
}

/// One row per category of `D` over the windows `held`, from `from` to `to`: each category's time as a percentage of
/// `whole`, but none for a category the definition calls unsupported.
fn shares<D: SpanDefinition>(
    held: &[Window],
    whole: Whole,
    label: fn(D::Category) -> &'static str,
    from: u64,
    to: u64,
    whole_run: bool,
) -> Vec<MetricPoint> {
    let mut times = vec![0u64; D::CATEGORIES.len()];
    let mut fleet = 0u64;
    for window in held {
        fleet += window.fleet_ms;
        times.iter_mut().zip(&window.times_ms).for_each(|(time, add)| *time += add);
    }
    let of = match whole {
        Whole::FleetTime => fleet,
        Whole::WaitingTime => times.iter().sum(),
    };
    D::CATEGORIES
        .iter()
        .zip(times)
        .map(|(category, time)| {
            let ratio = (!D::UNSUPPORTED.contains(category)).then_some(Ratio::new(time, of));
            MetricPoint {
                from_sim_ms: SimTime::ms(from),
                to_sim_ms: SimTime::ms(to),
                keys: vec![label(*category).to_owned()],
                value: ratio.and_then(Ratio::percent),
                numerator: ratio.map(|ratio| ratio.numerator as f64),
                denominator: ratio.map(|ratio| ratio.denominator as f64),
                whole_run,
            }
        })
        .collect()
}
