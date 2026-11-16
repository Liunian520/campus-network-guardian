//! Semantic light-theme tokens for the native Windows interface.
//!
//! `rgb` accepts display RGB and converts to the COLORREF (0x00BBGGRR)
//! expected by GDI. Keep color decisions here rather than in paint handlers.

pub const fn rgb(red: u32, green: u32, blue: u32) -> u32 {
    red | (green << 8) | (blue << 16)
}

// Native desktop dimensions in DIPs, scaled once by the window's system DPI.
pub const SIDEBAR_WIDTH: i32 = 168;
pub const PAGE_TITLE_SIZE: i32 = 22;
pub const CONNECTION_TITLE_SIZE: i32 = 24;

pub const BACKGROUND: u32 = rgb(0xf5, 0xf5, 0xf5);
pub const SURFACE: u32 = rgb(0xff, 0xff, 0xff);
pub const SIDEBAR: u32 = rgb(0xf0, 0xf0, 0xf0);
pub const SURFACE_BORDER: u32 = rgb(0xe0, 0xe0, 0xe0);
pub const CONTROL_BORDER: u32 = rgb(0x8a, 0x88, 0x86);
pub const WINDOW_BORDER: u32 = rgb(0xd1, 0xd1, 0xd1);

pub const TEXT_PRIMARY: u32 = rgb(0x24, 0x24, 0x24);
pub const TEXT_SECONDARY: u32 = rgb(0x42, 0x42, 0x42);
pub const TEXT_TERTIARY: u32 = rgb(0x61, 0x61, 0x61);
pub const PRIMARY: u32 = rgb(0x0f, 0x6c, 0xbd);
pub const PRIMARY_HOVER: u32 = rgb(0x11, 0x5e, 0xa3);
pub const PRIMARY_PRESSED: u32 = rgb(0x0c, 0x3b, 0x5e);
pub const PRIMARY_SUBTLE: u32 = rgb(0xeb, 0xf3, 0xfc);
pub const TEXT_ON_PRIMARY: u32 = rgb(0xff, 0xff, 0xff);
pub const FOCUS: u32 = rgb(0x0f, 0x6c, 0xbd);
pub const FOCUS_ON_PRIMARY: u32 = rgb(0xff, 0xff, 0xff);

pub const NAV_SELECTED_TEXT: u32 = rgb(0x0f, 0x6c, 0xbd);
pub const NAV_SELECTED_PRESSED_TEXT: u32 = rgb(0x0c, 0x3b, 0x5e);
pub const NAV_HOVER: u32 = rgb(0xe6, 0xe6, 0xe6);
pub const NAV_PRESSED: u32 = rgb(0xdb, 0xdb, 0xdb);
pub const NEUTRAL_HOVER: u32 = rgb(0xeb, 0xeb, 0xeb);
pub const NEUTRAL_PRESSED: u32 = rgb(0xe0, 0xe0, 0xe0);
pub const DISABLED_SURFACE: u32 = rgb(0xe0, 0xe0, 0xe0);
pub const DISABLED_TEXT: u32 = rgb(0x75, 0x75, 0x75);
pub const DANGER_TEXT: u32 = rgb(0xb1, 0x0e, 0x1c);

pub const HERO: u32 = rgb(0x0f, 0x25, 0x3a);
pub const TEXT_ON_HERO: u32 = rgb(0xff, 0xff, 0xff);
pub const TEXT_HERO_SECONDARY: u32 = rgb(0xc7, 0xd6, 0xe6);
