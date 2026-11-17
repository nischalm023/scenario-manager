//! Within `scenario-spec`, `generator` is spec 07 section 3.2's workload generator (rulings D91 and D96): a pure
//! function from a version's generator spec, the run's effective duration and two facts of the site to the orders
//! and stock a run reads, the same bytes for the same inputs from every build of this crate's version. Arrivals come
//! first, then each order's lines in arrival order, then the stock that covers them. Nothing is stored: the endpoint
//! streams its lines as `WorkloadLines` makes them (spec 07 section 3.8), and the check and the runner write dessim's
//! two normalized files from the same workload.

use std::cmp::Reverse;
use std::iter::{from_fn, once};
use std::vec::IntoIter;

use desunits::{SimDuration, SimTime};
use sha2::{Digest, Sha256};
use studio_protocol::{
    GenerateRequest, GeneratorSpec, OrderLine, PreloadFraction, Release, Tally, WorkloadHeader, WorkloadItem,
    WorkloadLine, WorkloadOrder, WorkloadSummary,
};

use crate::content::{HOUR_MS, Violation, duration, validate_generator};
use crate::rng::{Stream, StreamOf};
use crate::site::SiteFacts;

/// The generator's version, which the stream's header names and a manifest records.
pub const GENERATOR_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The endpoint body's version this generator reads.
pub const REQUEST_VERSION: u32 = 1;

/// What the generator made: the stream's header, the SKUs holding stock in SKU order, the orders in arrival order.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    pub header: WorkloadHeader,
    pub items: Vec<WorkloadItem>,
    pub orders: Vec<WorkloadOrder>,
}

/// Generate the workload a run of `duration` reads on a site with `facts`, whole.
pub fn generate(spec: &GeneratorSpec, duration: SimDuration, facts: SiteFacts) -> Generated {
    let mut lines = WorkloadLines::new(spec, duration, facts);
    let header = lines.header.take().expect("the stream opens with its header");
    let items = lines.items.by_ref().collect();
    let orders = from_fn(|| lines.order()).collect();
    Generated { header, items, orders }
}

/// The endpoint's body checked as a version's own content is: its version, its spec and its duration, each violation
/// naming its field; none when the generator may make it.
pub fn check_request(request: &GenerateRequest) -> Vec<Violation> {
    if request.v != REQUEST_VERSION {
        let rule = format!("is {REQUEST_VERSION}, the body's version this generator reads; this is {}", request.v);
        return vec![Violation { pointer: "/v".to_owned(), rule }];
    }
    let mut found = validate_generator(&request.generator, "/generator");
    duration(&mut found, "/duration_sim_ms", request.duration_sim_ms);
    found
}

/// The endpoint's body generated whole: refused, naming the field, when its version, its spec or its duration does
/// not pass the checks a version's own content passes.
pub fn generate_request(request: &GenerateRequest) -> Result<Generated, Vec<Violation>> {
    let found = check_request(request);
    if !found.is_empty() {
        return Err(found);
    }
    Ok(generate(&request.generator, request.duration_sim_ms, facts_of(request)))
}

/// The endpoint body's bytes as a runner sends them and its manifest hashes them: RFC 8785 canonical JSON.
pub fn request_bytes(request: &GenerateRequest) -> Vec<u8> {
    serde_jcs::to_vec(request).expect("a generator request is JSON")
}

pub fn facts_of(request: &GenerateRequest) -> SiteFacts {
    SiteFacts { slots: request.slots, brings_stock_in: request.brings_stock_in }
}

/// The workload's stream, line by line, each made when it is asked for, so the endpoint sends each as it is made and
/// nothing holds every order at once: a first pass over the arrivals and their lines counts the demand the header and
/// the stock need; the orders then come again in arrival order, drawn anew from the same seed.
pub struct WorkloadLines {
    header: Option<WorkloadHeader>,
    items: IntoIter<WorkloadItem>,
    arrivals: Box<dyn Iterator<Item = SimTime> + Send>,
    lines: Stream,
    lines_per_order: f64,
    sku_count: u64,
    skus: Namer,
    orders: Namer,
    made: u64,
}

impl WorkloadLines {
    pub fn new(spec: &GeneratorSpec, duration: SimDuration, facts: SiteFacts) -> Self {
        let sku_count = u64::from(spec.sku_count.0.max(1));
        let lines_per_order = spec.lines_per_order.0;
        let (mut demand, mut counted) = (vec![0u64; sku_count as usize], 0u64);
        let mut lines = Stream::new(spec.seed, StreamOf::Lines);
        for _ in arrivals(spec, duration) {
            for sku in draw(&mut lines, lines_per_order, sku_count) {
                demand[sku as usize] += 1;
            }
            counted += 1;
        }
        let skus = Namer::new("sku", sku_count);
        let (units, preload) = stock(spec, &demand, facts);
        let items: Vec<WorkloadItem> = units
            .iter()
            .enumerate()
            .filter(|(_, units)| **units > 0)
            .map(|(sku, units)| WorkloadItem { sku: skus.name(sku as u64), units: Tally(*units) })
            .collect();
        let header = WorkloadHeader {
            v: REQUEST_VERSION,
            generator: GENERATOR_VERSION.to_owned(),
            orders: Tally(counted),
            lines: Tally(demand.iter().sum()),
            skus: Tally(items.len() as u64),
            preload_fraction: preload,
        };
        Self {
            header: Some(header),
            items: items.into_iter(),
            arrivals: arrivals(spec, duration),
            lines: Stream::new(spec.seed, StreamOf::Lines),
            lines_per_order,
            sku_count,
            skus,
            orders: Namer::new("order", counted),
            made: 0,
        }
    }

    /// The next order in arrival order, its lines drawn as the first pass drew them.
    fn order(&mut self) -> Option<WorkloadOrder> {
        let arrival_ms = self.arrivals.next()?;
        let skus = draw(&mut self.lines, self.lines_per_order, self.sku_count);
        let lines = skus.iter().map(|&sku| OrderLine { sku: self.skus.name(sku), quantity: Tally(1) }).collect();
        let order = WorkloadOrder { id: self.orders.name(self.made), arrival_ms, lines };
        self.made += 1;
        Some(order)
    }
}

impl Iterator for WorkloadLines {
    type Item = WorkloadLine;

    fn next(&mut self) -> Option<WorkloadLine> {
        if let Some(header) = self.header.take() {
            return Some(WorkloadLine::Header(header));
        }
        self.items.next().map(WorkloadLine::Item).or_else(|| self.order().map(WorkloadLine::Order))
    }
}

/// One order's SKUs, in SKU order: its line count, then distinct SKUs, from the lines stream.
fn draw(lines: &mut Stream, lines_per_order: f64, sku_count: u64) -> Vec<u64> {
    let count = (1 + u64::from(lines.poisson(lines_per_order - 1.0))).min(sku_count);
    let mut skus: Vec<u64> = Vec::with_capacity(count as usize);
    while (skus.len() as u64) < count {
        let sku = lines.below(sku_count);
        if !skus.contains(&sku) {
            skus.push(sku);
        }
    }
    skus.sort_unstable();
    skus
}

/// Each order's arrival, in arrival order: every one inside the duration.
fn arrivals(spec: &GeneratorSpec, duration: SimDuration) -> Box<dyn Iterator<Item = SimTime> + Send> {
    let (rate, end) = (spec.orders_per_hour.0, duration.as_ms());
    let gap = HOUR_MS as f64 / rate;
    let count = (rate * end as f64 / HOUR_MS as f64).round() as u64;
    match spec.release {
        Release::AllAtStart => Box::new((0..count).map(|_| SimTime::ms(0))),
        Release::Even => Box::new((0..count).map(move |i| SimTime::ms((i as f64 * gap) as u64))),
        Release::Waves => {
            let every = spec.wave_every_sim_ms.map_or(HOUR_MS, SimDuration::as_ms).max(1);
            Box::new((0..count).map(move |i| SimTime::ms((i as f64 * gap) as u64 / every * every)))
        }
        Release::Random => {
            let (mut stream, mut at) = (Stream::new(spec.seed, StreamOf::Arrivals), 0.0);
            Box::new(from_fn(move || {
                at += stream.exponential(gap);
                (at < end as f64).then(|| SimTime::ms(at as u64))
            }))
        }
    }
}

/// Each SKU's units, and the share of them on the racks at simtime 0. The fill's totes go to the SKUs in proportion
/// to their demand by largest remainder, in exact integers; with putaway in the flow an item holds its start plus
/// what the orders take, else every unit starts on the racks.
fn stock(spec: &GeneratorSpec, demand: &[u64], facts: SiteFacts) -> (Vec<u64>, PreloadFraction) {
    let totes = (spec.inventory_fill_pct.0 / 100.0 * facts.slots.0 as f64).round() as u64;
    let per_tote = u64::from(spec.units_per_stored_tote.0.max(1));
    let total: u64 = demand.iter().sum();
    let mut share: Vec<u64> = vec![0; demand.len()];
    if total > 0 {
        let exact = |d: u64| u128::from(totes) * u128::from(d);
        let mut given = 0;
        for (sku, &d) in demand.iter().enumerate() {
            share[sku] = (exact(d) / u128::from(total)) as u64;
            given += share[sku];
        }
        let mut order: Vec<usize> = (0..demand.len()).collect();
        order.sort_by_key(|&sku| (Reverse(exact(demand[sku]) % u128::from(total)), sku));
        for &sku in order.iter().take((totes - given) as usize) {
            share[sku] += 1;
        }
    }
    let start: Vec<u64> = share.iter().map(|totes| totes * per_tote).collect();
    if !facts.brings_stock_in {
        let units = start.iter().zip(demand).map(|(start, d)| (*start).max(*d)).collect();
        return (units, PreloadFraction(1.0));
    }
    let units: Vec<u64> = start.iter().zip(demand).map(|(start, d)| start + d).collect();
    let (on_racks, all): (u64, u64) = (start.iter().sum(), units.iter().sum());
    (units, PreloadFraction(if all == 0 { 0.0 } else { on_racks as f64 / all as f64 }))
}

/// `sku-001` or `order-001`: numbered from 1, zero-padded to the widest number, so dessim's reader, which orders
/// orders by their ids' text, keeps them in arrival order.
struct Namer {
    prefix: &'static str,
    width: usize,
}

impl Namer {
    fn new(prefix: &'static str, count: u64) -> Self {
        Self { prefix, width: count.max(1).to_string().len() }
    }

    fn name(&self, index: u64) -> String {
        format!("{}-{:0width$}", self.prefix, index + 1, width = self.width)
    }
}

/// One line of the stream onto `out`: RFC 8785 canonical JSON, ended by a newline.
pub fn ndjson_line(line: &WorkloadLine, out: &mut Vec<u8>) {
    out.extend(serde_jcs::to_vec(line).expect("a workload line is strings and integers"));
    out.push(b'\n');
}

impl Generated {
    /// The stream's lines: the header, an item per SKU holding stock, an order per order.
    pub fn lines(&self) -> impl Iterator<Item = WorkloadLine> + '_ {
        once(WorkloadLine::Header(self.header.clone()))
            .chain(self.items.iter().cloned().map(WorkloadLine::Item))
            .chain(self.orders.iter().cloned().map(WorkloadLine::Order))
    }

    pub fn ndjson(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for line in self.lines() {
            ndjson_line(&line, &mut out);
        }
        out
    }

    /// The sha256 of the stream's bytes, in lowercase hex: what a run's manifest and the check's expectation record.
    pub fn sha256(&self) -> String {
        Sha256::digest(self.ndjson()).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn summary(&self) -> WorkloadSummary {
        let header = &self.header;
        WorkloadSummary {
            orders: header.orders,
            lines: header.lines,
            skus: header.skus,
            preload_fraction: header.preload_fraction,
        }
    }

    /// dessim's normalized items file, `sku,weight_kg,inventory_units`; every weight is 0 kg, as dessim's own
    /// generator writes.
    pub fn items_csv(&self) -> Vec<u8> {
        let mut out = String::from("sku,weight_kg,inventory_units\n");
        for item in &self.items {
            out.push_str(&format!("{},0,{}\n", item.sku, item.units.0));
        }
        out.into_bytes()
    }

    /// dessim's normalized orders file, `order_id,sku,quantity,weight_kg,arrival_ms`, in arrival order; dessim reads
    /// no `arrival_ms` yet (BACKLOG task 46).
    pub fn orders_csv(&self) -> Vec<u8> {
        let mut out = String::from("order_id,sku,quantity,weight_kg,arrival_ms\n");
        for order in &self.orders {
            for line in &order.lines {
                let arrival = order.arrival_ms.as_ms();
                out.push_str(&format!("{},{},{},0,{arrival}\n", order.id, line.sku, line.quantity.0));
            }
        }
        out.into_bytes()
    }
}
