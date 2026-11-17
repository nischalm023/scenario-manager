//! desvis -- DES warehouse simulation and analysis tool.
//!
//! Usage:
//!   desvis map.json                                     # GUI, manual start
//!   desvis map.json --run                               # GUI, auto-start
//!   desvis map.json --test                              # GUI, auto-start + exit
//!   desvis map.json --headless                          # headless benchmark
//!   desvis map.json --export-png out.png                # render map to PNG, no GUI
//!   desvis --recording <run.worldview>                  # GUI playback
//!   desvis --recording <r.worldview> --dump-render <ms,ms,...> <dir>
//!                                                       # headless render dump
//!   desvis repeat <run.worldview>                       # re-run a recording
//!   desvis compare <old.worldview> <new.worldview>      # deep comparison
//!   desvis fpi <map.json> [--pick-time <secs>]          # fetch metrics
//!   desvis info <map.json> [-v]                         # map topology
//!   desvis route <map.json> <from> <to> [--robot htm|vtm|rtp] [--laden] [--kinematic]  # path diagnostics

use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Parser, Subcommand};
use native_modules as _;

fn parse_duration_arg(s: &str) -> Result<Duration, String> {
    dessim::utils::parse_human_duration(s)
}

mod app;
mod bench;
mod carton_line_prop;
mod compare;
mod entity_panel;
mod export_png;
mod fpi;
mod gantt;
mod graph;
mod gui_launch;
mod info;
mod metrics_panel;
mod output;
mod route;
mod screen;
mod sidebar;
mod sim;
mod sim_config_display;
mod station_diag;
mod theme;
mod traffic;
#[cfg(feature = "walmart-demo")]
mod walmart_demo;
mod world_cli;
// mod tooltip; -- RETIRED with the entity hover card: the map's one hover is
// the entity explorer's reading (entity_panel::map_hover_card), which folded
// in the cell facts this module used to say. File kept for the review diff.
mod types;

#[derive(Parser)]
#[command(name = "desvis", about = "DES warehouse simulation and analysis tool")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    // --- GUI mode args (used when command is None) ---
    #[command(flatten)]
    gui: GuiArgs,
}

#[derive(clap::Args)]
pub(crate) struct GuiArgs {
    /// Path to warehouse map JSON file (live simulation mode).
    map: Option<PathBuf>,
    /// Path to a .worldview recording for playback mode.
    #[arg(long)]
    recording: Option<PathBuf>,
    /// Write the arrays the renderer would upload at each listed simulated
    /// instant into a directory, instead of opening a window. Takes a
    /// comma-separated millisecond list and the directory.
    #[arg(
        long,
        num_args = 2,
        value_names = ["TICKS_MS", "OUT_DIR"],
        action = ArgAction::Set,
        requires = "recording"
    )]
    dump_render: Option<Vec<String>>,
    /// Auto-start simulation on launch (GUI stays open for observation).
    #[arg(long)]
    run: bool,
    /// Auto-start simulation, wait for completion, then exit (for CI/testing).
    #[arg(long)]
    test: bool,
    /// Time-slice width in seconds. Unset leaves the spec's own width.
    #[arg(long)]
    time_slice: Option<u64>,
    /// Comma-separated list of metrics for the time-series view.
    #[arg(long)]
    metrics: Option<String>,
    /// SimConfig overrides as key=value pairs (repeat --settings for each).
    #[arg(long)]
    settings: Vec<String>,
    /// Use only the first N stations.
    #[arg(long)]
    stations: Option<usize>,
    /// Limit to the first N orders.
    #[arg(long)]
    orders: Option<usize>,
    /// Executor threads for the run, recorded on the cover sheet. Unset leaves the spec's own count.
    #[arg(long)]
    threads: Option<usize>,
    /// Simulation duration override. Accepts human-friendly forms:
    /// `1h`, `1h30m`, `1h 30m`, `1.5h`, `.5d`, `500ms`. Bare numbers are ms.
    #[arg(long, value_parser = parse_duration_arg)]
    simtime: Option<Duration>,
    /// Run without GUI (headless benchmark mode). Writes a .worldview.
    #[arg(long)]
    headless: bool,
    /// Output directory for the recording each run writes.
    #[arg(long, default_value = "perf/profiles")]
    output_dir: PathBuf,
    /// Human-readable name for this run (headless mode).
    #[arg(long)]
    name: Option<String>,
    /// Allow incomplete runs (headless mode).
    #[arg(long)]
    allow_incomplete: bool,
    /// Print event log to stderr.
    #[arg(long)]
    events: bool,
    /// Render a live progress bar to stderr during headless runs: sim time
    /// vs. desired sim duration, picks, picks/hr, and orders completed.
    #[arg(long)]
    progress: bool,
    /// Enable structured simulator trace output.
    #[arg(long)]
    trace: bool,
    /// Trace filter clause. Comma-separated tokens are AND'd within a
    /// clause; pass `--trace-filter` multiple times to OR clauses
    /// (e.g. `--trace-filter station=stn1,trace_name=presentation_committed
    /// --trace-filter station=stn1,trace_name=pick_dispatch_committed`).
    #[arg(long)]
    trace_filter: Vec<String>,
    /// SimTime trace window, e.g. 60s..180s.
    #[arg(long)]
    trace_window: Option<String>,
    /// Trace output format: text or jsonl.
    #[arg(long, default_value = "text")]
    trace_format: String,
    /// Write trace records to a file instead of stderr.
    #[arg(long)]
    trace_out: Option<PathBuf>,
    /// Movement fidelity mode: ghost, collision-des, tick-stepper.
    #[arg(long)]
    fidelity: Option<String>,
    /// Disable the task completion watchdog.
    #[arg(long)]
    no_watchdog: bool,
    /// Enable strict throughput diagnostic tripwires (station-presentation
    /// gap-abort and tight pathfinder horizons). Off by default so cencora-
    /// scale runs complete; turn on while tuning throughput / debugging
    /// timing drift.
    #[arg(long)]
    tripwires: bool,
    /// Wall-clock timeout. Stops the simulation gracefully after this much
    /// real time has elapsed. Accepts human-friendly forms: `30s`, `5m`,
    /// `1h30m`, `1.5h`, `500ms`. Bare numbers are ms.
    #[arg(long, value_parser = parse_duration_arg)]
    wall_timeout: Option<Duration>,
    /// Export metrics to xlsx on completion (works with --run, --test, --headless).
    #[arg(long)]
    xlsx: Option<PathBuf>,
    /// Free-form label appended to the window title and stderr summary.
    /// Empty by default — opt in to distinguish stacked runs (e.g.
    /// `--label "$(git rev-parse --short HEAD)"`).
    #[arg(long)]
    label: Option<String>,
    /// Render warehouse map as a PNG image and exit (no GUI).
    #[arg(long)]
    export_png: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Re-run a previously recorded run.
    Repeat {
        /// The .worldview recording to re-run, world document and all.
        recording: PathBuf,
        #[arg(long, default_value = "perf/profiles")]
        output_dir: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 1)]
        threads: usize,
        #[arg(long)]
        allow_incomplete: bool,
    },
    /// Deep statistical comparison of two recordings.
    Compare { old: PathBuf, new: PathBuf },
    /// Compute fetch metrics (FPI, F_cycle, saturation) using A* pathfinding.
    Fpi {
        map: String,
        #[arg(long)]
        pick_time: Option<f64>,
    },
    /// Print map topology info.
    Info {
        map: String,
        #[arg(short, long)]
        verbose: bool,
    },
    /// Diagnose A* pathfinding between two cells.
    Route {
        map: String,
        /// Source cell ID.
        from: u32,
        /// Destination cell ID.
        to: u32,
        /// Robot type: htm, vtm, or rtp (default: htm).
        #[arg(long, default_value = "htm")]
        robot: String,
        /// Treat robot as laden (carrying cargo).
        #[arg(long)]
        laden: bool,
        /// Use HTM kinematic motion model (trapezoidal accel/decel).
        /// Runs find_path_reserved with an empty reservation table.
        #[arg(long)]
        kinematic: bool,
    },
}

fn main() -> eframe::Result {
    let cli = Cli::parse();
    #[cfg(feature = "walmart-demo")]
    let cli = {
        let mut cli = cli;
        if cli.command.is_none() {
            walmart_demo::configure(&mut cli.gui).unwrap_or_else(|error| {
                eprintln!("Walmart demo: {error}");
                std::process::exit(1);
            });
        }
        cli
    };

    match cli.command {
        Some(Command::Repeat { recording, output_dir, name, threads, allow_incomplete }) => {
            bench::run_repeat(&recording, &output_dir, name, threads, allow_incomplete);
            Ok(())
        }
        Some(Command::Compare { old, new }) => {
            compare::run_compare(&old, &new);
            Ok(())
        }
        Some(Command::Fpi { map, pick_time }) => {
            fpi::run_fpi(&map, pick_time);
            Ok(())
        }
        Some(Command::Info { map, verbose }) => {
            info::run_info(&map, verbose);
            Ok(())
        }
        Some(Command::Route { map, from, to, robot, laden, kinematic }) => {
            use desgraph::RobotType;
            let robot_type = match robot.to_lowercase().as_str() {
                "vtm" => RobotType::Vtm,
                "rtp" => RobotType::Rtp,
                _ => RobotType::Htm,
            };
            route::run_route(&map, from, to, robot_type, laden, kinematic);
            Ok(())
        }
        None if cli.gui.export_png.is_some() => {
            let map_path = cli.gui.map.expect("map path required for --export-png mode");
            let loaded = world_cli::load(&map_path.to_string_lossy());
            let (map, cell_sizes_mm, footprint_stated, headings) = (
                loaded.world.map,
                loaded.world.cell_sizes_mm,
                loaded.world.cell_footprint_stated,
                loaded.world.cell_headings_rad,
            );
            let nominal_mm = map.grid_bounds.map(|b| b.cell_size_mm).unwrap_or(850.0);
            let tiles = worldview_floor::cells::tiles(&map, &cell_sizes_mm, &footprint_stated, &headings, nominal_mm);
            let cell_infos = app::grid_data::build_grid_data(&map, &tiles);
            export_png::export_png(&cell_infos, nominal_mm, cli.gui.export_png.as_ref().unwrap())
                .unwrap_or_else(|e| panic!("{}", e));
            Ok(())
        }
        None if cli.gui.dump_render.is_some() => {
            // clap holds both halves to the flag: --recording is required
            // beside it, and the flag takes exactly two values. The arms below
            // say so anyway rather than reaching into the vector blind.
            let (Some(words), Some(recording)) = (cli.gui.dump_render, cli.gui.recording) else {
                eprintln!("--dump-render needs --recording <file>");
                std::process::exit(2);
            };
            let [list, out] = words.as_slice() else {
                eprintln!("--dump-render takes exactly two values: <TICKS_MS> <OUT_DIR>");
                std::process::exit(2);
            };
            let ticks: Vec<u64> = list
                .split(',')
                .map(|word| {
                    word.trim().parse().unwrap_or_else(|e| {
                        eprintln!("--dump-render: '{}' is not a millisecond count: {e}", word.trim());
                        std::process::exit(2);
                    })
                })
                .collect();
            app::render_dump::run(&recording, &ticks, PathBuf::from(out).as_path()).unwrap_or_else(|e| {
                eprintln!("--dump-render: {e}");
                std::process::exit(1);
            });
            Ok(())
        }
        None if cli.gui.headless => {
            let map = cli.gui.map.expect("map path required for --headless mode").to_string_lossy().to_string();
            bench::run_fresh(
                map,
                &cli.gui.output_dir,
                cli.gui.name,
                cli.gui.threads,
                cli.gui.allow_incomplete,
                cli.gui.simtime,
                cli.gui.stations,
                cli.gui.orders,
                cli.gui.events,
                cli.gui.progress,
                cli.gui.settings,
                cli.gui.no_watchdog,
                cli.gui.tripwires,
                cli.gui.wall_timeout,
                cli.gui.fidelity,
                cli.gui.time_slice.map(|seconds| seconds * 1000),
                cli.gui.xlsx.as_deref(),
                cli.gui.trace,
                cli.gui.trace_filter,
                cli.gui.trace_window,
                cli.gui.trace_format,
                cli.gui.trace_out,
                cli.gui.label,
            );
            Ok(())
        }
        None => gui_launch::run_gui(cli.gui),
    }
}
