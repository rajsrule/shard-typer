use eframe::egui::{Color32, FontFamily, FontId, Vec2};

// One palette and sizing system for surfaces, fields, and interaction states.
pub const INK: Color32 = Color32::from_rgb(248, 250, 252);
pub const MUTED: Color32 = Color32::from_rgb(210, 217, 225);
pub const DISABLED: Color32 = Color32::from_rgb(148, 159, 170);
pub const ICE: Color32 = Color32::from_rgb(219, 237, 245);
pub const ERROR: Color32 = Color32::from_rgb(255, 187, 180);
pub const SURFACE: [u8; 3] = [12, 17, 23];
pub const PADDING: Vec2 = Vec2::new(20., 16.);
pub const GROUP_WIDTH: f32 = 400.;
pub const CORNER: f32 = 24.;
pub const ICON_HIT: f32 = 32.;
pub const ICON_GAP: f32 = 4.;
pub const ICON_STROKE: f32 = 1.5;
pub const CURVE_STROKE: f32 = 2.6;
pub const CURVE_HALO: f32 = 8.;
pub const HOVER_SECONDS: f32 = 0.10;
pub const FIELD_HEIGHT: f32 = 28.;
pub const FIELD_WIDTH: f32 = 80.;
pub const FIELD_RADIUS: u8 = 9;
pub const BORDER: Color32 = Color32::from_rgba_premultiplied(44, 44, 44, 44);
pub const DIVIDER: Color32 = Color32::from_rgba_premultiplied(22, 22, 22, 22);
pub const FIELD: Color32 = Color32::from_rgba_premultiplied(9, 9, 9, 9);
pub const HOVER: u8 = 16;
pub const PRESSED: u8 = 28;
pub const SELECTED: u8 = 25;
pub const SETTINGS_RADIUS: u8 = 16;
pub const SETTINGS_TINT: u8 = 70;
pub const SETTINGS_SELECTED: u8 = 52;
pub const SETTINGS_TRANSITION: f32 = 0.14;

pub fn heading_font() -> FontId {
    FontId::new(14., FontFamily::Name("shard-heading".into()))
}
pub fn settings_title_font() -> FontId {
    FontId::new(18., FontFamily::Name("shard-heading".into()))
}
