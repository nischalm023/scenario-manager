//! Within `results`, `values` is spec 08 sections 3.9 and 3.10's arithmetic over one attempt's stored values, apart
//! from the database so the canned fixture's tests hold it: which instants of simtime the recording holds values for
//! and whether a window's ends are among them, the report window as far as an attempt got, an accumulating metric's
//! running value at an instant, an additive metric's value over a window as the difference of two running values, its
//! value per simulated period, and any other class's readings as stated.

use desunits::{SimDuration, SimTime};
use studio_protocol::{
    Aggregation, DurationClock, MetricBasis, MetricRow, MetricUnit, PeriodPoint, ReportWindow, SeriesKind, SeriesPoint,
    SeriesShape, SimWindow, ValueBasis,
};

/// One metric the attempt declared, as its sidecar row holds it: the final value and where that came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Declared {
    pub metric: String,
    pub label: String,
    pub description: String,
    pub unit: MetricUnit,
    /// The clock of a duration or a rate's hour; none for any other unit and for an attempt stored before results
    /// named their clock.
    pub clock: Option<DurationClock>,
    pub aggregation: Aggregation,
    pub denominator: Option<String>,
    pub value: Option<f64>,
    pub basis: MetricBasis,
    /// Whether the value is the whole run's by definition, as the end block's Status figures are.
    pub whole_run: bool,
}

/// One metric's stored series: running values at slice ends for a class that accumulates, a gauge's readings else.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub kind: SeriesKind,
    pub points: Vec<SeriesPoint>,
}

/// The instants of simtime an attempt holds values for, rising: the run's start, every slice end any series names,
/// and the run's end, which is how far the attempt got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instants {
    rising: Vec<SimTime>,
}

/// A window end that falls between recorded instants, with the nearest recorded instant either side of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unrecorded {
    pub asked: SimTime,
    pub before: Option<SimTime>,
    pub after: Option<SimTime>,
}

impl Instants {
    pub fn new<'a>(series: impl IntoIterator<Item = &'a Stored>, end: SimTime) -> Self {
        let mut rising: Vec<SimTime> = vec![SimTime::ZERO, end];
        for stored in series {
            rising.extend(stored.points.iter().map(|point| point.0).filter(|at| *at <= end));
        }
        rising.sort_unstable();
        rising.dedup();
        Self { rising }
    }

    pub fn end(&self) -> SimTime {
        self.rising.last().copied().unwrap_or(SimTime::ZERO)
    }

    /// `at` when the recording holds values there, else the nearest instants either side of it.
    pub fn exact(&self, at: SimTime) -> Result<SimTime, Unrecorded> {
        match self.rising.binary_search(&at) {
            Ok(_) => Ok(at),
            Err(place) => Err(Unrecorded {
                asked: at,
                before: place.checked_sub(1).and_then(|before| self.rising.get(before)).copied(),
                after: self.rising.get(place).copied(),
            }),
        }
    }

    /// The window from `from` to `to`, the run's end when `to` is none, once both ends are recorded instants.
    pub fn window(&self, from: SimTime, to: Option<SimTime>) -> Result<SimWindow, Unrecorded> {
        let from = self.exact(from)?;
        Ok(SimWindow { from_sim_ms: from, to_sim_ms: self.exact(to.unwrap_or_else(|| self.end()))? })
    }

    /// The whole run, from its start to its end.
    pub fn whole(&self) -> SimWindow {
        SimWindow { from_sim_ms: SimTime::ZERO, to_sim_ms: self.end() }
    }

    /// `report` as far as the attempt got: its end the run's end where the attempt stopped before it, its start no
    /// later than its end, so an attempt that stopped early still answers for the part of its window it reached.
    pub fn reached(&self, report: &ReportWindow) -> SimWindow {
        let end = self.end();
        let to = report.to_sim_ms.map_or(end, |to| to.min(end));
        SimWindow { from_sim_ms: report.from_sim_ms.min(to), to_sim_ms: to }
    }
}

/// An accumulating metric's running value at `at`, its newest point at or before it; none before its first.
pub fn running_at(points: &[SeriesPoint], at: SimTime) -> Option<f64> {
    let stated = points.partition_point(|point| point.0 <= at);
    points[..stated].iter().rev().find_map(|point| point.1)
}

/// An additive metric's value over `window`: its running value at the end less at the start, where nothing stated
/// yet counts nothing; none when the run stated nothing by the window's end, which is never shown as 0.
pub fn window_difference(points: &[SeriesPoint], window: SimWindow) -> Option<f64> {
    let end = running_at(points, window.to_sim_ms)?;
    Some(end - running_at(points, window.from_sim_ms).unwrap_or(0.0))
}

/// One declared metric over `window`: a whole-run figure of the end block, such as the run's total picks, its declared
/// value over any window, marked by the whole run's; an additive one as its window difference from its running values,
/// no value where the window holds no span (an attempt that stopped before its report window), never the whole run's
/// number in its place; an additive one that stored no running values its declared value over the whole run's window
/// and no value over a narrower one; every other class its whole-run value, marked by the whole run's window.
pub fn row(declared: &Declared, stored: Option<&Stored>, window: SimWindow, whole: SimWindow) -> MetricRow {
    let running = stored.filter(|stored| stored.kind == SeriesKind::Running);
    let (value, window, basis) = match (declared.aggregation, running) {
        _ if declared.whole_run => (declared.value, whole, whole_basis(declared.basis)),
        (Aggregation::Additive, Some(running)) => {
            let spanned = window.from_sim_ms < window.to_sim_ms;
            let value = window_difference(&running.points, window).filter(|_| spanned);
            (value, window, ValueBasis::WindowDifference)
        }
        (Aggregation::Additive, None) if window == whole => (declared.value, whole, whole_basis(declared.basis)),
        (Aggregation::Additive, None) => (None, window, ValueBasis::WindowDifference),
        _ => (declared.value, whole, whole_basis(declared.basis)),
    };
    MetricRow {
        metric: declared.metric.clone(),
        label: declared.label.clone(),
        description: declared.description.clone(),
        unit: declared.unit,
        clock: declared.clock,
        aggregation: declared.aggregation,
        denominator: declared.denominator.clone(),
        value,
        window,
        basis,
    }
}

fn whole_basis(basis: MetricBasis) -> ValueBasis {
    match basis {
        MetricBasis::EndBlock => ValueBasis::EndBlock,
        MetricBasis::Rebuilt => ValueBasis::Rebuilt,
    }
}

/// The period boundaries from the run's start to `end`, every `period` and then the end, which may cut the last
/// period short.
pub fn periods(period: SimDuration, end: SimTime) -> Vec<SimWindow> {
    let mut windows = Vec::new();
    let mut from = SimTime::ZERO;
    while from < end && period > SimDuration::ms(0) {
        let to = from.saturating_add(period).min(end);
        windows.push(SimWindow { from_sim_ms: from, to_sim_ms: to });
        from = to;
    }
    windows
}

/// A series as section 3.10 draws it: an additive metric per period, exact since every boundary is a recorded
/// instant; any other class as its readings, one point per slice that stated one, never thinned. An accumulating
/// class that is not additive is stored as running values, so its readings are each slice's increase.
pub fn shaped(declared: &Declared, stored: &Stored, periods: &[SimWindow]) -> (SeriesShape, Vec<PeriodPoint>) {
    if declared.aggregation == Aggregation::Additive && stored.kind == SeriesKind::Running {
        let points = periods
            .iter()
            .map(|window| PeriodPoint {
                from_sim_ms: window.from_sim_ms,
                to_sim_ms: window.to_sim_ms,
                value: window_difference(&stored.points, *window),
            })
            .collect();
        return (SeriesShape::PerPeriod, points);
    }
    let mut before: Option<f64> = None;
    let points = stored
        .points
        .iter()
        .map(|point| {
            let value = match stored.kind {
                SeriesKind::Readings => point.1,
                SeriesKind::Running => point.1.map(|now| now - before.unwrap_or(0.0)),
            };
            before = point.1.or(before);
            PeriodPoint { from_sim_ms: point.0, to_sim_ms: point.0, value }
        })
        .collect();
    (SeriesShape::Readings, points)
}
