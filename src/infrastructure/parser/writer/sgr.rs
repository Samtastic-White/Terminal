//! SGR dispatch: style attributes and 16/256/24-bit color parsing.

use vte::Params;

use super::Core;
use crate::domain::output::{Color, TextStyle};

impl Core {
    /// SGR dispatch with full `38;5;n` / `38;2;r;g;b` (and background twins)
    /// parameter traversal. Incomplete color sequences are consumed without
    /// altering state.
    pub(super) fn sgr(&mut self, params: &Params) {
        let flat: Vec<u16> = params.iter().flatten().copied().collect();
        if flat.is_empty() {
            self.style = TextStyle::default();
            return;
        }
        let mut i = 0;
        while i < flat.len() {
            let consumed = self.apply_sgr(&flat[i..]);
            if consumed == 0 {
                break;
            }
            i += consumed;
        }
    }

    /// Applies one SGR parameter, returning how many flat pieces it consumed
    /// (color forms eat their arguments).
    fn apply_sgr(&mut self, rest: &[u16]) -> usize {
        match rest[0] {
            0 => {
                self.style = TextStyle::default();
                1
            }
            1 => {
                self.style.bold = true;
                1
            }
            3 => {
                self.style.italic = true;
                1
            }
            4 => {
                self.style.underline = true;
                1
            }
            22 => {
                self.style.bold = false;
                1
            }
            23 => {
                self.style.italic = false;
                1
            }
            24 => {
                self.style.underline = false;
                1
            }
            38 | 48 => {
                let which = if rest[0] == 38 { "fg" } else { "bg" };
                match rest.get(1) {
                    Some(&5) => match rest.get(2) {
                        Some(&idx) => {
                            self.style.set_color(which, Color::Indexed(idx as u8));
                            3
                        }
                        // Incomplete color form: consume the remainder,
                        // state unchanged.
                        None => rest.len(),
                    },
                    Some(&2) => match (rest.get(2), rest.get(3), rest.get(4)) {
                        (Some(&r), Some(&g), Some(&b)) => {
                            self.style.set_color(
                                which,
                                Color::Rgb {
                                    r: r as u8,
                                    g: g as u8,
                                    b: b as u8,
                                },
                            );
                            5
                        }
                        // Incomplete RGB form: consume the remainder.
                        _ => rest.len(),
                    },
                    // Malformed (missing or wrong mode): consume the
                    // remainder, state unchanged.
                    _ => rest.len(),
                }
            }
            30..=37 => {
                self.style.fg = Color::Indexed((rest[0] - 30) as u8);
                1
            }
            90..=97 => {
                self.style.fg = Color::Indexed((rest[0] - 90 + 8) as u8);
                1
            }
            39 => {
                self.style.fg = Color::Default;
                1
            }
            40..=47 => {
                self.style.bg = Color::Indexed((rest[0] - 40) as u8);
                1
            }
            100..=107 => {
                self.style.bg = Color::Indexed((rest[0] - 100 + 8) as u8);
                1
            }
            49 => {
                self.style.bg = Color::Default;
                1
            }
            // Outside the subset (blink, reverse, fonts, underline styles):
            // ignored.
            _ => 1,
        }
    }
}

impl TextStyle {
    fn set_color(&mut self, which: &str, color: Color) {
        if which == "fg" {
            self.fg = color;
        } else {
            self.bg = color;
        }
    }
}
