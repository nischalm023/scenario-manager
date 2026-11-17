//! Hover tooltip for grid cells.

use desgraph::types::RackOrientation;
use eframe::egui;

use crate::types::{BotMarker, BotType, CellInfo, HeatmapData, HtmAccessMarker, ViewMode, VtmAccessMarker};

/// Show a tooltip for the hovered cell.
pub fn show_cell_tooltip(
    ui: &egui::Ui,
    info: &CellInfo,
    heatmap_data: Option<&HeatmapData>,
    view_mode: ViewMode,
    bot_positions: &[BotMarker],
    vtm_access_cells: &[VtmAccessMarker],
    htm_access_cells: &[HtmAccessMarker],
) {
    let cell_id = info.id;
    let tooltip_id = ui.id().with("cell_tooltip");
    let layer_id = egui::LayerId::new(egui::Order::Tooltip, tooltip_id);

    egui::show_tooltip_at_pointer(ui.ctx(), layer_id, tooltip_id, |ui| {
        ui.monospace(format!("CellId: {}", info.id));
        let c = info.center_mm();
        ui.monospace(format!("Center: ({:.0}, {:.0}) mm", c[0], c[1]));
        ui.monospace(format!("CellKind: {:?}", info.kind));
        ui.monospace(format!("ZoneId: {}", info.zone_id));
        ui.monospace(format!(
            "RackOrientation: {}",
            match info.rack_orientation {
                Some(RackOrientation::NorthSouth) => "NorthSouth",
                Some(RackOrientation::EastWest) => "EastWest",
                None => "None",
            }
        ));
        ui.monospace(format!("Outgoing edges: {}", info.outgoing_edges));

        if let Some(ref relay) = info.relay_info {
            ui.monospace(relay);
        }
        if let Some(ref station) = info.station_info {
            ui.monospace(station);
        }

        if let Some(hm) = heatmap_data {
            let vtm = hm.vtm_counts.get(cell_id as usize).copied().unwrap_or(0);
            let htm = hm.htm_counts.get(cell_id as usize).copied().unwrap_or(0);
            if vtm > 0 || htm > 0 {
                ui.monospace(format!("DA50 visits: {}", vtm));
                ui.monospace(format!("D30 visits: {}", htm));
            }
        }

        // State mode: show bot and VTM access info.
        if view_mode == ViewMode::State {
            for bot in bot_positions {
                if bot.cell_id == cell_id {
                    let type_name = match bot.bot_type {
                        BotType::Vtm => "DA50",
                        BotType::Htm => "D30",
                        BotType::Rtp => "RTP",
                    };
                    ui.monospace(format!("Bot: {} #{} (starting position)", type_name, bot.bot_id));
                }
            }
            for access in vtm_access_cells {
                if access.cell_id == cell_id {
                    ui.monospace(format!(
                        "DA50 reporting anchor for relay at CellId({}), block {}, side={:?}",
                        access.relay_position, access.block_id, access.side,
                    ));
                }
            }
            for access in htm_access_cells {
                if access.cell_id == cell_id {
                    ui.monospace(format!("D30 Access for station {}", access.station_id,));
                }
            }
        }
    });
}
