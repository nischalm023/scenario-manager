//! Within `series`, `view` is ONE TIME SERIES AS ONE ASK CUTS IT: a row per slice, or trailing windows over the
//! slices, from the run's start to where the slices taken reach. Its formula's keys must be declared, and an asked
//! window must be whole slices of a series that is no running total.

use studio_protocol::{Bucket, MetricDescriptor, MetricPoint, TrailingSpec};

use super::slices::Slices;
use super::trailing::Trailing;
use super::{TimeSeries, default_bucket, descriptor};
use crate::follower::{Unrecorded, Unservable};

pub(crate) struct SeriesView {
    series: TimeSeries,
    bucket: Bucket,
    trailing: Option<Trailing>,
    /// The next slice to take.
    next: usize,
}

impl SeriesView {
    /// `series` cut as `asked`, or in its own bucketing.
    pub(crate) fn opened(series: TimeSeries, asked: Option<TrailingSpec>, slices: &Slices) -> Result<Self, Unservable> {
        if let Some(key) = series.keys().into_iter().find(|key| !slices.declares(key)) {
            return Err(Unservable::NotRecorded(Unrecorded::Scalar(key)));
        }
        let width = slices.width();
        let spec = match asked {
            Some(asked) if series.is_cumulative() || !asked.fits(width) => {
                return Err(Unservable::UnsupportedTrailing(asked));
            }
            Some(asked) => Some(asked),
            None => series.default_trailing(width),
        };
        let (bucket, trailing) = match spec {
            Some(spec) => (Bucket::Trailing(spec), Some(Trailing::new(spec, series.trailing_sums()))),
            None => (default_bucket(series, width), None),
        };
        Ok(Self { series, bucket, trailing, next: 0 })
    }

    pub(crate) fn descriptor(&self) -> MetricDescriptor {
        descriptor(self.series, self.bucket)
    }

    /// The rows of every slice stated since the view last advanced: one per slice, or each window a slice closes.
    pub(crate) fn advance(&mut self, slices: &Slices) -> Vec<MetricPoint> {
        let taken = &slices.taken()[self.next..];
        self.next += taken.len();
        let points = taken.iter().map(|slice| slice.point(self.series));
        match &mut self.trailing {
            Some(trailing) => points.filter_map(|point| trailing.push(&point)).collect(),
            None => points.collect(),
        }
    }
}
