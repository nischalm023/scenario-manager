//! The follower over simrunner's canned recording, three hours of a repository world written before the robot state
//! channel: every family's views as they come frame by frame, what each ask is refused, and a family that does not
//! read failing alone. `tests/live_and_rebuilt.rs` follows a recording that carries the channel.

use std::fs::{File, read};
use std::path::PathBuf;

use dessim::metrics::worldview::journals::VTM_OPERATIONS;
use desunits::{SimDuration, SimTime};
use rustc_hash::FxHashMap;
use serde_json::from_slice;
use studio_protocol::{
    Bucket, MetricPoint, MetricSource, MetricsFile, RunOutcome, SeriesFile, SeriesKind, SeriesPoint, TrailingSpec,
};
use worldview::container::reader::frame_at;
use worldview::frames::TABLES;
use worldview::frames::tables::TablesDelta;
use worldview::{End, Frame, Payload, Reader, Scenario, Sections, Tag};

use super::*;
use crate::metric::Metric;
use crate::reports::ReportSeries;
use crate::series::TimeSeries;
use crate::states::StateSeries;
use crate::summary::SummarySeries;

fn canned(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../simrunner/fixtures/canned").join(name)
}

/// The canned recording's frames in file order, its index block left out, as a runner sends them.
fn frames() -> Vec<(Tag, Vec<u8>)> {
    let path = canned("recording.worldview");
    let reader = Reader::open(&path).expect("the canned recording opens");
    let mut file = File::open(&path).expect("it reads");
    let records = reader.index().records().to_vec();
    records
        .into_iter()
        .filter(|record| record.tag != Tag::Index)
        .map(|record| (record.tag, frame_at(&mut file, record.offset).expect("a frame").to_bytes()))
        .collect()
}

fn ask(metric: Metric) -> Ask {
    Ask { metric, trailing: None }
}

fn asked(metric: Metric, window_ms: u64, step_ms: u64) -> Ask {
    let trailing = TrailingSpec { window_sim_ms: SimDuration::ms(window_ms), step_sim_ms: SimDuration::ms(step_ms) };
    Ask { metric, trailing: Some(trailing) }
}

/// Every frame through one follower with `asks` opened after its scenario block: each ask's rows in the order they
/// came, and the follower.
fn followed(frames: &[(Tag, Vec<u8>)], asks: &[Ask]) -> (FxHashMap<Ask, Vec<MetricPoint>>, Follower) {
    let mut follower = Follower::from_scenario(&frames[0].1).expect("the scenario block reads");
    let mut rows: FxHashMap<Ask, Vec<MetricPoint>> = FxHashMap::default();
    for ask in asks {
        rows.insert(*ask, follower.open(*ask).expect("served").rows);
    }
    for (_, bytes) in &frames[1..] {
        for made in follower.frame(bytes).expect("a frame reads") {
            rows.get_mut(&made.ask).expect("an open view").extend(made.rows.expect("the view goes on"));
        }
    }
    (rows, follower)
}

/// The canned run's committed `series.json`: every declared scalar at each slice end, as simrunner wrote it from the
/// recording's own scalars rather than through the follower.
fn committed_series() -> SeriesFile {
    from_slice(&read(canned("series.json")).expect("series.json reads")).expect("parses")
}

/// Each slice's own value of the scalar `key`, slice by slice from simtime 0: a running total's rise over the slice, a
/// gauge's reading in it, none in a slice before the run first stated it.
fn per_slice(file: &SeriesFile, key: &str) -> Vec<Option<f64>> {
    let series = file.metrics.iter().find(|series| series.metric == key).expect(key);
    let at: FxHashMap<u64, f64> =
        series.points.iter().filter_map(|SeriesPoint(at, value)| Some((at.as_ms(), (*value)?))).collect();
    let width = file.slice_sim_ms.as_ms();
    (1..=file.end_sim_ms.as_ms() / width)
        .map(|slice| {
            let value = *at.get(&(slice * width))?;
            Some(match series.kind {
                SeriesKind::Running => value - at.get(&((slice - 1) * width)).copied().unwrap_or(0.0),
                SeriesKind::Readings => value,
            })
        })
        .collect()
}

/// `rows`' values against `expected`, one row per slice, each to within rounding.
fn assert_values(rows: &[MetricPoint], expected: &[Option<f64>], what: &str) {
    assert_eq!(rows.len(), expected.len(), "{what}: one row per slice");
    for (slice, (row, expected)) in rows.iter().zip(expected).enumerate() {
        let close = match (row.value, *expected) {
            (Some(value), Some(expected)) => (value - expected).abs() <= 1e-9 * expected.abs().max(1.0),
            (value, expected) => value == expected,
        };
        assert!(close, "{what}, slice {slice}: {:?}, not {expected:?}", row.value);
    }
}

/// The canned scenario block with one metric, or one table, it declares left out.
fn scenario_without(metric: Option<&str>, table: Option<&str>) -> Vec<u8> {
    let (frame, _) = Frame::decode_at(&frames()[0].1, 0).expect("the scenario block decodes");
    let mut block = Scenario::from_bytes(&frame.contents(0).expect("its contents"), 0).expect("a scenario block");
    block.metrics.retain(|declared| Some(declared.name.as_str()) != metric);
    block.tables.retain(|declared| Some(declared.name.as_str()) != table);
    Frame::encode(Tag::Scenario, frame.header.t, &block.to_bytes().expect("encodes"), false).expect("a frame")
}

/// Three hours and a little at 10 s a slice, one row per slice from simtime 0, and a running total's last row the
/// committed `metrics.json`'s final value of its key, which the end block states: the running sum from the slices
/// reaches what the simulator itself counted.
#[test]
fn the_slices_reach_the_simulators_own_totals() {
    let frames = frames();
    let series = [
        TimeSeries::PicksPerSlice,
        TimeSeries::CumulativePicks,
        TimeSeries::CumulativeOrders,
        TimeSeries::CumulativePresentations,
    ];
    let asks = series.map(|series| ask(Metric::Series(series)));
    let (rows, follower) = followed(&frames, &asks);
    let picks = &rows[&asks[0]];
    assert!(picks.len() >= 1080, "three hours of 10 s slices at least: {}", picks.len());
    for (index, row) in picks.iter().enumerate() {
        assert_eq!(
            (row.from_sim_ms, row.to_sim_ms),
            (SimTime::ms(index as u64 * 10_000), row.from_sim_ms + SimDuration::ms(10_000))
        );
        assert!(row.value.is_some(), "slice {index} states its picks");
    }
    let committed: MetricsFile =
        from_slice(&read(canned("metrics.json")).expect("metrics.json reads")).expect("parses");
    let final_of = |key: &str| {
        committed.metrics.iter().find(|metric| metric.metric == key).and_then(|metric| metric.value).expect(key)
    };
    let last = |ask: &Ask| rows[ask].last().and_then(|row| row.value);
    assert_eq!(last(&asks[1]), Some(final_of("picks_completed")));
    assert_eq!(last(&asks[2]), Some(final_of("orders_completed")));
    assert_eq!(last(&asks[3]), Some(final_of("presentations")));
    assert_eq!(Some(picks.iter().filter_map(|row| row.value).sum::<f64>()), last(&asks[1]));
    assert!(rows[&asks[1]].iter().all(|row| row.from_sim_ms == SimTime::ZERO), "a running total spans the run");
    assert_eq!(follower.outcome(), Some(RunOutcome::SimTimeLimit));
}

/// The formulas desvis applies, read off one slice with picks, each series cut one slice to a window: a rate is the
/// slice's count per simulated hour. A robot count is duty over the slice's width, and the idle D30s the D30s stated in
/// any state less the busy ones: every slice's, against the figures the slice's own scalars in `series.json` give.
#[test]
fn the_formulas_read_as_desvis_computes_them() {
    let frames = frames();
    let one_slice = |series| asked(Metric::Series(series), 10_000, 10_000);
    let series = [
        TimeSeries::PicksPerSlice,
        TimeSeries::PicksPerHour,
        TimeSeries::Presentations,
        TimeSeries::PresentationsPerStationHour,
        TimeSeries::HtmTraveling,
        TimeSeries::HtmIdle,
        TimeSeries::SimSpeed,
    ];
    let asks = series.map(one_slice);
    let (rows, _) = followed(&frames, &asks);
    let busy = rows[&asks[0]].iter().position(|row| row.value > Some(0.0)).expect("a slice with picks");
    let value = |series: TimeSeries| rows[&one_slice(series)][busy].value.expect("a value");
    assert_eq!(value(TimeSeries::PicksPerHour), value(TimeSeries::PicksPerSlice) * 360.0);
    assert_eq!(value(TimeSeries::PresentationsPerStationHour), value(TimeSeries::Presentations) * 360.0 / 5.0);
    assert!(value(TimeSeries::SimSpeed) > 0.0, "the canned run took wall clock");
    let committed = committed_series();
    let slice = |key: &str| per_slice(&committed, key);
    let width = 10_000.0;
    let traveling: Vec<Option<f64>> =
        slice("htm_duty_traveling_ms").into_iter().map(|duty| duty.map(|duty| duty / width)).collect();
    let busy = ["htm_duty_traveling_ms", "htm_duty_loading_unloading_ms", "htm_duty_processing_ms"].map(slice);
    let states: Vec<Vec<Option<f64>>> = committed
        .metrics
        .iter()
        .filter(|series| series.metric.starts_with("htm_state_"))
        .map(|series| slice(&series.metric))
        .collect();
    let idle: Vec<Option<f64>> = (0..traveling.len())
        .map(|index| {
            let stated = states.iter().map(|state| state[index].unwrap_or(0.0)).sum::<f64>();
            let busy = busy.iter().map(|duty| duty[index]).sum::<Option<f64>>()?;
            Some((stated - busy / width).max(0.0))
        })
        .collect();
    assert!(traveling.iter().filter(|robots| **robots > Some(0.0)).count() > 1_000, "the canned D30s travel");
    assert!(idle.iter().filter(|robots| **robots > Some(0.0)).count() > 1_000, "and stand idle");
    assert_values(&rows[&one_slice(TimeSeries::HtmTraveling)], &traveling, "D30s traveling");
    assert_values(&rows[&one_slice(TimeSeries::HtmIdle)], &idle, "D30s idle");
}

/// The canned world's slices count rack storage slots and relays from the first (`rack_capacity`, `relay_cells_total`;
/// `metrics.json` states 0 for both, from an end block written before a run's totals stood at its last slice's
/// readings), so the seven series measuring them have in every slice the value desvis draws, which the slice's own
/// scalars in `series.json` give, and say in their descriptors when they would have none. Pres/stn/hr has a value in
/// every slice of the world's five stations, the rows of the scenario block's station table (the five its world
/// compiles to, as desvis counts them), and none in a recording whose table names no station, where desvis says
/// "Unavailable".
#[test]
fn a_series_measuring_stations_rack_slots_or_relays_has_a_value_in_a_world_with_them_and_none_without() {
    let frames = frames();
    let one_slice = |series| asked(Metric::Series(series), 10_000, 10_000);
    let measuring = [
        TimeSeries::RackOccupancy,
        TimeSeries::RackFillPercent,
        TimeSeries::RackMaxFillPercent,
        TimeSeries::RackFillMinAislePercent,
        TimeSeries::RackFillMaxAislePercent,
        TimeSeries::RelayOccupancyPercent,
        TimeSeries::RelayReservationUtilPercent,
        TimeSeries::PresentationsPerStationHour,
    ];
    let asks = measuring.map(one_slice);
    let (rows, mut follower) = followed(&frames, &asks);
    let committed = committed_series();
    let slice = |key: &str| per_slice(&committed, key);
    let percent = |part: &str, whole: &str| -> Vec<Option<f64>> {
        let wholes = slice(whole);
        slice(part).iter().zip(&wholes).map(|(part, whole)| Some(100.0 * (*part)? / (*whole)?)).collect()
    };
    let (reserved_ms, relays) = (slice("relay_reserved_busy_ms"), slice("relay_cells_total"));
    let reserved: Vec<Option<f64>> = reserved_ms
        .iter()
        .zip(&relays)
        .map(|(busy, relays)| Some(100.0 * (*busy)? / ((*relays)? * 10_000.0)))
        .collect();
    let per_station_hour = slice("presentations").into_iter().map(|count| Some(count? * 360.0 / 5.0)).collect();
    let (fill, relays_occupied) =
        (percent("rack_occupancy", "rack_capacity"), percent("relay_cells_reserved", "relay_cells_total"));
    assert!(fill.iter().any(|fill| *fill > Some(0.0)), "the canned rack holds totes");
    assert!(relays_occupied.iter().any(|occupied| *occupied > Some(0.0)), "and its relays reserve cells");
    let expected: [Vec<Option<f64>>; 8] = [
        slice("rack_occupancy"),
        fill,
        percent("rack_occupancy_peak", "rack_capacity"),
        slice("rack_fill_min_pct"),
        slice("rack_fill_max_pct"),
        relays_occupied,
        reserved,
        per_station_hour,
    ];
    for (ask, expected) in asks.iter().zip(&expected) {
        assert!(expected.iter().all(Option::is_some), "{ask:?}: every slice states its scalars");
        assert_values(&rows[ask], expected, &format!("{ask:?}"));
    }
    let mut described = |ask| follower.open(ask).expect("served").descriptor.description;
    assert!(described(asks[0]).ends_with("No value in a world without rack storage slots."));
    assert!(described(asks[5]).ends_with("No value in a world without relays."));
    assert!(described(asks[7]).ends_with("No value in a world without stations."));
    let per_station = asks[7];
    let (frame, _) = Frame::decode_at(&frames[0].1, 0).expect("the scenario block decodes");
    let mut block = Scenario::from_bytes(&frame.contents(0).expect("its contents"), 0).expect("a scenario block");
    assert_eq!(block.stations.len(), 5, "the canned world's five stations");
    block.stations.clear();
    let mut stationless = frames.clone();
    stationless[0].1 =
        Frame::encode(Tag::Scenario, frame.header.t, &block.to_bytes().expect("encodes"), false).expect("a frame");
    let (none, _) = followed(&stationless, &[per_station]);
    assert!(none[&per_station].len() >= 1080 && none[&per_station].iter().all(|row| row.value.is_none()));
}

/// A time series in its own bucketing, one row per slice, or in the trailing windows asked: a window at every step
/// once one fits, each exactly as wide as asked.
#[test]
fn a_time_series_follows_as_slices_or_as_the_windows_asked_for() {
    let frames = frames();
    let plain = ask(Metric::Series(TimeSeries::PicksPerSlice));
    let windows = asked(Metric::Series(TimeSeries::PicksPerSlice), 60_000, 30_000);
    let mut follower = Follower::from_scenario(&frames[0].1).expect("reads");
    assert_eq!(
        follower.open(plain).expect("served").descriptor.bucket,
        Bucket::Slice { width_sim_ms: SimDuration::ms(10_000) }
    );
    assert_eq!(
        follower.open(windows).expect("served").descriptor.bucket,
        Bucket::Trailing(windows.trailing.expect("asked"))
    );
    let (rows, _) = followed(&frames, &[windows]);
    let rows = &rows[&windows];
    assert_eq!(rows[0].to_sim_ms, SimTime::ms(60_000), "the first window ends once a whole one fits");
    assert!(
        rows.iter().all(
            |row| row.to_sim_ms.as_ms() % 30_000 == 0 && row.to_sim_ms - row.from_sim_ms == SimDuration::ms(60_000)
        )
    );
}

/// Trailing windows are served exactly as asked or refused before any row: windows that are not whole slices, and any
/// asked of a running total or of a figure the end block states once.
#[test]
fn a_trailing_ask_is_served_exactly_as_asked_or_refused() {
    let frames = frames();
    let mut follower = Follower::from_scenario(&frames[0].1).expect("reads");
    let picks = Metric::Series(TimeSeries::PicksPerSlice);
    let refused = [
        asked(picks, 25_000, 10_000),
        asked(picks, 30_000, 15_000),
        asked(Metric::Series(TimeSeries::CumulativePicks), 30_000, 10_000),
        asked(Metric::Summary(SummarySeries::TotalPicks), 30_000, 10_000),
        asked(Metric::Report(ReportSeries::Da50Operations), 25_000, 10_000),
    ];
    for ask in refused {
        let trailing = ask.trailing.expect("asked");
        assert_eq!(follower.open(ask), Err(Unservable::UnsupportedTrailing(trailing)), "{ask:?}");
    }
    for ask in [asked(picks, 30_000, 20_000), asked(Metric::Report(ReportSeries::Da50Operations), 1_200_000, 600_000)] {
        let served = follower.open(ask).expect("served").descriptor.bucket;
        assert_eq!(served, Bucket::Trailing(ask.trailing.expect("asked")), "{ask:?}");
    }
}

/// A recording made before the robot state channel does not record it, one that does not declare a scalar a formula
/// reads does not record that scalar, and one that does not declare a journal's table does not record that table;
/// what reads none of them is served.
#[test]
fn what_a_recording_does_not_declare_is_not_recorded() {
    let canned = frames();
    let mut follower = Follower::from_scenario(&canned[0].1).expect("reads");
    for series in StateSeries::ALL {
        let refused = follower.open(ask(Metric::State(series)));
        assert_eq!(refused, Err(Unservable::NotRecorded(Unrecorded::RobotState)), "{series:?}");
    }
    let mut follower = Follower::from_scenario(&scenario_without(Some("picks_completed"), None)).expect("reads");
    let refused = follower.open(ask(Metric::Series(TimeSeries::PicksPerHour)));
    assert_eq!(refused, Err(Unservable::NotRecorded(Unrecorded::Scalar("picks_completed"))));
    assert!(follower.open(ask(Metric::Series(TimeSeries::Presentations))).is_ok(), "reads no pick count");
    let mut follower = Follower::from_scenario(&scenario_without(Some("rack_capacity"), None)).expect("reads");
    let refused = follower.open(ask(Metric::Series(TimeSeries::RackOccupancy)));
    assert_eq!(
        refused,
        Err(Unservable::NotRecorded(Unrecorded::Scalar("rack_capacity"))),
        "the scalar that says whether the world has rack storage slots"
    );
    let mut follower = Follower::from_scenario(&scenario_without(None, Some(VTM_OPERATIONS))).expect("reads");
    let refused = follower.open(ask(Metric::Report(ReportSeries::Da50Operations)));
    assert_eq!(refused, Err(Unservable::NotRecorded(Unrecorded::Table(VTM_OPERATIONS))));
    assert!(follower.open(ask(Metric::Report(ReportSeries::StorageOccupied))).is_ok(), "reads another table");
}

/// A report series' windows come at the step, each once it is final; then, at the end block, the whole run from
/// simtime 0 to the run's end, marked as such.
#[test]
fn a_report_series_states_its_windows_as_they_become_final_and_the_whole_run_at_the_end() {
    let frames = frames();
    let operations = ask(Metric::Report(ReportSeries::Da50Operations));
    let (rows, follower) = followed(&frames, &[operations]);
    let rows = &rows[&operations];
    let windows: Vec<&MetricPoint> = rows.iter().filter(|row| !row.whole_run).collect();
    let whole: Vec<&MetricPoint> = rows.iter().filter(|row| row.whole_run).collect();
    assert!(!windows.is_empty() && !whole.is_empty(), "{} rows", rows.len());
    assert!(rows[rows.len() - whole.len()..].iter().all(|row| row.whole_run), "the whole run comes last");
    assert!(windows.iter().all(|row| row.to_sim_ms - row.from_sim_ms == SimDuration::ms(900_000)));
    assert_eq!(windows[0].to_sim_ms, SimTime::ms(900_000), "the first window ends at 15 min");
    let ends: Vec<SimTime> = windows.iter().map(|row| row.to_sim_ms).collect();
    assert!(ends.windows(2).all(|pair| pair[0] <= pair[1]), "in order");
    let (_, end_block) = frames.iter().find(|(tag, _)| *tag == Tag::End).expect("the end block");
    let (end, _) = Frame::decode_at(end_block, 0).expect("decodes");
    let run_end = SimTime::ms(end.header.t.0);
    assert!(
        whole.iter().all(|row| (row.from_sim_ms, row.to_sim_ms) == (SimTime::ZERO, run_end)),
        "the end block's instant"
    );
    assert_eq!(follower.reached(), run_end, "the follower has read to the end block, and no further");
}

/// While the run goes, the follower has read to the newest slice's start and no further: where that slice ends is
/// known only from the next frame.
#[test]
fn while_the_run_goes_the_follower_has_read_to_the_newest_slices_start() {
    let frames = frames();
    let mut follower = Follower::from_scenario(&frames[0].1).expect("the scenario block reads");
    for (tag, bytes) in frames[1..].iter().filter(|(tag, _)| *tag != Tag::End) {
        follower.frame(bytes).expect("a frame reads");
        if *tag == Tag::Delta {
            let (frame, _) = Frame::decode_at(bytes, 0).expect("decodes");
            assert_eq!(follower.reached(), instant(frame.header.t), "after the slice at {}", frame.header.t.0);
        }
    }
}

/// A summary series answers nothing until the end block, then its figure over the whole run, as the simulator's own
/// summary states it: the same picks the slices' running total reaches, but the one in flight as the run stopped.
#[test]
fn a_summary_series_answers_once_at_the_end_with_the_harnesss_own_figure() {
    let frames = frames();
    let total = ask(Metric::Summary(SummarySeries::TotalPicks));
    let cumulative = ask(Metric::Series(TimeSeries::CumulativePicks));
    let stations = ask(Metric::Summary(SummarySeries::StationUtilizationPercent));
    let (rows, _) = followed(&frames, &[total, cumulative, stations]);
    assert_eq!(rows[&total].len(), 1, "{:?}", rows[&total]);
    assert!(rows[&total][0].whole_run, "an end-block figure states the whole run");
    let (figure, counted) = (
        rows[&total][0].value.expect("a figure"),
        rows[&cumulative].last().and_then(|row| row.value).expect("a total"),
    );
    assert!((figure - counted).abs() <= 1.0, "summary {figure} against the slices' {counted}");
    assert!(rows[&stations].iter().all(|row| row.keys.len() == 1 && row.from_sim_ms == SimTime::ZERO));
    let mut follower = Follower::from_scenario(&frames[0].1).expect("reads");
    assert_eq!(follower.open(total).expect("served").descriptor.source, MetricSource::EndBlock);
}

/// One delta's journal rows that do not read fail the views built from the journals, then and at any later open, and
/// no other view: the slices go on.
#[test]
fn a_journal_that_does_not_read_fails_its_own_views_alone() {
    let mut frames = frames();
    let delta = frames.iter().position(|(tag, _)| *tag == Tag::Delta).expect("a delta");
    let (frame, _) = Frame::decode_at(&frames[delta].1, 0).expect("decodes");
    let sections = Sections::decode(frame.contents(0).expect("its contents"), 0).expect("its sections");
    let damaged: Vec<(u8, &[u8])> =
        sections.iter().map(|(id, bytes)| (id, if id == TABLES { &[0xff, 0xff, 0xff][..] } else { bytes })).collect();
    frames[delta].1 = Frame::encode(Tag::Delta, frame.header.t, &Sections::encode(&damaged), false).expect("a frame");

    let operations = ask(Metric::Report(ReportSeries::Da50Operations));
    let picks = ask(Metric::Series(TimeSeries::PicksPerSlice));
    let mut follower = Follower::from_scenario(&frames[0].1).expect("reads");
    follower.open(operations).expect("served until the rows fail");
    follower.open(picks).expect("served");
    let mut failed = Vec::new();
    let mut picked = 0;
    for (_, bytes) in &frames[1..] {
        for made in follower.frame(bytes).expect("every frame reads") {
            match made.rows {
                Ok(rows) => picked += usize::from(made.ask == picks) * rows.len(),
                Err(fault) => failed.push((made.ask, fault)),
            }
        }
    }
    assert!(matches!(failed.as_slice(), [(ask, Unservable::Unreadable(_))] if *ask == operations), "{failed:?}");
    assert!(picked >= 1080, "the slices went on: {picked}");
    assert!(matches!(follower.open(operations), Err(Unservable::Unreadable(_))), "and stay failed for a later socket");
}

/// One table's rows that do not read, the frame's other tables reading, fail only the views of that table's journal:
/// the DA50 operations fail while the occupancy and the presentations, read from other journals, state every window
/// and the whole run, as they do from the undamaged recording.
#[test]
fn one_journal_that_does_not_read_fails_the_views_of_that_journal_alone() {
    let mut frames = frames();
    let block =
        Scenario::from_bytes(&Frame::decode_at(&frames[0].1, 0).expect("decodes").0.contents(0).expect("it"), 0)
            .expect("a scenario block");
    let operations_table = block.tables.iter().find(|table| table.name == VTM_OPERATIONS).expect("declared").id;
    let damaged = frames
        .iter()
        .position(|(tag, bytes)| {
            *tag == Tag::Delta && {
                let (frame, _) = Frame::decode_at(bytes, 0).expect("decodes");
                let sections = Sections::decode(frame.contents(0).expect("its contents"), 0).expect("its sections");
                sections.get(TABLES).is_some_and(|bytes| {
                    TablesDelta::from_bytes(bytes, 0).expect("reads").get(operations_table).is_some()
                })
            }
        })
        .expect("a delta with DA50 operations");
    let (frame, _) = Frame::decode_at(&frames[damaged].1, 0).expect("decodes");
    let sections = Sections::decode(frame.contents(0).expect("its contents"), 0).expect("its sections");
    let mut tables = TablesDelta::from_bytes(sections.get(TABLES).expect("tables"), 0).expect("reads");
    for rows in tables.0.iter_mut().filter(|rows| rows.table == operations_table) {
        rows.rows = vec![0xff; 3];
    }
    let tables = tables.to_bytes().expect("encodes");
    let sections: Vec<(u8, &[u8])> =
        sections.iter().map(|(id, bytes)| (id, if id == TABLES { tables.as_slice() } else { bytes })).collect();
    let undamaged = frames.clone();
    frames[damaged].1 =
        Frame::encode(Tag::Delta, frame.header.t, &Sections::encode(&sections), false).expect("a frame");

    let operations = ask(Metric::Report(ReportSeries::Da50Operations));
    let occupancy = ask(Metric::Report(ReportSeries::StorageOccupied));
    let presentations = ask(Metric::Report(ReportSeries::PresentationsByAisleStation));
    let (expected, _) = followed(&undamaged, &[occupancy, presentations]);
    let mut follower = Follower::from_scenario(&frames[0].1).expect("reads");
    let mut rows: FxHashMap<Ask, Vec<MetricPoint>> = FxHashMap::default();
    for ask in [operations, occupancy, presentations] {
        rows.insert(ask, follower.open(ask).expect("served until the rows fail").rows);
    }
    let mut failed = Vec::new();
    for (_, bytes) in &frames[1..] {
        for made in follower.frame(bytes).expect("every frame reads") {
            match made.rows {
                Ok(made_rows) => rows.get_mut(&made.ask).expect("open").extend(made_rows),
                Err(fault) => failed.push((made.ask, fault)),
            }
        }
    }
    assert!(matches!(failed.as_slice(), [(ask, Unservable::Unreadable(_))] if *ask == operations), "{failed:?}");
    for ask in [occupancy, presentations] {
        assert!(rows[&ask].iter().any(|row| row.whole_run), "{ask:?} reached the whole run");
        assert_eq!(rows[&ask], expected[&ask], "{ask:?}");
    }
    assert!(matches!(follower.open(operations), Err(Unservable::Unreadable(_))), "and stays failed for a later socket");
    assert!(follower.open(occupancy).is_ok());
}

/// A view closed, its last socket gone, states nothing more; the views still open go on.
#[test]
fn a_closed_view_states_nothing_more() {
    let frames = frames();
    let picks = ask(Metric::Series(TimeSeries::PicksPerSlice));
    let total = ask(Metric::Series(TimeSeries::CumulativePicks));
    let mut follower = Follower::from_scenario(&frames[0].1).expect("reads");
    follower.open(picks).expect("served");
    follower.open(total).expect("served");
    follower.close(&picks);
    let made: Vec<Made> =
        frames[1..].iter().flat_map(|(_, bytes)| follower.frame(bytes).expect("every frame reads")).collect();
    assert!(made.iter().all(|made| made.ask == total) && !made.is_empty(), "only the open view states rows");
}

/// An end block whose stop reason no outcome names, as a later simulator's might, is a recording that does not read:
/// the follower cannot say how the run ended.
#[test]
fn an_end_block_whose_stop_reason_no_outcome_names_does_not_read() {
    let frames = frames();
    let (index, (_, end_block)) =
        frames.iter().enumerate().find(|(_, (tag, _))| *tag == Tag::End).expect("an end block");
    let (frame, _) = Frame::decode_at(end_block, 0).expect("decodes");
    let mut end = End::from_bytes(&frame.contents(0).expect("its contents"), 0).expect("an end block");
    end.stop_reason = "Paused".to_owned();
    let paused = Frame::encode(Tag::End, frame.header.t, &end.to_bytes().expect("encodes"), true).expect("a frame");
    let mut follower = Follower::from_scenario(&frames[0].1).expect("reads");
    for (_, bytes) in &frames[1..index] {
        follower.frame(bytes).expect("a frame reads");
    }
    let refused = follower.frame(&paused).expect_err("no outcome names `Paused`");
    assert!(refused.to_string().contains("Paused"), "{refused}");
}

/// A frame that does not read, or a scenario block where none may be, fails the follower itself.
#[test]
fn a_frame_that_does_not_read_fails_the_whole_recording() {
    let frames = frames();
    let mut follower = Follower::from_scenario(&frames[0].1).expect("reads");
    assert!(follower.frame(&frames[0].1).is_err(), "a second scenario block");
    assert!(follower.frame(&[0x00, 0x01]).is_err(), "no frame at all");
    assert!(Follower::from_scenario(&frames[1].1).is_err(), "a recording opens with its scenario block");
}
