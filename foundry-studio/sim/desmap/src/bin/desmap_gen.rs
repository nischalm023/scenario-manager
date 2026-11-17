//! CLI tool for generating desmap warehouse JSON from parametric config.
//!
//! Usage:
//!   desmap_gen                          # default 20x20 RTP config
//!   desmap_gen --width 30 --depth 30    # custom dimensions
//!   desmap_gen --relay                  # RELAY system type
//!   desmap_gen -o warehouse.json        # write to file

use desmap::generator::{self, GeneratorConfig, StationLayout};
use desmap::types::SystemType;
use std::fs::File;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut config = GeneratorConfig::default();
    let mut output_path: Option<String> = None;
    let mut pretty = true;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--width" | "-w" => {
                i += 1;
                config.grid_width = args[i].parse().expect("invalid width");
            }
            "--depth" | "-d" => {
                i += 1;
                config.grid_depth = args[i].parse().expect("invalid depth");
            }
            "--cell-size" => {
                i += 1;
                config.cell_size_mm = args[i].parse().expect("invalid cell size");
            }
            "--stations" | "-s" => {
                i += 1;
                config.station_count = args[i].parse().expect("invalid station count");
            }
            "--layout" => {
                i += 1;
                config.station_layout = match args[i].as_str() {
                    "bottom" => StationLayout::SingleBottom,
                    "top" => StationLayout::SingleTop,
                    "left" => StationLayout::SingleLeft,
                    "right" => StationLayout::SingleRight,
                    "opposite_vertical" => StationLayout::OppositeVertical,
                    "opposite_horizontal" => StationLayout::OppositeHorizontal,
                    "four_edge" => StationLayout::FourEdge,
                    other => {
                        eprintln!("unknown layout: {other}");
                        std::process::exit(1);
                    }
                };
            }
            "--highways" => {
                i += 1;
                config.highways_per_edge = args[i].parse().expect("invalid highways");
            }
            "--rows-per-block" => {
                i += 1;
                config.rows_per_block = args[i].parse().expect("invalid rows per block");
            }
            "--rack-columns" => {
                i += 1;
                config.rack_columns = args[i].parse().expect("invalid rack columns");
            }
            "--cross-aisle" => {
                i += 1;
                config.cross_aisle_width = args[i].parse().expect("invalid cross aisle width");
            }
            "--relay" => {
                config.system_type = SystemType::Relay;
            }
            "--rtp" => {
                config.system_type = SystemType::Rtp;
            }
            "--compact" => {
                pretty = false;
            }
            "-o" | "--output" => {
                i += 1;
                output_path = Some(args[i].clone());
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let map = match generator::generate(&config) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("generation error: {e}");
            std::process::exit(1);
        }
    };

    let json = if pretty {
        serde_json::to_string_pretty(&map).expect("serialization failed")
    } else {
        serde_json::to_string(&map).expect("serialization failed")
    };

    match output_path {
        Some(path) => {
            let mut file = File::create(&path).expect("failed to create output file");
            file.write_all(json.as_bytes()).expect("failed to write output");
            eprintln!("wrote {} cells, {} edges to {path}", map.graph.cell_count(), map.graph.edge_count());
        }
        None => {
            println!("{json}");
        }
    }
}

fn print_help() {
    eprintln!(
        "desmap_gen -- parametric warehouse generator

USAGE:
    desmap_gen [OPTIONS]

OPTIONS:
    -w, --width <N>        Grid width (columns), default 20
    -d, --depth <N>        Grid depth (rows), default 20
    --cell-size <MM>       Cell size in mm, default 1000.0
    -s, --stations <N>     Station count, default 4
    --layout <TYPE>        Station layout: bottom, top, left, right,
                           opposite_vertical, opposite_horizontal, four_edge
    --highways <N>         Highways per edge, default 3
    --rows-per-block <N>   Rack rows per block, default 10
    --rack-columns <N>     Rack columns per block, default 2
    --cross-aisle <N>      Cross-aisle width, default 1
    --relay                Use RELAY system type
    --rtp                  Use RTP system type (default)
    --compact              Output compact JSON (no pretty-printing)
    -o, --output <FILE>    Write to file instead of stdout
    -h, --help             Show this help"
    );
}
