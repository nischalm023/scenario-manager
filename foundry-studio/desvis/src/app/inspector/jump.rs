//! Jump-to-resource search box in the inspector header: type a fragment,
//! fuzzy/substring-match against station / bot / relay ids + type labels, and
//! click a ranked match to select it. Ids come from `station_metas` /
//! `bot_positions` / `relay_metas`. Returns the pick through the same
//! `InspectorAction.select` path as a cross-link (no mid-frame `self`
//! mutation). Query text is transient egui memory; the app holds no state.

use desgraph::StationType;
use eframe::egui;

use super::super::DesvisApp;
use super::{bot_label, station_label};
use crate::types::{BotType, SelectedResource};

/// Max matches shown in the result list — keeps the header compact.
const MAX_JUMP_RESULTS: usize = 8;

/// One searchable resource: its display label and the selection it yields.
struct JumpCandidate {
    label: String,
    target: SelectedResource,
}

impl DesvisApp {
    /// Render the "Jump to" search box + ranked result list. Returns the picked
    /// resource (a click, or Enter on the top match). Query is transient egui
    /// state keyed by id.
    pub(super) fn draw_jump_to(&self, ui: &mut egui::Ui) -> Option<SelectedResource> {
        let query_id = egui::Id::new("jump_query");
        let mut query = ui.data(|d| d.get_temp::<String>(query_id)).unwrap_or_default();

        let mut submit = false;
        ui.horizontal(|ui| {
            ui.label("Jump to:");
            let response =
                ui.add(egui::TextEdit::singleline(&mut query).hint_text("station/bot/relay id…").desired_width(160.0));
            submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.small_button("✕").clicked() {
                query.clear();
            }
        });

        let picked = if query.trim().is_empty() {
            None
        } else {
            let matches = self.rank_jump_candidates(&query);
            // Enter selects the top match; a click selects that row.
            let mut picked = if submit { matches.first().map(|c| c.target) } else { None };
            for candidate in &matches {
                if ui.selectable_label(false, &candidate.label).clicked() {
                    picked = Some(candidate.target);
                }
            }
            if matches.is_empty() {
                ui.weak("no matches");
            }
            picked
        };

        // Clear the box once a pick is made so the next open starts fresh.
        if picked.is_some() {
            query.clear();
        }
        ui.data_mut(|d| d.insert_temp(query_id, query));
        picked
    }

    /// Build the candidate list and return the best `MAX_JUMP_RESULTS` matches
    /// for `query`, ranked by match quality (see [`fuzzy_score`]).
    fn rank_jump_candidates(&self, query: &str) -> Vec<JumpCandidate> {
        let needle = query.trim().to_lowercase();
        let mut scored: Vec<(i32, JumpCandidate)> = Vec::new();
        // `aliases` are extra searchable words folded into the match haystack
        // but not shown in `label` (e.g. "induction" for putaway stations).
        let mut consider = |label: String, aliases: &str, target: SelectedResource| {
            let haystack =
                if aliases.is_empty() { label.to_lowercase() } else { format!("{label} {aliases}").to_lowercase() };
            if let Some(score) = fuzzy_score(&needle, &haystack) {
                scored.push((score, JumpCandidate { label, target }));
            }
        };
        for meta in &self.station_metas {
            // The label already carries "pick"/"putaway"; putaway stations
            // also answer to "induction" (the domain synonym) via alias.
            // "induction" stays the putaway alias (the pre-box domain
            // synonym); box stations answer to "box".
            let aliases = match meta.station_type {
                StationType::Putaway => "induction",
                StationType::Pick => "",
                StationType::BoxInduction => "box",
                StationType::Extraction => "", // label already says it
                StationType::ContainerPickup => "container pickup",
            };
            consider(
                station_label(meta.station_type, meta.id, meta.process_label),
                aliases,
                SelectedResource::Station(meta.id),
            );
        }
        let mut bots: Vec<(BotType, u32)> = self.bot_positions.iter().map(|b| (b.bot_type, b.bot_id)).collect();
        bots.sort_by_key(|(_, id)| *id);
        bots.dedup();
        for (bot_type, bot_id) in bots {
            consider(bot_label(bot_type, bot_id), "", SelectedResource::Bot { bot_type, bot_id });
        }
        for meta in &self.relay_metas {
            consider(
                format!("Relay {} (cell {})", meta.relay_id, meta.htm_access_cell),
                "",
                SelectedResource::Relay { htm_access_cell: meta.htm_access_cell },
            );
        }
        // Higher score first; stable label tiebreak for determinism.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
        scored.truncate(MAX_JUMP_RESULTS);
        scored.into_iter().map(|(_, candidate)| candidate).collect()
    }
}

/// Case-insensitive match score of `needle` against `haystack` (both already
/// lowercased). `None` = no match. Prefers, in order: exact, prefix,
/// contiguous substring, then subsequence (fuzzy). Earlier matches score
/// higher so "5" ranks "Station 5" above "Station 15".
fn fuzzy_score(needle: &str, haystack: &str) -> Option<i32> {
    if needle == haystack {
        return Some(1000);
    }
    if haystack.starts_with(needle) {
        return Some(500 - haystack.len() as i32);
    }
    if let Some(pos) = haystack.find(needle) {
        return Some(300 - pos as i32);
    }
    // Subsequence fallback: every needle char appears in order.
    let mut chars = haystack.chars();
    let matched = needle.chars().all(|nc| chars.any(|hc| hc == nc));
    if matched { Some(100 - haystack.len() as i32) } else { None }
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

    #[test]
    fn fuzzy_score_ranks_and_filters() {
        // No match when a needle char is absent.
        assert!(fuzzy_score("z", "station 5").is_none());
        // Exact beats prefix beats substring beats subsequence.
        assert!(fuzzy_score("station 5", "station 5").unwrap() > fuzzy_score("station", "station 5").unwrap());
        assert!(fuzzy_score("station", "station 5").unwrap() > fuzzy_score("5", "station 5").unwrap());
        // Earlier substring position ranks higher: "5" in "station 5" (pos 8)
        // vs "5" in "5" handled by exact; compare two substrings.
        assert!(fuzzy_score("a", "abc").unwrap() > fuzzy_score("a", "xabc").unwrap());
        // Subsequence matches (chars in order, non-contiguous) but scores low.
        assert!(fuzzy_score("sn", "station").is_some());
        assert!(fuzzy_score("sn", "station").unwrap() < fuzzy_score("st", "station").unwrap());
    }
}
