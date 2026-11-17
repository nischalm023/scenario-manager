//! Within `follower`, `view` is ONE METRIC AS ONE ASK CUTS IT: what a socket asks of an attempt (`Ask`), the view the
//! follower keeps of it while a socket holds it, and why an ask cannot be served (`Unservable`), each reason one of
//! the wire's refusals. A view is a cursor over what the follower has taken: opened at any time, it states every row
//! final so far, then each later one as the frames that make it final are taken, the same rows whenever it was opened.

use std::fmt;

use studio_protocol::{MetricDescriptor, MetricPoint, TrailingSpec};
use thiserror::Error;

use super::Taken;
use crate::metric::Metric;
use crate::reports::ReportView;
use crate::series::SeriesView;
use crate::states::StateView;
use crate::summary::SummaryView;

/// What a socket asks of an attempt: one metric, in its own bucketing or as trailing windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ask {
    pub metric: Metric,
    pub trailing: Option<TrailingSpec>,
}

/// Why an ask cannot be served from an attempt's recording, each the reason of one of the wire's refusals.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Unservable {
    /// The metric is not cut into the trailing windows asked: it is a running total or stated once over the whole run,
    /// or the windows are not whole slices of the recording (`unsupported-trailing`).
    #[error("the metric is not cut into the trailing windows asked, {0:?}")]
    UnsupportedTrailing(TrailingSpec),
    /// The recording lacks a fact the metric is computed from (`not-recorded`).
    #[error("the recording does not record {0}")]
    NotRecorded(Unrecorded),
    /// The recording, or the part of it the metric is computed from, does not read (`recording-unreadable`).
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

/// A fact a metric is computed from that a recording may lack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unrecorded {
    /// A scalar the recording does not declare, by its key.
    Scalar(&'static str),
    /// The robot state channel, which a recording declares in its scenario block.
    RobotState,
    /// A journal table the recording does not declare, by its name.
    Table(&'static str),
}

impl fmt::Display for Unrecorded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scalar(key) => write!(f, "the scalar `{key}`"),
            Self::RobotState => f.write_str("the robot state channel"),
            Self::Table(name) => write!(f, "the table `{name}`"),
        }
    }
}

/// Why the recording, or the part of it a metric is computed from, does not read; in words, for the server's log.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct Unreadable(pub(crate) String);

impl Unreadable {
    pub(crate) fn because(why: impl fmt::Display) -> Self {
        Self(why.to_string())
    }
}

/// One ask's view, by the family of its metric.
pub(crate) enum View {
    Series(SeriesView),
    State(StateView),
    Report(ReportView),
    Summary(SummaryView),
}

impl View {
    /// The view of `ask` over what has been taken, or why it cannot be served; nothing is stated yet.
    pub(crate) fn opened(ask: Ask, taken: &Taken) -> Result<Self, Unservable> {
        Ok(match ask.metric {
            Metric::Series(series) => Self::Series(SeriesView::opened(series, ask.trailing, &taken.slices)?),
            Metric::State(series) => Self::State(StateView::opened(series, ask.trailing, taken)?),
            Metric::Report(series) => Self::Report(ReportView::opened(series, ask.trailing, taken)?),
            Metric::Summary(series) => Self::Summary(SummaryView::opened(series, ask.trailing, taken)?),
        })
    }

    pub(crate) fn descriptor(&self) -> MetricDescriptor {
        match self {
            Self::Series(view) => view.descriptor(),
            Self::State(view) => view.descriptor(),
            Self::Report(view) => view.descriptor(),
            Self::Summary(view) => view.descriptor(),
        }
    }

    /// Every row made final since the view last advanced, in order; or why the view can go no further. A view states
    /// the whole run when it advances over the end block, which happens once: the follower takes nothing behind the
    /// end block, and a view opened after it advances only as it opens.
    pub(crate) fn advance(&mut self, taken: &Taken) -> Result<Vec<MetricPoint>, Unservable> {
        match self {
            Self::Series(view) => Ok(view.advance(&taken.slices)),
            Self::State(view) => view.advance(taken),
            Self::Report(view) => view.advance(taken),
            Self::Summary(view) => view.advance(taken),
        }
    }
}
