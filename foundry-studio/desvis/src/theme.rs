//! Visual theme: palette tokens, typography, spacing, widget styling.
//!
//! Single light theme. Applied once at app launch via `apply(&Context)`.
//! Light is chosen because the wgpu grid renderer clears to a near-white
//! background — the surrounding chrome matches that so the central viewport
//! doesn't read as a glaring patch in the middle of a dark UI.
//!
//! Public color tokens are exported so panels can reuse them for custom
//! painter rendering (Gantt bars, legend swatches, chart cursors).

use eframe::egui::{
    self, Color32, FontFamily, FontId, Margin, Rounding, Stroke, TextStyle, Vec2, Visuals, epaint::Shadow,
};

// ---------------------------------------------------------------------------
// Palette tokens
// ---------------------------------------------------------------------------

// Surface scale. Inverted from the typical "chrome darker than canvas"
// pattern: canvas (central panel + chart-panel chrome) is the slightly
// gray BG_0, sidebar and chart-well are bright BG_1 white. The eye treats
// the bright surfaces as the active content area and the gray canvas as
// the workspace they sit on.
pub(crate) const BG_0: Color32 = Color32::from_rgb(0xF3, 0xF5, 0xF8); // canvas (panels behind content)
pub const BG_1: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF); // bright surfaces (sidebar, chart well, widget chrome)
pub(crate) const BG_2: Color32 = Color32::from_rgb(0xE2, 0xE7, 0xEF); // striped rows / faint inset
pub(crate) const BG_3: Color32 = Color32::from_rgb(0xD8, 0xDF, 0xE8); // hovered / input

// Borders.
pub(crate) const BORDER: Color32 = Color32::from_rgb(0xD9, 0xDE, 0xE6);
pub(crate) const BORDER_STRONG: Color32 = Color32::from_rgb(0xB8, 0xC0, 0xCC);

// Text scale.
pub const TEXT_0: Color32 = Color32::from_rgb(0x1A, 0x1F, 0x2B); // primary (near-black)
pub const TEXT_1: Color32 = Color32::from_rgb(0x5A, 0x65, 0x73); // secondary
pub const TEXT_2: Color32 = Color32::from_rgb(0x8B, 0x95, 0xA3); // muted / caption

// Accent + semantic.
pub(crate) const ACCENT: Color32 = Color32::from_rgb(0x0E, 0xA5, 0xE9); // sky-500
pub(crate) const ACCENT_DIM: Color32 = Color32::from_rgb(0xBA, 0xE6, 0xFD); // selection fill on light
pub const OK: Color32 = Color32::from_rgb(0x16, 0xA3, 0x4A);
pub(crate) const WARN: Color32 = Color32::from_rgb(0xD9, 0x77, 0x06);
pub const ERR: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);

// Gantt state colors. Calibrated for light backgrounds — Idle is a soft
// gray gap, busy states are saturated enough to read without overwhelming.
pub const STATE_IDLE: Color32 = Color32::from_rgb(0xE7, 0xEC, 0xF2);
pub const STATE_TRAVELING: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);
pub const STATE_LOADING: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
pub const STATE_PROCESSING: Color32 = Color32::from_rgb(0x8B, 0x5C, 0xF6);

// Station-type colors. Pick = blue (throughput-binding); Putaway = amber.
pub const STATION_PICK: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);
pub const STATION_PUTAWAY: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
pub const STATION_BOX: Color32 = Color32::from_rgb(0x8B, 0x5C, 0xF6);
pub const STATION_EXTRACTION: Color32 = Color32::from_rgb(0x16, 0xA3, 0x4A);

// Chart "now" cursor.
pub const CURSOR: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);

// Cell-type legend colors (sidebar). These mirror the GPU shader palette
// for the central viewport — keep in sync with the case-match in
// `worldview-render`'s `shaders/grid.wgsl::cell_color` so the legend reflects
// what's actually painted.
pub const CELL_HIGHWAY: Color32 = Color32::from_rgb(0x87, 0xCE, 0xFA); // grid.wgsl Aisle index 0
pub const CELL_AISLE: Color32 = Color32::from_rgb(0xB4, 0xDC, 0xFF); // grid.wgsl Aisle index 1
pub const CELL_STORAGE: Color32 = Color32::from_rgb(0xED, 0xC9, 0x47);
pub const CELL_RELAY: Color32 = Color32::from_rgb(0x66, 0xC7, 0x78);
pub const CELL_STATION: Color32 = Color32::from_rgb(0xB0, 0x7D, 0xD4);

// Same contract: mirror `bots.wgsl::bot_color`.
pub const BOT_VTM: Color32 = Color32::from_rgb(0x32, 0x64, 0xFF);
pub const BOT_HTM: Color32 = Color32::from_rgb(0xFF, 0x64, 0x32);
pub const BOT_RTP: Color32 = Color32::from_rgb(0x64, 0xC8, 0x64);

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

pub(crate) const WIDGET_ROUNDING: f32 = 4.0;
pub(crate) const GROUP_ROUNDING: f32 = 6.0;

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

/// Embedded font data (TTF, sourced from upstream releases under the SIL
/// OFL 1.1 / Apache 2.0 licenses respectively).
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const JETBRAINS_MONO_REGULAR: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

/// Replace egui's default proportional (Ubuntu-Light) and monospace
/// (Hack) families with Inter and JetBrains Mono. Keep egui's bundled
/// emoji + icon fonts as trailing fallbacks for codepoints Inter doesn't
/// cover (e.g. emoji in user-supplied scenario names).
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert("Inter-Regular".to_owned(), egui::FontData::from_static(INTER_REGULAR).into());
    fonts.font_data.insert("Inter-SemiBold".to_owned(), egui::FontData::from_static(INTER_SEMIBOLD).into());
    fonts.font_data.insert("JetBrainsMono".to_owned(), egui::FontData::from_static(JETBRAINS_MONO_REGULAR).into());

    // Proportional: Inter Regular primary, SemiBold as a sibling for
    // styled-bold text; emoji/icon fonts trail for special characters.
    let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    proportional.insert(0, "Inter-Regular".to_owned());
    proportional.insert(1, "Inter-SemiBold".to_owned());

    // Monospace: JetBrains Mono primary, defaults trail.
    let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
    monospace.insert(0, "JetBrainsMono".to_owned());

    ctx.set_fonts(fonts);
}

// ---------------------------------------------------------------------------
// apply
// ---------------------------------------------------------------------------

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut style = (*ctx.style()).clone();

    // --- Typography ---
    style.text_styles.insert(TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));

    // --- Spacing ---
    let spacing = &mut style.spacing;
    spacing.item_spacing = Vec2::new(8.0, 5.0);
    spacing.window_margin = Margin::same(10.0);
    spacing.menu_margin = Margin::same(6.0);
    // More generous click targets. The previous (10, 4) made buttons /
    // ComboBoxes feel cramped — modern apps run (12-16, 7-10).
    spacing.button_padding = Vec2::new(14.0, 8.0);
    spacing.indent = 16.0;
    spacing.combo_height = 240.0;
    spacing.interact_size = Vec2::new(32.0, 28.0);
    // NOTE: scrollbar style is applied per-ScrollArea via `ui.set_style`
    // (see `app/frame.rs`). Setting `spacing.scroll.*` on the Context-level
    // Style here had no effect on the rendered scrollbar — the painter
    // reads from the immediate parent ui's Style, not the global one.

    // Disable label text selection globally — the sidebar is full of
    // headings/captions/status text where the I-beam cursor and selectable
    // text just feel wrong. Plot/text-edit widgets remain selectable.
    style.interaction.selectable_labels = false;

    // Slightly longer animation easing for egui's built-in animated
    // transitions (collapsing chevrons, animate_bool calls). Default 0.083s
    // feels snappy in a 60 Hz refresh window but reads as instant for the
    // eye. 0.15s is the sweet spot — perceptible without feeling laggy.
    style.animation_time = 0.15;

    // --- Visuals ---
    let mut v = Visuals::light();
    v.override_text_color = Some(TEXT_0);
    // Canvas color for default panels (central + chart-panel chrome).
    // Sidebar overrides its own frame fill to BG_1 (bright) in frame.rs.
    v.panel_fill = BG_0;
    v.window_fill = BG_1;
    v.window_stroke = Stroke::new(1.0, BORDER);
    // Chart "well" is the bright surface, sitting inside the gray canvas.
    v.extreme_bg_color = BG_1;
    v.faint_bg_color = BG_2;
    v.code_bg_color = BG_2;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERR;
    v.hyperlink_color = ACCENT;
    v.window_rounding = Rounding::same(GROUP_ROUNDING);
    v.menu_rounding = Rounding::same(GROUP_ROUNDING);
    // Soft, low-opacity shadows. Light themes look toy-like with strong drop
    // shadows; we just need a subtle hint of elevation.
    v.window_shadow =
        Shadow { offset: Vec2::new(0.0, 4.0), blur: 12.0, spread: 0.0, color: Color32::from_black_alpha(20) };
    v.popup_shadow =
        Shadow { offset: Vec2::new(0.0, 3.0), blur: 8.0, spread: 0.0, color: Color32::from_black_alpha(24) };

    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    let widget_rounding = Rounding::same(WIDGET_ROUNDING);

    v.widgets.noninteractive.bg_fill = BG_1;
    v.widgets.noninteractive.weak_bg_fill = BG_1;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_1);
    v.widgets.noninteractive.rounding = widget_rounding;
    v.widgets.noninteractive.expansion = 0.0;

    // Interactive widgets get the panel fill (BG_1, white). Section frames
    // are BG_2 (subtle gray), so widgets read as "carved out" wells against
    // the section. Outside section frames they sit flush with the panel —
    // the BORDER stroke still defines them.
    v.widgets.inactive.bg_fill = BG_1;
    v.widgets.inactive.weak_bg_fill = BG_1;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_0);
    v.widgets.inactive.rounding = widget_rounding;
    v.widgets.inactive.expansion = 0.0;

    v.widgets.hovered.bg_fill = BG_3;
    v.widgets.hovered.weak_bg_fill = BG_3;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_0);
    v.widgets.hovered.rounding = widget_rounding;
    v.widgets.hovered.expansion = 0.0;

    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT_0);
    v.widgets.active.rounding = widget_rounding;
    v.widgets.active.expansion = 0.0;

    v.widgets.open.bg_fill = BG_3;
    v.widgets.open.weak_bg_fill = BG_3;
    v.widgets.open.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.widgets.open.fg_stroke = Stroke::new(1.0, TEXT_0);
    v.widgets.open.rounding = widget_rounding;
    v.widgets.open.expansion = 0.0;

    v.indent_has_left_vline = false;
    v.striped = true;
    v.collapsing_header_frame = false;
    v.slider_trailing_fill = true;

    style.visuals = v;
    ctx.set_style(style);
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// Floating-card frame for top-level panels (sidebar, bottom chart
/// panel). Outer margin pushes the painted fill inward so the canvas
/// (BG_0) shows around the card; rounded corners + bright BG_1 fill
/// give the panel a raised "Linear / Notion" feel against the gray
/// canvas behind it.
pub fn card_frame() -> egui::Frame {
    egui::Frame {
        inner_margin: Margin::symmetric(6.0, 6.0),
        outer_margin: Margin::same(6.0),
        rounding: Rounding::same(8.0),
        shadow: Shadow::NONE,
        fill: BG_1,
        // 1px subtle gray outline to define the card edge against the
        // canvas — egui renders 1px strokes with antialiasing so they
        // read close to half-pixel at typical scaling.
        stroke: Stroke::new(1.0, BORDER),
    }
}

/// Apply our thin auto-hiding scrollbar style to the immediate parent
/// `ui`. egui's ScrollArea reads `spacing.scroll.*` and the inactive
/// widget visuals from its parent ui at construction time — setting
/// these on the global `Context` style was found to have no effect on
/// what the scroll painter actually drew, so callers wrap their
/// `ScrollArea` calls like:
///
/// ```ignore
/// theme::apply_scrollbar_style(ui);
/// egui::ScrollArea::vertical().show(ui, |ui| { ... });
/// ```
pub fn apply_scrollbar_style(ui: &mut egui::Ui) {
    apply_scrollbar_style_with_right_offset(ui, 2.0);
}

/// Same as `apply_scrollbar_style`, but with a tunable right-offset (in px)
/// pushing the scrollbar further past the content edge. Used by the bottom
/// panel where the chart/Gantt content has more inner-padding and the bar
/// needs to sit closer to the card border.
pub fn apply_scrollbar_style_with_right_offset(ui: &mut egui::Ui, right_offset: f32) {
    let mut s: egui::Style = (**ui.style()).clone();
    s.spacing.scroll.floating = true;
    // `foreground_color = true` → handle uses `widgets.inactive.fg_stroke.color`
    // (TEXT_0, near-black). With low opacity it reads as a soft gray.
    s.spacing.scroll.foreground_color = true;
    s.spacing.scroll.bar_width = 5.0;
    s.spacing.scroll.floating_width = 5.0;
    s.spacing.scroll.floating_allocated_width = 0.0;
    s.spacing.scroll.handle_min_length = 24.0;
    // Negative outer_margin pushes the bar right of the content edge so it
    // sits closer to the card's right border instead of crowding text.
    s.spacing.scroll.bar_outer_margin = -right_offset;
    s.spacing.scroll.dormant_background_opacity = 0.0;
    s.spacing.scroll.active_background_opacity = 0.0;
    s.spacing.scroll.interact_background_opacity = 0.0;
    s.spacing.scroll.dormant_handle_opacity = 0.0;
    s.spacing.scroll.active_handle_opacity = 0.20;
    s.spacing.scroll.interact_handle_opacity = 0.45;
    ui.set_style(s);
}

/// Sidebar variant: same floating card look as `card_frame`, but with a
/// slightly tighter inner_margin on the left so section content sits
/// closer to the card's left edge. The caller is responsible for ensuring
/// the card fills the SidePanel's full available height (e.g. via
/// `ScrollArea::auto_shrink([false, false])`).
pub fn sidebar_card_frame() -> egui::Frame {
    let mut frame = card_frame();
    frame.inner_margin.left = 4.0;
    frame
}

/// Sidebar section frame. Transparent fill — sections are differentiated
/// by heading + spacing alone, no surface color. Inner margin gives
/// consistent breathing room.
pub fn section_frame() -> egui::Frame {
    egui::Frame {
        inner_margin: Margin::symmetric(10.0, 6.0),
        outer_margin: Margin::ZERO,
        rounding: Rounding::ZERO,
        shadow: Shadow::NONE,
        fill: Color32::TRANSPARENT,
        stroke: Stroke::NONE,
    }
}

/// Section heading: small, bold, low-emphasis (TEXT_1) — used inside
/// `section_frame()` as the label above grouped controls.
pub fn heading(text: &str) -> egui::RichText {
    egui::RichText::new(text.to_uppercase()).small().color(TEXT_1).strong()
}

/// Caption (TEXT_2, small) for label hints like "(0 = all cores)".
pub fn caption(text: &str) -> egui::RichText {
    egui::RichText::new(text).small().color(TEXT_2)
}

/// Linear interpolation between two opaque colors in sRGB space. Good
/// enough for UI hover/animation; not for chart heatmaps where you'd
/// want a perceptually-uniform ramp.
fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp_u8 = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(lerp_u8(a.r(), b.r()), lerp_u8(a.g(), b.g()), lerp_u8(a.b(), b.b()))
}

/// Drop-in replacement for `ui.button(text)` that smoothly animates the
/// background/border on hover and active states.
///
/// egui's built-in `Button` swaps `widgets.{state}.bg_fill` directly on
/// every state change — visually a snap. Here we keep the same Response
/// API but interpolate idle → hover → active via `animate_bool_with_time`,
/// which reads from a per-id animation slot in `Context::memory`.
pub fn button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    button_with_min_size(ui, text, egui::Vec2::ZERO)
}

/// Themed combobox header that matches `button`'s animation. Renders a
/// rounded rectangle with the `selected_text` on the left and a triangle
/// chevron on the right that rotates 90° when the popup is open. Clicking
/// the header toggles the popup; click-outside closes it.
///
/// `menu_contents` receives a ui and should add `ui.selectable_value(...)`
/// rows. The popup's frame and size come from egui's `popup_below_widget`.
pub fn combobox<R>(
    ui: &mut egui::Ui,
    id_str: &str,
    selected_text: &str,
    menu_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    combobox_with_width(ui, id_str, selected_text, ui.available_width(), menu_contents)
}

/// `combobox` with an explicit width — useful in right-aligned toolbars
/// where `ui.available_width()` is the whole row and would inflate the
/// dropdown to fill it.
pub fn combobox_with_width<R>(
    ui: &mut egui::Ui,
    id_str: &str,
    selected_text: &str,
    width: f32,
    menu_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(selected_text.to_string(), font, TEXT_0);

    const CHEVRON: f32 = 8.0;
    const CHEVRON_GAP: f32 = 8.0;

    let height = (galley.size().y + padding.y * 2.0).max(ui.spacing().interact_size.y);
    let desired = egui::Vec2::new(width, height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    let ctx = ui.ctx();
    let id = ui.make_persistent_id(id_str);
    let popup_id = id.with("popup");
    let is_open = ctx.memory(|m| m.is_popup_open(popup_id));

    if response.clicked() {
        ctx.memory_mut(|m| m.toggle_popup(popup_id));
    }

    let hover_t = ctx.animate_bool_with_time(id.with("cmb_hover"), response.hovered() || is_open, 0.12);
    let active_t = ctx.animate_bool_with_time(id.with("cmb_active"), response.is_pointer_button_down_on(), 0.06);

    let mut bg = lerp_color(BG_1, BG_3, hover_t);
    bg = lerp_color(bg, BORDER, active_t);
    let stroke_color = lerp_color(BORDER, BORDER_STRONG, hover_t.max(active_t));

    let painter = ui.painter();
    let rounding = Rounding::same(WIDGET_ROUNDING);
    painter.rect_filled(rect, rounding, bg);
    painter.rect_stroke(rect, rounding, Stroke::new(1.0, stroke_color));

    // Selected text, left-aligned.
    let text_pos = egui::pos2(rect.left() + padding.x, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_pos, galley, TEXT_0);

    // Chevron, right-aligned. Rotates from right-pointing (closed) to
    // down-pointing (open). Same shape as `collapsing_section` for visual
    // consistency.
    let openness = ctx.animate_bool_with_time(id.with("cmb_open"), is_open, 0.15);
    let tri_center = egui::pos2(rect.right() - CHEVRON_GAP - CHEVRON / 2.0, rect.center().y);
    let tri_rect = egui::Rect::from_center_size(tri_center, egui::Vec2::splat(CHEVRON));
    let mut points = vec![tri_rect.left_top(), tri_rect.right_top(), tri_rect.center_bottom()];
    let rotation = egui::emath::Rot2::from_angle(egui::lerp(-std::f32::consts::FRAC_PI_2..=0.0, openness));
    for p in &mut points {
        *p = tri_rect.center() + rotation * (*p - tri_rect.center());
    }
    painter.add(egui::Shape::convex_polygon(points, TEXT_1, egui::Stroke::NONE));

    if response.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // `CloseOnClick`: any click inside the popup (i.e. an item selection)
    // dismisses it — standard combobox UX.
    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::PopupCloseBehavior::CloseOnClick,
        |ui: &mut egui::Ui| {
            ui.set_min_width(response.rect.width());
            menu_contents(ui)
        },
    );

    response
}

/// Same as `button` but lays out at least `min_size` (e.g. for a wide
/// "Run Simulation" button that should fill the sidebar row).
pub fn button_with_min_size(ui: &mut egui::Ui, text: &str, min_size: egui::Vec2) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, TEXT_0);

    let desired = egui::Vec2::new(
        (galley.size().x + padding.x * 2.0).max(min_size.x),
        (galley.size().y + padding.y * 2.0).max(min_size.y).max(ui.spacing().interact_size.y),
    );
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    let ctx = ui.ctx();
    let id = response.id;
    let hover_t = ctx.animate_bool_with_time(id.with("btn_hover"), response.hovered(), 0.12);
    let active_t = ctx.animate_bool_with_time(id.with("btn_active"), response.is_pointer_button_down_on(), 0.06);

    // Idle → hover lightens, then active drops further toward a darker
    // pressed shade. Border tracks fill so it stays one step ahead.
    let mut bg = lerp_color(BG_1, BG_3, hover_t);
    bg = lerp_color(bg, BORDER, active_t);
    let stroke_color = lerp_color(BORDER, BORDER_STRONG, hover_t.max(active_t));

    let painter = ui.painter();
    let rounding = Rounding::same(WIDGET_ROUNDING);
    painter.rect_filled(rect, rounding, bg);
    painter.rect_stroke(rect, rounding, Stroke::new(1.0, stroke_color));

    let text_pos = rect.center() - galley.size() / 2.0;
    painter.galley(text_pos, galley, TEXT_0);

    if response.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    response
}

/// Collapsible section with a themed heading and an inline chevron right
/// next to the title. Body is unindented (no left bar). Open state is
/// persisted per `id_str` across frames.
///
/// The chevron is drawn as a triangle shape rather than a unicode glyph
/// because egui's default proportional font renders most arrow codepoints
/// as missing-glyph boxes. The row gets a subtle hover background and a
/// pointing-hand cursor to read as clickable.
pub fn collapsing_section(
    ui: &mut egui::Ui,
    id_str: &str,
    title: &str,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.make_persistent_id(id_str);
    let mut open: bool = ui.ctx().data_mut(|d| *d.get_persisted_mut_or_insert_with::<bool>(id, || default_open));

    let row_height = 20.0;
    let row_size = egui::Vec2::new(ui.available_width(), row_height);
    let (rect, response) = ui.allocate_exact_size(row_size, egui::Sense::click());

    if response.clicked() {
        open = !open;
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // Hover background: animated fade from the panel color to a slightly
    // darker shade. Interpolate the RGB itself (opaque) instead of fading
    // alpha — egui premultiplies alpha when painting, which dims the color
    // and produces the wrong blend over a light background.
    let hover_t = ui.ctx().animate_bool(id.with("hover"), response.hovered());
    if hover_t > 0.0 {
        let bg = lerp_color(BG_1, BG_3, hover_t);
        ui.painter().rect_filled(rect, Rounding::same(WIDGET_ROUNDING), bg);
    }

    // Heading text, vertically centered.
    let font = egui::FontId::new(11.0, egui::FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(title.to_uppercase(), font, TEXT_1);
    let text_pos = egui::pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley.clone(), TEXT_1);

    // Triangle chevron to the right of the text. Rotates with `open`
    // for a clear open/closed signal. Same shape egui's built-in
    // CollapsingHeader uses.
    let openness = ui.ctx().animate_bool(id.with("open_anim"), open);
    let tri_size = 8.0;
    let tri_center = egui::pos2(text_pos.x + galley.size().x + 8.0, rect.center().y);
    let tri_rect = egui::Rect::from_center_size(tri_center, egui::Vec2::splat(tri_size));
    let mut points = vec![tri_rect.left_top(), tri_rect.right_top(), tri_rect.center_bottom()];
    let rotation = egui::emath::Rot2::from_angle(egui::lerp(-std::f32::consts::FRAC_PI_2..=0.0, openness));
    for p in &mut points {
        *p = tri_rect.center() + rotation * (*p - tri_rect.center());
    }
    ui.painter().add(egui::Shape::convex_polygon(points, TEXT_1, egui::Stroke::NONE));

    if open {
        ui.add_space(3.0);
        body(ui);
    }

    ui.ctx().data_mut(|d| d.insert_persisted(id, open));
}
