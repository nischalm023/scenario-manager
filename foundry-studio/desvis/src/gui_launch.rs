//! GUI launch: parse CLI args and start the eframe window.

use desgraph::CellKind;
use serde_json::Value;

use dessim::harness::RunOverrides;
use dessim::metrics::worldview::read_worldview;
use dessim::worldmap::WorldFile;

use dessim::{SLICE_WIDTH_MS, ScenarioConfig, ScenarioSourceKind, ScenarioSummary, SimConfig, TraceConfig};

use crate::GuiArgs;
use crate::app::{DesvisApp, LaunchFile, RunPreparation, build_grid_data};
use crate::entity_panel::EntityPanel;
use crate::types;
use crate::types::{MetricKind, Scene};
use worldview_floor::easyloop::Placement;
use worldview_floor::rack as rack_geometry;
use worldview_render::{Floor, FloorPatch, MSAA_SAMPLES, RackVisual, Scene as DrawnScene, Underlay};

pub(crate) fn run_gui(args: GuiArgs) -> eframe::Result {
    // Parse --metrics flag.
    let cli_metrics: Option<Vec<MetricKind>> = args.metrics.as_deref().map(|s| match s {
        "list" => {
            eprintln!("Available metrics (comma-separated cli names, or 'all'):");
            for kind in MetricKind::all() {
                eprintln!("  {:<25} {}", kind.cli_name(), kind.label());
            }
            std::process::exit(0);
        }
        "all" => MetricKind::all().to_vec(),
        _ => s
            .split(',')
            .map(|name| {
                MetricKind::from_cli_name(name.trim()).unwrap_or_else(|| {
                    eprintln!("unknown metric: '{}'. Use --metrics list to see available names.", name.trim());
                    std::process::exit(1);
                })
            })
            .collect(),
    });

    // The GUI's own charts need a width; the spec's is the default it takes.
    let time_slice_ms = args.time_slice.map(|seconds| seconds * 1000).unwrap_or(SLICE_WIDTH_MS);

    // Playback: a finished run, read from the recording it wrote.
    if let Some(ref path) = args.recording {
        // A damaged or half-written file is an answer, not a crash: it comes
        // back as the error eframe reports, with the path in it.
        let recording =
            read_worldview(path).map_err(|e| eframe::Error::AppCreation(format!("{}: {e}", path.display()).into()))?;
        if recording.truncated {
            eprintln!("note: {} has no end block — the run crashed or is still going", path.display());
        }
        let prof = recording.profile;
        let opened = path.clone();
        let grid_w = prof.metadata.grid_width;
        let grid_d = prof.metadata.grid_depth;
        let max_win_w = 1800.0_f32;
        let max_win_h = 1200.0_f32;
        let chrome_w = types::SIDEBAR_WIDTH + 60.0;
        let chrome_h = 40.0 + 250.0;
        let cell_px_w = (max_win_w - chrome_w) / grid_w as f32;
        let cell_px_h = (max_win_h - chrome_h) / grid_d as f32;
        let initial_cell_px = cell_px_w.min(cell_px_h).min(24.0);
        let win_w = grid_w as f32 * initial_cell_px + chrome_w;
        let win_h = (grid_d as f32 * initial_cell_px + chrome_h).max(800.0);

        let options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size([win_w, win_h]),
            renderer: eframe::Renderer::Wgpu,
            depth_buffer: 32,
            multisampling: MSAA_SAMPLES as u16,
            ..Default::default()
        };

        return eframe::run_native(
            "desvis (playback)",
            options,
            Box::new(move |cc| {
                crate::theme::apply(&cc.egui_ctx);
                Ok(Box::new(DesvisApp::from_profile(prof, &opened)?))
            }),
        );
    }

    // Live simulation mode: world-protocol documents and specs only.
    let map_label = match args.map.as_ref() {
        Some(path) => path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string()),
        None => "generated".to_string(),
    };
    #[allow(clippy::type_complexity)]
    let (
        map,
        steel,
        building_floors,
        equipment_strips,
        easyloops,
        cell_sizes_mm,
        cell_footprint_stated,
        cell_headings_rad,
        racking,
        entities,
        doc,
        is_relay,
        vtc,
        mut config,
        mut embedded_scenario,
        robot_catalog,
        world_path,
        launch_file,
    ) = if let Some(ref path) = args.map {
        let json = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
        // World-protocol documents, a site's flagged export and their spec
        // files load through dessim::worldmap rather than the SimSpec/compile
        // path. View-first: every cell arrives with a world position and no
        // grid index, which is exactly the free-node path below.
        let head: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
        let world_file = WorldFile::of(path, &head)
            .unwrap_or_else(|e| panic!("failed to load world document {}: {e}", path.display()));
        let is_world_spec = head.get("world").is_some() && head.get("version").is_none();
        if world_file.is_some() || is_world_spec {
            let (
                m,
                steel,
                sizes,
                stated,
                headings,
                racking,
                building,
                easyloops,
                entities,
                doc,
                scenario,
                cfg,
                fleets,
                from,
            ) = if is_world_spec {
                let loaded = dessim::world_spec::load(path)
                    .unwrap_or_else(|e| panic!("failed to load world spec {}: {e}", path.display()));
                let easyloops = Placement::load(&loaded.world).map_err(|e| eframe::Error::AppCreation(e.into()))?;
                (
                    loaded.world.map,
                    loaded.world.steel,
                    loaded.world.cell_sizes_mm,
                    loaded.world.cell_footprint_stated,
                    loaded.world.cell_headings_rad,
                    loaded.world.racking,
                    loaded.world.building,
                    easyloops,
                    loaded.world.entities,
                    loaded.world.world,
                    loaded.scenario,
                    loaded.settings,
                    loaded.world.robot_catalog,
                    loaded.world_path,
                )
            } else {
                let loaded = dessim::worldmap::load_parsed(path, &head)
                    .unwrap_or_else(|e| panic!("failed to load world document {}: {e}", path.display()));
                // A bare document carries no settings block, so the fleet has
                // to come from the document itself -- the same rule
                // `world_spec::load` applies to a spec that omits num_htms.
                // Without it a live run deploys zero HTMs and cannot dispatch,
                // while the sidebar claims live runs are enabled.
                let easyloops = Placement::load(&loaded).map_err(|e| eframe::Error::AppCreation(e.into()))?;
                let mut cfg = SimConfig::default();
                cfg.num_htms = loaded.map.fleet_config().htms;
                (
                    loaded.map,
                    loaded.steel,
                    loaded.cell_sizes_mm,
                    loaded.cell_footprint_stated,
                    loaded.cell_headings_rad,
                    loaded.racking,
                    loaded.building,
                    easyloops,
                    loaded.entities,
                    loaded.world,
                    ScenarioConfig::default(),
                    cfg,
                    loaded.robot_catalog,
                    path.clone(),
                )
            };
            // A run is prepared from the file desvis was launched with, as
            // headless prepares one.
            let launch_file =
                if is_world_spec { LaunchFile::Spec(path.clone()) } else { LaunchFile::World(path.clone()) };
            // The building's ground, under the cells: Blank (white), so that
            // wherever no cell stands -- a tiling's carve-out, an untiled
            // margin -- the inside of the walls reads as bare ground, never
            // as floor.
            let building_floors: Vec<FloorPatch> =
                building.into_iter().map(|rect_mm| FloorPatch { rect_mm, kind: CellKind::Blank }).collect();
            // Machine envelopes remain available through Option-click inspection.
            // They describe membership and bounds, not painted floor or equipment.
            let equipment_strips = Vec::new();
            let relay = !m.relays.is_empty();
            // The uniform ladder height must match the extruded steel, or
            // VTMs ride inside the boxes instead of on top of them.
            let vtc = steel.iter().map(|s| s.levels).max().unwrap_or(1).max(1);
            (
                m,
                steel,
                building_floors,
                equipment_strips,
                easyloops,
                sizes,
                stated,
                headings,
                Some(racking),
                entities,
                doc,
                relay,
                vtc,
                cfg,
                scenario,
                fleets,
                from,
                launch_file,
            )
        } else {
            let version = head.get("version").and_then(Value::as_u64);
            panic!(
                "{} is not a world-protocol document or spec (version={:?}). The v3 design \
                 format is retired; maps come in through WPS and its adapters.",
                path.display(),
                version,
            );
        }
    } else {
        panic!(
            "desvis needs a world-protocol document or spec file. The v3 parametric \
             generator retired with the v3 compiler."
        );
    };

    // Apply CLI flags to config. These must stay in step with the headless
    // path (bench::run_fresh): the same flag meaning two different things
    // depending on whether the GUI is up is worse than not having it.
    if let Some(d) = args.simtime {
        config.sim_duration = d;
    }
    if let Some(n) = args.orders {
        if let ScenarioConfig::Generated(ref mut g) = embedded_scenario {
            if n < g.num_orders {
                let ratio = n as f64 / g.num_orders as f64;
                let new_picks = (g.total_order_picks as f64 * ratio).round() as usize;
                eprintln!("Limiting to {} of {} orders ({} picks)", n, g.num_orders, new_picks);
                g.num_orders = n;
                g.total_order_picks = new_picks;
            }
        }
        config.max_orders = Some(n);
    }
    if args.stations.is_some() {
        // v3 sliced the design before compiling; a world document states its
        // stations as entities and slicing the compiled map would orphan
        // their queue cells. Rule a subset in the spec instead.
        eprintln!("--stations is not supported for world documents; ignoring");
    }
    // A refused setting stops the launch, as it stops a headless run: a window
    // opened on the spec's own figures would pass for the run that was asked.
    dessim::apply_cli_settings(&mut config, &args.settings, &robot_catalog, &world_path)
        .map_err(|e| eframe::Error::AppCreation(e.to_string().into()))?;
    dessim::apply_scenario_settings(&mut embedded_scenario, &args.settings)
        .map_err(|e| eframe::Error::AppCreation(e.to_string().into()))?;
    if let Some(threads) = args.threads {
        config.num_threads = threads;
    }
    if let Some(f) = &args.fidelity {
        config.fidelity = f.parse().unwrap_or_else(|e| panic!("{}", e));
    }
    // Flags that turn something on: given, they win; absent, the spec's answer
    // stands, which is the rule `dessim::harness` keeps for the headless path.
    config.no_watchdog |= args.no_watchdog;
    config.tripwires |= args.tripwires;
    if let Some(wt) = args.wall_timeout {
        config.wall_timeout_secs = Some(wt.as_secs_f64());
    }
    config.trace = TraceConfig::from_cli(
        args.trace,
        args.trace_filter.clone(),
        args.trace_window.as_deref(),
        &args.trace_format,
        args.trace_out.clone(),
    )
    .unwrap_or_else(|e| panic!("{}", e));

    // The flags whose effect lies outside the configuration `config` settles
    // here, which every run takes from the window.
    let run_preparation = RunPreparation {
        file: launch_file,
        overrides: RunOverrides {
            max_orders: args.orders,
            settings: args.settings.clone(),
            events: args.events,
            ..RunOverrides::default()
        },
        output_dir: args.output_dir.clone(),
    };

    let bounds = map.grid_bounds.expect("parametric map must have grid_bounds");
    let width = bounds.width;
    let depth = bounds.depth;

    let tiles = worldview_floor::cells::tiles(
        &map,
        &cell_sizes_mm,
        &cell_footprint_stated,
        &cell_headings_rad,
        bounds.cell_size_mm,
    );
    let edges = worldview_floor::cells::edges(&map);
    let cell_infos = build_grid_data(&map, &tiles);
    // A world document states every slot outright: real frames, real
    // heights, real tote positions. A slotless map falls back to the
    // uniform vtc ladder.
    let rack_visual = match &racking {
        Some(r) if !r.columns.is_empty() => rack_geometry::from_worldmap(&map, r, &steel),
        _ => RackVisual::uniform(vtc),
    };

    // Scenario summary from embedded config.
    let scenario_summary = match &embedded_scenario {
        ScenarioConfig::Generated(g) => Some(ScenarioSummary {
            source: ScenarioSourceKind::Generated,
            num_skus: g.num_skus,
            num_popular_skus: (g.num_skus as f64 * g.popular_skus_pct / 100.0) as usize,
            total_inventory_units: g.total_inventory_units as u64,
            num_orders: g.num_orders,
            total_picks: g.total_order_picks,
            picks_dropped: 0,
            completable: true,
        }),
        _ => None, // Summary computed after scenario resolution for Inline/Csv.
    };

    let max_win_w = 1800.0_f32;
    let max_win_h = 1200.0_f32;
    let chrome_w = types::SIDEBAR_WIDTH + 60.0;
    let chrome_h = 40.0 + 250.0;
    let cell_px_w = (max_win_w - chrome_w) / width as f32;
    let cell_px_h = (max_win_h - chrome_h) / depth as f32;
    let initial_cell_px = cell_px_w.min(cell_px_h).min(24.0);
    let win_w = width as f32 * initial_cell_px + chrome_w;
    let win_h = (depth as f32 * initial_cell_px + chrome_h).max(800.0);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([win_w, win_h]),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 32,
        multisampling: MSAA_SAMPLES as u16,
        ..Default::default()
    };

    eframe::run_native(
        "desvis",
        options,
        Box::new(move |cc| {
            crate::theme::apply(&cc.egui_ctx);
            let mut app = DesvisApp::new(
                Scene {
                    // The floor and every layer desvis derived to stand on it.
                    drawn: DrawnScene::new(Floor::from_tiles(
                        cell_infos.iter().map(|slot| slot.as_ref().map(|info| info.cell.clone())).collect(),
                        bounds.cell_size_mm,
                    ))
                    .with_edges(edges)
                    .with_rack(rack_visual)
                    .with_underlay(Underlay {
                        steel_mm: steel.iter().map(|bay| bay.rect_mm).collect(),
                        building: building_floors,
                        equipment: equipment_strips,
                    }),
                    cell_infos,
                    easyloops,
                },
                width,
                depth,
                bounds.cell_size_mm,
                Some(map),
                is_relay,
                scenario_summary,
                args.run || args.test,
                args.test,
                Some(run_preparation),
                args.allow_incomplete,
                time_slice_ms,
                cli_metrics,
                config,
                embedded_scenario,
                args.xlsx,
                map_label.clone(),
                args.label.clone(),
            );
            // Set here rather than through the constructor: the roll is the
            // loaded DOCUMENT's, and everything `DesvisApp::new` takes is
            // built from the compiled map.
            app.entity_panel = EntityPanel::from_census(entities, Some(doc));
            Ok(Box::new(app))
        }),
    )
}
