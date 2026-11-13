//! The built-in dark theme, "graphite and amber".
//!
//! Neutral, slightly warm greys carry the interface; one amber accent marks
//! what is active (the current tab, the focused field, HEAD, the primary
//! action) and nothing else. Text sits on four steps whose contrast against
//! the background stays readable (TEXT_3 is about 5:1). Solid colours only
//! (remote rule: codecs compress flat fills well); depth comes from value
//! steps, not shadows or gradients.

pub const BG: u32 = 0x1c1d21;
pub const BG_SIDEBAR: u32 = 0x17181b;
pub const BG_BAR: u32 = 0x202125;
pub const BG_PANEL: u32 = 0x222327;
pub const BG_INPUT: u32 = 0x292a2f;
/// Under the pointer.
pub const BG_HOVER: u32 = 0x26272c;
/// A button or field under the pointer (one step brighter than rows).
pub const CONTROL_HOVER: u32 = 0x323339;
/// While the mouse button is down.
pub const BG_PRESS: u32 = 0x303237;
pub const BG_SELECT: u32 = 0x2c3442;
pub const BG_SELECT_DIM: u32 = 0x272a31;
/// A chosen segment of a segmented control.
pub const BG_CHOSEN: u32 = 0x373940;
/// Selected text in a field.
pub const BG_TEXT_SELECT: u32 = 0x3b4c6b;
pub const BORDER: u32 = 0x2f3136;
pub const BORDER_SOFT: u32 = 0x26272b;
/// Outline of floating panels (palette, settings, agents).
pub const BORDER_STRONG: u32 = 0x3c3e44;
/// A control's border under the pointer.
pub const BORDER_HOVER: u32 = 0x494c53;

pub const TEXT: u32 = 0xeceae6;
pub const TEXT_2: u32 = 0xc0beb8;
pub const TEXT_3: u32 = 0x96948e;
pub const TEXT_4: u32 = 0x706e69;

/// ffwd's one accent (the logo's colour).
pub const ACCENT: u32 = 0xe8a33e;
/// Text on an accent fill.
pub const ACCENT_TEXT: u32 = 0x1d1608;
/// Accent-coloured text on dark backgrounds.
pub const ACCENT_FG: u32 = 0xf0c07a;
/// A faint accent tint (the primary button at rest).
pub const ACCENT_SOFT: u32 = 0x33291a;
pub const ACCENT_SOFT_HOVER: u32 = 0x3f321c;
pub const ACCENT_BORDER: u32 = 0x6a5228;

pub const ADD: u32 = 0x7bc68f;
pub const ADD_BG: u32 = 0x1f2a22;
pub const ADD_TEXT: u32 = 0xd8eadb;
pub const ADD_GUTTER: u32 = 0x4f7d5c;
pub const DEL: u32 = 0xe48080;
pub const DEL_BG: u32 = 0x302124;
pub const DEL_TEXT: u32 = 0xf2d6d6;
pub const DEL_GUTTER: u32 = 0x8c5659;
pub const DANGER_BORDER: u32 = 0x5c3237;
pub const DIFF_ADD_LINE: u32 = 0x1a241d;
pub const DIFF_DEL_LINE: u32 = 0x281c1f;
pub const HUNK_BG: u32 = 0x24272e;
pub const HUNK_TEXT: u32 = 0x98aacb;
pub const HUNK_BORDER: u32 = 0x34405a;
pub const MOD: u32 = 0xd9b25a;
pub const REN: u32 = 0x92b2e2;

pub const LOCAL_BG: u32 = 0x243127;
pub const LOCAL_FG: u32 = 0xa9d6b3;
pub const REMOTE_BG: u32 = 0x252d3a;
pub const REMOTE_FG: u32 = 0xb0c6e8;
pub const TAG_BG: u32 = 0x37301f;
pub const TAG_FG: u32 = 0xe6c986;
pub const STASH_BG: u32 = 0x2f2838;
pub const STASH_FG: u32 = 0xcfb8e5;

/// Windows' own caption colours for close (hover, press).
pub const CLOSE_HOVER: u32 = 0xc42b1c;
pub const CLOSE_PRESS: u32 = 0x9e2a1f;

/// Graph lanes: distinct, but at one muted saturation.
pub const LANES: [u32; 10] =
    [0x6fa8dc, 0xe8a33e, 0x83c092, 0xc39bd3, 0xdc8f6c, 0x5fb3ad, 0xd88aa0, 0xadbb6b, 0x95a5e3, 0xcbb072];

pub fn lane(i: u16) -> u32 {
    LANES[i as usize % LANES.len()]
}

pub fn avatar(seed: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in seed.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    const AV: [u32; 8] = [0x5b7fb8, 0xb06d4f, 0x4f8f6f, 0x8a6db3, 0xa58a45, 0x4d8f9a, 0xa95f7d, 0x6f8a4a];
    AV[(h % AV.len() as u32) as usize]
}
