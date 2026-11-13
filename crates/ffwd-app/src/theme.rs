//! The built-in dark theme, "graphite and amber".
//!
//! Neutral, slightly warm greys carry the interface; one amber accent marks
//! what is active (the current tab, the focused field, HEAD, the primary
//! action) and nothing else. Text sits on four steps whose contrast against
//! the background stays readable: TEXT about 14:1, TEXT_2 9:1, TEXT_3 7:1,
//! TEXT_4 4.7:1 on BG (FF-57 readability pass; TEXT_4 is for hints only).
//! Borders and surface steps are strong enough to separate panes without
//! shadows (BORDER 1.5:1, BORDER_STRONG 2:1 on BG). Solid colours only
//! (remote rule: codecs compress flat fills well); depth comes from value
//! steps, not shadows or gradients.

pub const BG: u32 = 0x1c1d21;
pub const BG_SIDEBAR: u32 = 0x141518;
pub const BG_BAR: u32 = 0x202125;
pub const BG_PANEL: u32 = 0x222327;
pub const BG_INPUT: u32 = 0x292a2f;
/// Under the pointer.
pub const BG_HOVER: u32 = 0x2a2b31;
/// A button or field under the pointer (one step brighter than rows).
pub const CONTROL_HOVER: u32 = 0x323339;
/// While the mouse button is down.
pub const BG_PRESS: u32 = 0x303237;
pub const BG_SELECT: u32 = 0x2f3b50;
pub const BG_SELECT_DIM: u32 = 0x2b2f38;
/// A chosen segment of a segmented control.
pub const BG_CHOSEN: u32 = 0x373940;
/// Selected text in a field.
pub const BG_TEXT_SELECT: u32 = 0x3b4c6b;
pub const BORDER: u32 = 0x3a3c43;
pub const BORDER_SOFT: u32 = 0x2f3136;
/// Outline of floating panels (palette, settings, agents).
pub const BORDER_STRONG: u32 = 0x4a4d55;
/// A control's border under the pointer.
pub const BORDER_HOVER: u32 = 0x5c5f67;
/// Menus, the palette and other floating panels: a step above everything.
pub const BG_FLOAT: u32 = 0x2a2b31;
/// Whitespace marks in diffs: present, but quieter than comments.
pub const WS_MARK: u32 = 0x55585f;

pub const TEXT: u32 = 0xeceae6;
pub const TEXT_2: u32 = 0xc0beb8;
pub const TEXT_3: u32 = 0xa8a6a0;
pub const TEXT_4: u32 = 0x8a8882;

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
pub const ADD_BG: u32 = 0x1c2f22;
pub const ADD_TEXT: u32 = 0xd8eadb;
pub const ADD_GUTTER: u32 = 0x6fa07c;
pub const DEL: u32 = 0xe48080;
pub const DEL_BG: u32 = 0x352225;
pub const DEL_TEXT: u32 = 0xf2d6d6;
pub const DEL_GUTTER: u32 = 0xb47478;
pub const DANGER_BORDER: u32 = 0x5c3237;
pub const DIFF_ADD_LINE: u32 = 0x1a2c20;
pub const DIFF_DEL_LINE: u32 = 0x33201f;
pub const HUNK_BG: u32 = 0x24272e;
pub const HUNK_TEXT: u32 = 0x98aacb;
pub const HUNK_BORDER: u32 = 0x4a5c82;
pub const MOD: u32 = 0xd9b25a;
/// The changed words inside a changed line (word highlights).
pub const ADD_WORD: u32 = 0x356944;
pub const DEL_WORD: u32 = 0x723c46;
/// Syntax colours: muted, readable on the plain, added and deleted
/// backgrounds alike.
pub const SYN_KEYWORD: u32 = 0xd2a0dc;
pub const SYN_TYPE: u32 = 0x8cbde6;
pub const SYN_STRING: u32 = 0xa8d49a;
pub const SYN_NUMBER: u32 = 0xe9b870;
pub const SYN_COMMENT: u32 = 0x9a978f;
pub const REN: u32 = 0x92b2e2;

pub const LOCAL_BG: u32 = 0x243127;
pub const LOCAL_FG: u32 = 0xa9d6b3;
pub const REMOTE_BG: u32 = 0x252d3a;
pub const REMOTE_FG: u32 = 0xb0c6e8;
pub const TAG_BG: u32 = 0x37301f;
pub const TAG_FG: u32 = 0xe6c986;
pub const STASH_BG: u32 = 0x2f2838;
pub const STASH_FG: u32 = 0xcfb8e5;
/// The conflict resolver's two sides: rows and their edge stripe.
pub const OURS_BG: u32 = 0x1f3046;
pub const OURS_FG: u32 = 0x92b2e2;
pub const THEIRS_BG: u32 = 0x2e2540;
pub const THEIRS_FG: u32 = 0xcfb8e5;

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
