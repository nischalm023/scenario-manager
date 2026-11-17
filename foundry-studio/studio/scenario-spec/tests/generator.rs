//! Spec 07 section 8, statement 5, the generator's property: one body gives one stream, made line by line or whole,
//! the seed changes the orders, the order count follows the rate and the duration (random arrivals within 1 % on
//! average over 1,000 seeds), each release arrives as section 3.2 states, the stock covers every order, and the start
//! stock is the fill of the slots.

use desunits::{SimDuration, SimTime};
use rustc_hash::FxHashMap;
use scenario_spec::{SiteFacts, WorkloadLines, generate, generate_request, ndjson_line, stated_defaults};
use studio_protocol::{
    FillPercent, GenerateRequest, GeneratorSpec, OrdersPerHour, Release, Seed, SlotCount, UnitsPerTote,
};

const HOUR: u64 = 3_600_000;

fn spec(release: Release, rate: f64, seed: u64) -> GeneratorSpec {
    let mut spec = stated_defaults().workload.generator;
    spec.release = release;
    spec.orders_per_hour = OrdersPerHour(rate);
    spec.seed = Seed(seed);
    if release == Release::Waves {
        spec.wave_every_sim_ms = Some(SimDuration::ms(900_000));
    }
    spec
}

const FACTS: SiteFacts = SiteFacts { slots: SlotCount(1_200), brings_stock_in: true };

#[test]
fn one_body_gives_byte_identical_streams_and_the_endpoint_gives_the_same() {
    let body = spec(Release::Random, 300.0, 7);
    let duration = SimDuration::ms(3 * HOUR);
    let (first, second) = (generate(&body, duration, FACTS), generate(&body, duration, FACTS));
    assert_eq!(first.ndjson(), second.ndjson());
    let request = GenerateRequest {
        v: 1,
        generator: body,
        duration_sim_ms: duration,
        slots: FACTS.slots,
        brings_stock_in: FACTS.brings_stock_in,
    };
    assert_eq!(generate_request(&request).expect("the body reads").ndjson(), first.ndjson());
    assert_eq!(first.sha256().len(), 64);
}

/// Spec 07 section 3.8's stream, made as it is sent (Astra's run-chain review, the tenth finding): the lines
/// `WorkloadLines` makes one at a time are, byte for byte, the whole workload's stream, for every release.
#[test]
fn the_lines_made_one_at_a_time_are_the_whole_workloads_stream() {
    let duration = SimDuration::ms(2 * HOUR);
    for release in [Release::AllAtStart, Release::Even, Release::Waves, Release::Random] {
        let body = spec(release, 400.0, 11);
        let mut streamed = Vec::new();
        for line in WorkloadLines::new(&body, duration, FACTS) {
            ndjson_line(&line, &mut streamed);
        }
        assert_eq!(streamed, generate(&body, duration, FACTS).ndjson(), "{release:?}");
    }
}

/// The endpoint's body is refused, naming the field, when its duration lies outside one minute to seven days of
/// simtime, as a version's own duration is, before any order is made.
#[test]
fn the_endpoint_refuses_a_duration_outside_a_minute_to_seven_days_of_simtime() {
    for ms in [0, 59_999, 604_800_001] {
        let request = GenerateRequest {
            v: 1,
            generator: spec(Release::AllAtStart, 300.0, 0),
            duration_sim_ms: SimDuration::ms(ms),
            slots: FACTS.slots,
            brings_stock_in: FACTS.brings_stock_in,
        };
        let refused = generate_request(&request).expect_err("a duration outside the range is refused");
        let pointers: Vec<&str> = refused.iter().map(|violation| violation.pointer.as_str()).collect();
        assert_eq!(pointers, ["/duration_sim_ms"], "{ms} ms");
    }
}

#[test]
fn the_seed_changes_the_orders_and_seeds_zero_and_one_differ() {
    let duration = SimDuration::ms(HOUR);
    for release in [Release::AllAtStart, Release::Random] {
        let zero = generate(&spec(release, 300.0, 0), duration, FACTS);
        let one = generate(&spec(release, 300.0, 1), duration, FACTS);
        assert_ne!(zero.orders, one.orders, "{release:?}");
    }
}

#[test]
fn the_rate_times_the_duration_gives_the_count_of_every_release_but_random() {
    for (rate, hours) in [(300.0, 3), (1.0, 1), (437.5, 2), (10_000.0, 1)] {
        let duration = SimDuration::ms(hours * HOUR);
        let expected = (rate * hours as f64).round() as usize;
        for release in [Release::AllAtStart, Release::Even, Release::Waves] {
            let made = generate(&spec(release, rate, 3), duration, FACTS);
            assert_eq!(made.orders.len(), expected, "{release:?} at {rate} an hour for {hours} h");
            assert_eq!(made.header.orders.0, expected as u64);
        }
    }
}

#[test]
fn random_arrivals_average_the_rate_times_the_duration_over_a_thousand_seeds() {
    let duration = SimDuration::ms(3 * HOUR);
    let total: usize =
        (0..1_000).map(|seed| generate(&spec(Release::Random, 300.0, seed), duration, FACTS).orders.len()).sum();
    let mean = total as f64 / 1_000.0;
    assert!((mean - 900.0).abs() <= 9.0, "the mean count over 1,000 seeds is {mean}, more than 1 % from 900");
}

#[test]
fn each_release_arrives_ordered_inside_the_duration_as_stated() {
    let duration = SimDuration::ms(3 * HOUR);
    let end = SimTime::ms(3 * HOUR);
    for release in [Release::AllAtStart, Release::Even, Release::Waves, Release::Random] {
        let made = generate(&spec(release, 300.0, 11), duration, FACTS);
        let arrivals: Vec<u64> = made.orders.iter().map(|order| order.arrival_ms.as_ms()).collect();
        assert!(arrivals.windows(2).all(|pair| pair[0] <= pair[1]), "{release:?} arrives in order");
        assert!(made.orders.iter().all(|order| order.arrival_ms < end), "{release:?} arrives inside the duration");
        for (i, at) in arrivals.iter().enumerate() {
            let even = (i as f64 * 12_000.0) as u64;
            match release {
                Release::AllAtStart => assert_eq!(*at, 0),
                Release::Even => assert_eq!(*at, even),
                Release::Waves => assert_eq!(*at, even / 900_000 * 900_000),
                Release::Random => {}
            }
        }
    }
}

#[test]
fn the_stock_covers_every_order_and_a_line_is_one_unit_of_a_sku_none_twice_in_an_order() {
    for brings_stock_in in [true, false] {
        let facts = SiteFacts { slots: SlotCount(300), brings_stock_in };
        let made = generate(&spec(Release::Random, 500.0, 5), SimDuration::ms(2 * HOUR), facts);
        let stock: FxHashMap<&str, u64> = made.items.iter().map(|item| (item.sku.as_str(), item.units.0)).collect();
        let mut demand: FxHashMap<&str, u64> = FxHashMap::default();
        for order in &made.orders {
            let mut skus: Vec<&str> = order.lines.iter().map(|line| line.sku.as_str()).collect();
            skus.dedup();
            assert_eq!(skus.len(), order.lines.len(), "no SKU twice in {}", order.id);
            for line in &order.lines {
                assert_eq!(line.quantity.0, 1);
                *demand.entry(line.sku.as_str()).or_default() += 1;
            }
        }
        for (sku, wanted) in &demand {
            assert!(stock.get(sku).is_some_and(|held| held >= wanted), "{sku}: {wanted} wanted, stock covers it");
        }
        assert_eq!(made.header.lines.0, demand.values().sum::<u64>());
    }
}

#[test]
fn the_start_stock_is_the_fill_of_the_slots_and_the_preload_its_share() {
    let mut body = spec(Release::AllAtStart, 300.0, 9);
    body.inventory_fill_pct = FillPercent(80.0);
    body.units_per_stored_tote = UnitsPerTote(4);
    let made = generate(&body, SimDuration::ms(3 * HOUR), FACTS);
    let total: u64 = made.items.iter().map(|item| item.units.0).sum();
    let start = total - made.header.lines.0;
    assert_eq!(start, 960 * 4, "80 % of 1,200 slots, 4 units a tote");
    assert!((made.header.preload_fraction.0 - start as f64 / total as f64).abs() < 1e-12);

    let without = generate(&body, SimDuration::ms(3 * HOUR), SiteFacts { brings_stock_in: false, ..FACTS });
    assert_eq!(without.header.preload_fraction.0, 1.0, "every unit starts on the racks without putaway");
}
