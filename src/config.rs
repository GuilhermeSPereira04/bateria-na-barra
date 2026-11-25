//! Panel colors. Stub: Task 4 adds the config file and the accent/tint choices.

/// The colors the panel is drawn with, resolved from the user's choice.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Colors {
    /// RGB of buttons that are on, bars, the active tab and today.
    pub accent: u32,
    /// RGB and opacity of the layer painted over the glass.
    pub tint: (u32, f32),
}

impl Default for Colors {
    fn default() -> Self {
        Self { accent: 0x9BE3B0, tint: (0x000000, 0.25) }
    }
}

/// Preset accent colors offered under the gear, in order.
pub const ACCENTS: [u32; 8] = [0x9BE3B0, 0x60CDFF, 0xB4A0FF, 0xFF8FB1, 0xFFB86B, 0xF5E07A, 0x7FE0D4, 0xE6E6E6];
