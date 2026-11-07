//! Dark tty7-inspired palette with distinct inbox and reading surfaces.
//! System UI text and monospace code use separate, explicit typography scales.
use gpui::*;
use gpui_component::{Theme, ThemeMode};

// The surrounding canvas creates a visible gutter between the content panels.
pub const PAGE: u32 = 0x141518;
pub const SIDEBAR: u32 = 0x1c1e22;
pub const QUEUE: u32 = 0x202328;
pub const READING: u32 = 0x292c32;
pub const TOOLBAR: u32 = 0x272a30;
pub const FIELD: u32 = 0x30343b;
pub const HOVER: u32 = 0x30363f;
pub const SELECTED: u32 = 0x34445e;
pub const SELECTED_HOVER: u32 = 0x3b475b;
pub const LINE: u32 = 0x3c414a;
pub const PANEL_LINE: u32 = 0x535965;
pub const INK: u32 = 0xeef0f4;
pub const HEADING: u32 = 0xf5f6f8;
pub const MUTED: u32 = 0xbbc2ce;
pub const SUBTLE: u32 = 0xb0b5bf;
pub const SUCCESS: u32 = 0x6bcc91;
pub const ERROR: u32 = 0xf08080;
pub const WARNING: u32 = 0xefb85d;
pub const INFO: u32 = 0x8ab4f8;
pub const CLAUDE: u32 = 0xe68e6b;
pub const SUCCESS_BG: u32 = 0x20382b;
pub const ERROR_BG: u32 = 0x42282d;
pub const UI_FONT: &str = ".AppleSystemUIFont";
pub const CODE_FONT: &str = "Menlo";
pub const UI_SIZE: f32 = 14.;
pub const CODE_SIZE: f32 = 12.;
pub const SIDEBAR_WIDTH: f32 = 200.;
pub const TITLEBAR_HEIGHT: f32 = 52.;
pub const CONTEXT_WIDTH: f32 = 300.;
pub const RAIL_WIDTH: f32 = 52.;
// Rounded "cartouches", from the largest surface to the smallest chip.
pub const RADIUS_SHEET: f32 = 10.;
pub const RADIUS_CARD: f32 = 6.;
pub const RADIUS_CONTROL: f32 = 6.;
pub const RADIUS_CHIP: f32 = 3.;

/// A shrinkable, clipped container for [`crate::ellipsis::ellipsis`] text. GPUI 0.2.2's
/// own `truncate()` and `line_clamp(1)` paint layouts from stale measurements.
pub trait OneLine: Styled + Sized {
    fn one_line(self) -> Self {
        self.min_w_0().overflow_hidden().flex()
    }
}
impl<T: Styled> OneLine for T {}

pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = UI_FONT.into();
    theme.font_size = px(UI_SIZE);
    theme.mono_font_family = CODE_FONT.into();
    theme.mono_font_size = px(CODE_SIZE);
    theme.radius = px(RADIUS_CONTROL);
    theme.radius_lg = px(RADIUS_SHEET);
    theme.shadow = false;
    theme.background = rgb(READING).into();
    theme.foreground = rgb(INK).into();
    theme.muted = rgb(FIELD).into();
    theme.muted_foreground = rgb(MUTED).into();
    theme.accent = rgb(SELECTED).into();
    theme.accent_foreground = rgb(HEADING).into();
    theme.primary = rgb(0xaac9fa).into();
    theme.primary_hover = rgb(0xc1d8fc).into();
    theme.primary_active = rgb(0x95b9ef).into();
    theme.primary_foreground = rgb(PAGE).into();
    theme.secondary = rgb(FIELD).into();
    theme.secondary_hover = rgb(HOVER).into();
    theme.secondary_active = rgb(SELECTED).into();
    theme.secondary_foreground = rgb(INK).into();
    theme.border = rgb(LINE).into();
    theme.input = rgb(PANEL_LINE).into();
    theme.ring = rgb(INFO).into();
    theme.caret = rgb(HEADING).into();
    theme.selection = rgb(SELECTED_HOVER).into();
    theme.sidebar = rgb(SIDEBAR).into();
    theme.sidebar_foreground = rgb(INK).into();
    theme.sidebar_border = rgb(PANEL_LINE).into();
    theme.sidebar_accent = rgb(SELECTED).into();
    theme.sidebar_accent_foreground = rgb(HEADING).into();
    theme.popover = rgb(FIELD).into();
    theme.popover_foreground = rgb(INK).into();
    theme.title_bar = rgb(PAGE).into();
    theme.title_bar_border = rgb(PANEL_LINE).into();
    theme.success = rgb(SUCCESS).into();
    theme.success_hover = rgb(0x86d9a6).into();
    theme.success_active = rgb(0x50b879).into();
    theme.success_foreground = rgb(PAGE).into();
    theme.danger = rgb(ERROR).into();
    theme.warning = rgb(WARNING).into();
    theme.link = rgb(INFO).into();
    theme.link_hover = rgb(HEADING).into();
    theme.scrollbar = transparent_black();
    theme.scrollbar_thumb = rgb(0x626b79).into();
    theme.scrollbar_thumb_hover = rgb(0x8793a5).into();
}
