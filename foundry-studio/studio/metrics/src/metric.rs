//! Within `studio-metrics`, `metric` is ONE OF THE METRICS THE SERVER COMPUTES, of whichever family, and where the
//! families meet: `catalogue` lists every metric as a page is offered it, and `find` names one by the name a socket
//! asks for.

use studio_protocol::{Bucket, MetricDescriptor};

use crate::reports::{ReportSeries, default_trailing, descriptor as report_descriptor};
use crate::series::{TimeSeries, find_series, series_catalogue};
use crate::states::{StateSeries, descriptor as state_descriptor};
use crate::summary::{SummarySeries, descriptor as summary_descriptor};

/// One of the metrics the server computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Metric {
    Series(TimeSeries),
    State(StateSeries),
    Report(ReportSeries),
    Summary(SummarySeries),
}

impl Metric {
    pub fn name(self) -> &'static str {
        match self {
            Self::Series(series) => series.name(),
            Self::State(series) => series.name(),
            Self::Report(series) => series.name(),
            Self::Summary(series) => series.name(),
        }
    }
}

/// Every metric the server computes, each in the bucketing it is served in unless an attach asks for another, at a
/// 10 s slice: the time series in desvis's order, then the report tabs' series, those of time in a state first, then
/// the end block's.
pub fn catalogue() -> Vec<MetricDescriptor> {
    let tab = Bucket::Trailing(default_trailing());
    let mut all = series_catalogue();
    all.extend(StateSeries::ALL.iter().map(|series| state_descriptor(*series, tab)));
    all.extend(ReportSeries::ALL.iter().map(|series| report_descriptor(*series, tab)));
    all.extend(SummarySeries::ALL.iter().map(|series| summary_descriptor(*series)));
    all
}

/// The metric `name` names, if any.
pub fn find(name: &str) -> Option<Metric> {
    find_series(name)
        .map(Metric::Series)
        .or_else(|| StateSeries::ALL.iter().copied().find(|series| series.name() == name).map(Metric::State))
        .or_else(|| ReportSeries::ALL.iter().copied().find(|series| series.name() == name).map(Metric::Report))
        .or_else(|| SummarySeries::ALL.iter().copied().find(|series| series.name() == name).map(Metric::Summary))
}

#[cfg(test)]
mod tests {
    use studio_protocol::{DurationClock, MetricSource, MetricUnit};

    use super::*;

    #[test]
    fn the_catalogue_holds_every_metric_of_every_family_and_finds_each_by_name() {
        let all = catalogue();
        assert_eq!(all.len(), 74 + 4 + 7 + 22);
        for described in &all {
            assert_eq!(find(&described.metric).map(Metric::name), Some(described.metric.as_str()));
        }
        assert!(matches!(find("da50_state_share"), Some(Metric::State(StateSeries::Da50State))));
        assert!(matches!(find("da50_wait_share"), Some(Metric::State(StateSeries::Da50Wait))));
        assert!(matches!(find("picks"), Some(Metric::Series(TimeSeries::PicksPerSlice))));
        assert!(matches!(find("run_total_picks"), Some(Metric::Summary(SummarySeries::TotalPicks))));
        assert_eq!(find("nothing"), None);
        let sources = |source| all.iter().filter(|described| described.source == source).count();
        assert_eq!(
            [MetricSource::Scalars, MetricSource::RobotState, MetricSource::Journals, MetricSource::EndBlock]
                .map(sources),
            [74, 4, 7, 22],
            "each family names the part of the recording it is computed from"
        );
    }

    /// A page labels a duration, or a rate's hour, with the clock its descriptor names: a metric measuring time that
    /// named none would go unlabelled, and the two wall-clock figures would read as simtime.
    #[test]
    fn every_duration_and_rate_names_its_clock_and_only_the_wall_clock_figures_name_the_wall_clock() {
        let all = catalogue();
        for described in &all {
            let measures_time = matches!(described.unit, MetricUnit::DurationMs | MetricUnit::RatePerHour);
            assert_eq!(described.clock.is_some(), measures_time, "{}", described.metric);
        }
        let on_the_wall_clock: Vec<&str> = all
            .iter()
            .filter(|described| described.clock == Some(DurationClock::Wall))
            .map(|described| described.metric.as_str())
            .collect();
        assert_eq!(on_the_wall_clock, ["wall_clock_per_slice", "run_wall_clock"]);
    }

    /// What a page shows of a metric and the name it attaches by say what they measure in the project's terms: none
    /// of the words the project rejects as vague, "leg", "dwell", "settle", "returning", "advertisement", in a name,
    /// label, description or dimension.
    #[test]
    fn no_metric_is_named_or_described_in_a_word_the_project_rejects() {
        let rejected = ["leg", "legs", "dwell", "dwells", "settle", "settles", "returning", "advertisement"];
        for described in catalogue() {
            let mut said = vec![described.metric.clone(), described.label.clone(), described.description.clone()];
            said.extend(
                described
                    .dimensions
                    .iter()
                    .flat_map(|dimension| [dimension.name.clone(), dimension.description.clone()]),
            );
            for text in said {
                let words = text.split(|c: char| !c.is_ascii_alphabetic()).map(str::to_ascii_lowercase);
                let found: Vec<String> = words.filter(|word| rejected.contains(&word.as_str())).collect();
                assert!(found.is_empty(), "{}: {found:?} in {text:?}", described.metric);
            }
        }
    }
}
