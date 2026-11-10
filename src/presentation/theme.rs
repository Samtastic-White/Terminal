//! Design tokens, colors, paddings, font configurations.

use gpui::*;

/// Catppuccin Mocha palette. The full token set stays available even when
/// only a subset is read.
#[allow(dead_code)]
pub mod colors {
    use gpui::Rgba;

    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
    }

    pub const fn hex(value: u32) -> Rgba {
        let [_, r, g, b] = value.to_be_bytes();
        rgb(r, g, b)
    }

    pub const ROSEWATER: Rgba = hex(0xf5e0dc);
    pub const FLAMINGO: Rgba = hex(0xf2cdcd);
    pub const PINK: Rgba = hex(0xf5c2e7);
    pub const MAUVE: Rgba = hex(0xcba6f7);
    pub const RED: Rgba = hex(0xf38ba8);
    pub const MAROON: Rgba = hex(0xeba0ac);
    pub const PEACH: Rgba = hex(0xfab387);
    pub const YELLOW: Rgba = hex(0xf9e2af);
    pub const GREEN: Rgba = hex(0xa6e3a1);
    pub const TEAL: Rgba = hex(0x94e2d5);
    pub const SKY: Rgba = hex(0x89dceb);
    pub const SAPPHIRE: Rgba = hex(0x74c7ec);
    pub const BLUE: Rgba = hex(0x89b4fa);
    pub const LAVENDER: Rgba = hex(0xb4befe);
    pub const TEXT: Rgba = hex(0xcdd6f4);
    pub const SUBTEXT1: Rgba = hex(0xbac2de);
    pub const SUBTEXT0: Rgba = hex(0xa6adc8);
    pub const OVERLAY2: Rgba = hex(0x9399b2);
    pub const OVERLAY1: Rgba = hex(0x7f849c);
    pub const OVERLAY0: Rgba = hex(0x6c7086);
    pub const SURFACE2: Rgba = hex(0x585b70);
    pub const SURFACE1: Rgba = hex(0x45475a);
    pub const SURFACE0: Rgba = hex(0x313244);
    pub const BASE: Rgba = hex(0x1e1e2e);
    pub const MANTLE: Rgba = hex(0x181825);
    pub const CRUST: Rgba = hex(0x11111b);

    /// ANSI 16-color mapping to Catppuccin Mocha.
    pub const ANSI: [Rgba; 16] = [
        hex(0x1e1e2e), // 0  black
        hex(0xf38ba8), // 1  red
        hex(0xa6e3a1), // 2  green
        hex(0xf9e2af), // 3  yellow
        hex(0x89b4fa), // 4  blue
        hex(0xf5c2e7), // 5  magenta
        hex(0x94e2d5), // 6  cyan
        hex(0xbac2de), // 7  white
        hex(0x585b70), // 8  bright black
        hex(0xf38ba8), // 9  bright red
        hex(0xa6e3a1), // 10 bright green
        hex(0xf9e2af), // 11 bright yellow
        hex(0x89b4fa), // 12 bright blue
        hex(0xf5c2e7), // 13 bright magenta
        hex(0x94e2d5), // 14 bright cyan
        hex(0xcdd6f4), // 15 bright white
    ];
}

#[derive(Clone)]
pub struct TerminalTheme {
    pub background: Rgba,
    pub foreground: Rgba,
    #[allow(dead_code)]
    pub cursor: Rgba,
    #[allow(dead_code)]
    pub selection: Rgba,
    pub font_size: f32,
    pub line_height: f32,
    pub font_family: &'static str,
}

impl Default for TerminalTheme {
    fn default() -> Self {
        Self {
            background: colors::BASE,
            foreground: colors::TEXT,
            cursor: colors::ROSEWATER,
            selection: colors::SURFACE1,
            font_size: 14.0,
            line_height: 1.5,
            font_family: "JetBrains Mono",
        }
    }
}

impl TerminalTheme {
    pub fn resolve_color(&self, color: &crate::domain::output::Color) -> Rgba {
        use crate::domain::output::Color;
        match color {
            Color::Default => self.foreground,
            Color::Indexed(index) => self.palette_256(*index),
            Color::Rgb { r, g, b } => colors::rgb(*r, *g, *b),
        }
    }

    /// Converts a domain background color to a GPUI color. `Default` means the
    /// terminal background, i.e. no per-span paint.
    pub fn resolve_bg(&self, color: &crate::domain::output::Color) -> Option<Rgba> {
        match color {
            crate::domain::output::Color::Default => None,
            other => Some(self.resolve_color(other)),
        }
    }

    /// Full xterm 256-color palette: 16 ANSI entries, the 6×6×6 color cube,
    /// and the 24-step grayscale ramp.
    fn palette_256(&self, index: u8) -> Rgba {
        match index {
            0..=15 => colors::ANSI[index as usize],
            16..=231 => {
                const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
                let n = u32::from(index) - 16;
                colors::rgb(
                    LEVELS[(n / 36) as usize],
                    LEVELS[((n % 36) / 6) as usize],
                    LEVELS[(n % 6) as usize],
                )
            }
            232..=255 => {
                let level = 8 + (u32::from(index) - 232) * 10;
                colors::rgb(level as u8, level as u8, level as u8)
            }
        }
    }
}
