//! Render run settings as a title and startup transcript. Catalog robot
//! models resolve their physical figures separately; the legacy motion
//! settings here are input provenance, not the catalog fleet's physics.
//!
//! `compact_title` is what desvis paints into the OS title bar each frame.
//! `print_summary` is logged to stderr at sim launch (headless bench and
//! the GUI's `start_simulation`) so the user has a copy in the terminal
//! even when stacking many windows.

use std::fmt::Write;

use dessim::SimConfig;
/// One-line title-bar summary of run policy after CLI and UI overlays.
/// Robot motion can differ by model and is not inferred from these settings.
/// Keep it short and scannable; multi-window stacking relies
/// on this being readable as a chip in the OS dock/taskbar.
///
/// `vtm_count` is the resolved VTM fleet size, derived from the map
/// (`WarehouseMap::vtm_count`) — VTM count is not a SimConfig parameter.
///
/// `label` is an optional free-form suffix (e.g. a git commit short
/// hash). Empty by default so the title stays unobtrusive; opt in via
/// `--label` when running matrix experiments.
pub fn compact_title(config: &SimConfig, vtm_count: usize, map_label: &str, label: Option<&str>) -> String {
    let sim_hours = config.sim_duration.as_secs_f64() / 3600.0;
    let mut parts: Vec<String> = Vec::new();
    let head = match label {
        Some(l) if !l.is_empty() => format!("desvis [{l}] · {map_label}"),
        _ => format!("desvis · {map_label}"),
    };
    parts.push(head);
    parts.push(format!("{}H/{}V", config.num_htms, vtm_count));
    if config.num_rtps > 0 {
        parts.push(format!("{}R", config.num_rtps));
    }
    parts.push(format!("qf={:.1}", config.queue_factor));
    parts.push(format!("pick={:.0}s", config.operator.time_per_pick_secs));
    parts.push(format!("emb={:.0}s", config.pick.embargo_secs));
    parts.push(format!("{:.1}h", sim_hours));
    parts.push(format!("{:?}", config.fidelity));
    if config.htm_only {
        parts.push("HTM-only".to_string());
    }
    if let Some(wt) = config.wall_timeout_secs {
        parts.push(format!("wt={:.0}s", wt));
    }
    if let Some(mo) = config.max_orders {
        parts.push(format!("max_orders={}", mo));
    }
    parts.join(" · ")
}

/// Multi-line stderr dump of run settings. Printed at sim
/// launch (`start_simulation` in the GUI, `run_fresh` in headless bench)
/// so the user has a transcript copy of exactly what the run was
/// configured with — critical when stacking many runs.
pub fn print_summary(config: &SimConfig, vtm_count: usize, map_label: &str, label: Option<&str>) {
    let mut buf = String::new();
    let sim_hours = config.sim_duration.as_secs_f64() / 3600.0;
    let _ = writeln!(buf);
    let header = match label {
        Some(l) if !l.is_empty() => format!("=== Run settings — {map_label} [{l}] ==="),
        _ => format!("=== Run settings — {map_label} ==="),
    };
    let _ = writeln!(buf, "{}", header);
    let _ = writeln!(buf, "  fidelity:        {:?}", config.fidelity);
    // Provenance only: "auto" resolves to the map's FPI at solver init
    // (after this summary prints); `desvis fpi <map>` reports that value.
    let putaway_bias = match config.putaway.direct_bias_secs {
        Some(secs) => format!("{secs:.1}s (fixed)"),
        None => "auto (FPI)".to_string(),
    };
    let _ = writeln!(buf, "  putaway bias:    {}", putaway_bias);
    let _ = writeln!(buf, "  sim duration:    {:.2}h ({}s)", sim_hours, config.sim_duration.as_secs());
    let _ = writeln!(
        buf,
        "  fleet:           {} HTMs / {} VTMs (map-derived) / {} RTPs",
        config.num_htms, vtm_count, config.num_rtps
    );
    // Preserve legacy input values without presenting them as the catalog's
    // resolved model. A mixed catalog need not have one fleet-wide model.
    let _ = writeln!(buf, "  Legacy motion settings (catalog model physics resolve separately):");
    let _ = writeln!(buf, "  legacy model:    {:?}", config.motion_model);
    let vtm_cycle = config.vtm.cycle_times();
    let _ = writeln!(
        buf,
        "  grid speed:      {:.1} m/s  (vtm.speed: floor-graph estimates, not VTM travel)",
        config.vtm.speed
    );
    let _ = writeln!(
        buf,
        "  VTM chassis:     {:.2} m/s · {:.2} m/s²  (cycle model, along the aisle)",
        config.vtm.chassis_speed, config.vtm.chassis_accel
    );
    let _ = writeln!(
        buf,
        "  VTM lift:        {:.2} m/s · {:.2} m/s²  (cycle model, fork lift)",
        config.vtm.lift_speed, config.vtm.lift_accel
    );
    let _ = writeln!(
        buf,
        "  VTM alignment:   {:.2}s pick · {:.2}s drop",
        vtm_cycle.pick_alignment_secs, vtm_cycle.drop_alignment_secs
    );
    let _ = writeln!(buf, "  VTM fork:        {:.2}s pick · {:.2}s drop", config.vtm.pick_secs, config.vtm.drop_secs);
    let _ = writeln!(
        buf,
        "  HTM const speed: {:.1} loaded · {:.1} empty m/s  (ConstantSpeed mode)",
        config.htm.loaded_constant_speed, config.htm.empty_constant_speed
    );
    let _ = writeln!(
        buf,
        "  HTM v_max:       {:.1} loaded · {:.1} empty m/s  (HtmKinematic mode)",
        config.htm.loaded_speed, config.htm.empty_speed
    );
    let _ = writeln!(
        buf,
        "  HTM accel:       {:.1} loaded · {:.1} empty m/s²",
        config.htm.loaded_accel, config.htm.empty_accel
    );
    let _ = writeln!(
        buf,
        "  HTM rotate 90°:  {:.2} loaded · {:.2} empty s",
        config.htm.loaded_rotate, config.htm.empty_rotate
    );
    // Describe the legacy reversal fallback for each load state.
    if config.htm.loaded_rotate_180.is_some() || config.htm.empty_rotate_180.is_some() {
        let reversal = |figure: Option<f64>, quarter: f64, load: &str| match figure {
            Some(secs) => format!("{secs:.2}s {load} (one action)"),
            None => format!("{:.2}s {load} (two 90°s)", quarter * 2.0),
        };
        let _ = writeln!(
            buf,
            "  HTM rotate 180°: {} · {}",
            reversal(config.htm.loaded_rotate_180, config.htm.loaded_rotate, "loaded"),
            reversal(config.htm.empty_rotate_180, config.htm.empty_rotate, "empty")
        );
    }
    let _ = writeln!(
        buf,
        "  HTM fork:        {:.1}s pick · {:.1}s deposit",
        config.htm.pickup_secs, config.htm.deposit_secs
    );
    // Resolve only the legacy conveyor fallback shown in this section.
    let _ = writeln!(
        buf,
        "  HTM at conveyor: {:.2}s pick · {:.2}s deposit  (station belt)",
        config.htm.conveyor_pickup_time().as_ms() as f64 / 1000.0,
        config.htm.conveyor_deposit_time().as_ms() as f64 / 1000.0
    );
    let _ = writeln!(buf, "  Run policy:");
    // Dimensionless multipliers since admission caps became derivations, so
    // say so here: the counts they produce print in the "Admission caps" block
    // the solver logs next, once the map has resolved each station's F_max.
    let pinned = match config.order.max_inflight_picks_override {
        Some(n) => format!(" · inventory admission cap override {n}"),
        None => String::new(),
    };
    let _ = writeln!(buf, "  queue factor:    {:.2} (derived pick/inventory admission){}", config.queue_factor, pinned);
    let putaway_source = match config.putaway.htm_wip_factor {
        Some(_) => "set by putaway.htm_wip_factor",
        None => "default, putaway.htm_wip_factor unset",
    };
    let _ = writeln!(
        buf,
        "  htm wip factor:  {:.2} (derived putaway admission; {putaway_source})",
        config.putaway.htm_wip_multiplier()
    );
    let _ = writeln!(buf, "  pick time:       {:.2}s", config.operator.time_per_pick_secs);
    let _ = writeln!(buf, "  pallet swap:     {:.2}s", config.operator.pallet_swap_secs);
    let _ = writeln!(buf, "  embargo:         {:.0}s", config.pick.embargo_secs);
    let _ = writeln!(buf, "  pick seq:        {}", config.pick.sequencing.label());
    let _ = writeln!(buf, "  pick pace:       {}", if config.pick.pace_to_station { "station-JIT" } else { "soonest" });
    let _ = writeln!(buf, "  nav factor:      {}", config.nav_factor);
    let _ = writeln!(buf, "  rng seed:        {}", config.rng_seed);
    let _ = writeln!(buf, "  threads:         {}", config.num_threads);
    let _ = writeln!(buf, "  htm_only:        {}", config.htm_only);
    let _ = writeln!(buf, "  watchdog:        {}", if config.no_watchdog { "disabled" } else { "enabled" });
    let _ = writeln!(buf, "  tripwires:       {}", config.tripwires);
    if let Some(wt) = config.wall_timeout_secs {
        let _ = writeln!(buf, "  wall timeout:    {:.1}s", wt);
    }
    if let Some(mo) = config.max_orders {
        let _ = writeln!(buf, "  max orders:      {}", mo);
    }
    eprint!("{}", buf);
}
