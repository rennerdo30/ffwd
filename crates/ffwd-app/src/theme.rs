//! ffwd's two themes (FF-57): dark, "graphite and amber", and light,
//! "paper and amber". Every colour is a function that reads the active
//! theme (one relaxed atomic load), so the whole window switches at once;
//! `set_light` picks the theme (Settings → Appearance: System, Dark,
//! Light). The rules below are the dark theme's; the light theme keeps the
//! same roles and contrast steps on a warm white.
//!
//! Dark theme notes:
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

use std::sync::atomic::AtomicBool;
#[cfg(not(test))]
use std::sync::atomic::Ordering;

static LIGHT: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    // Tests paint in parallel threads: each keeps its own theme.
    static TEST_LIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Is the light theme on?
pub fn is_light() -> bool {
    #[cfg(test)]
    {
        let _ = &LIGHT;
        TEST_LIGHT.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        LIGHT.load(Ordering::Relaxed)
    }
}

/// Switches the theme; the caller repaints everything.
pub fn set_light(on: bool) {
    #[cfg(test)]
    TEST_LIGHT.with(|c| c.set(on));
    #[cfg(not(test))]
    LIGHT.store(on, Ordering::Relaxed);
}

macro_rules! colours {
    ($($(#[$m:meta])* $name:ident = $dark:literal, $light:literal;)*) => {
        $(
            $(#[$m])*
            #[inline]
            #[allow(non_snake_case)]
            pub fn $name() -> u32 {
                if is_light() { $light } else { $dark }
            }
        )*
    };
}

colours! {
    BG = 0x1c1d21, 0xfbfaf8;
    BG_SIDEBAR = 0x141518, 0xf1efeb;
    BG_BAR = 0x202125, 0xf5f3ef;
    BG_PANEL = 0x222327, 0xffffff;
    BG_INPUT = 0x292a2f, 0xffffff;
    /// Under the pointer.
    BG_HOVER = 0x2a2b31, 0xebe8e2;
    /// A button or field under the pointer (one step brighter than rows).
    CONTROL_HOVER = 0x323339, 0xe4e0d8;
    /// While the mouse button is down.
    BG_PRESS = 0x303237, 0xdcd8cf;
    BG_SELECT = 0x2f3b50, 0xd5e2f5;
    BG_SELECT_DIM = 0x2b2f38, 0xe7ebf1;
    /// A chosen segment of a segmented control.
    BG_CHOSEN = 0x373940, 0xe3e0d9;
    /// Selected text in a field.
    BG_TEXT_SELECT = 0x3b4c6b, 0xb7cbef;
    BORDER = 0x3a3c43, 0xd3cfc6;
    BORDER_SOFT = 0x2f3136, 0xe4e1db;
    /// Outline of floating panels (palette, settings, agents).
    BORDER_STRONG = 0x4a4d55, 0xbfbab0;
    /// A control's border under the pointer.
    BORDER_HOVER = 0x5c5f67, 0x9f998e;
    /// Menus, the palette and other floating panels: a step above everything.
    BG_FLOAT = 0x2a2b31, 0xffffff;
    /// Whitespace marks in diffs: present, but quieter than comments.
    WS_MARK = 0x55585f, 0xb4afa5;
    TEXT = 0xeceae6, 0x1e1c19;
    TEXT_2 = 0xc0beb8, 0x3b3833;
    TEXT_3 = 0xa8a6a0, 0x58544d;
    TEXT_4 = 0x8a8882, 0x726d64;
    /// ffwd's one accent (the logo's colour).
    ACCENT = 0xe8a33e, 0xd98d22;
    /// Text on an accent fill.
    ACCENT_TEXT = 0x1d1608, 0x1d1608;
    /// Accent-coloured text on dark backgrounds.
    ACCENT_FG = 0xf0c07a, 0x8f5300;
    /// A faint accent tint (the primary button at rest).
    ACCENT_SOFT = 0x33291a, 0xfbedd5;
    ACCENT_SOFT_HOVER = 0x3f321c, 0xf6dfb6;
    ACCENT_BORDER = 0x6a5228, 0xdcab5c;
    ADD = 0x7bc68f, 0x2c8746;
    ADD_BG = 0x1c2f22, 0xe1f3e5;
    ADD_TEXT = 0xd8eadb, 0x17361f;
    ADD_GUTTER = 0x6fa07c, 0x3f9158;
    DEL = 0xe48080, 0xbd3a3a;
    DEL_BG = 0x352225, 0xfbe3e3;
    DEL_TEXT = 0xf2d6d6, 0x481a1a;
    DEL_GUTTER = 0xb47478, 0xbd6464;
    DANGER_BORDER = 0x5c3237, 0xe5a0a0;
    DIFF_ADD_LINE = 0x1a2c20, 0xd3eed9;
    DIFF_DEL_LINE = 0x33201f, 0xf7d6d6;
    HUNK_BG = 0x24272e, 0xedf0f6;
    HUNK_TEXT = 0x98aacb, 0x40598a;
    HUNK_BORDER = 0x4a5c82, 0xa6b8da;
    MOD = 0xd9b25a, 0x9c6f05;
    /// The changed words inside a changed line (word highlights).
    ADD_WORD = 0x356944, 0xa4d9b1;
    DEL_WORD = 0x723c46, 0xeeadad;
    /// Syntax colours: muted, readable on the plain, added and deleted
    /// backgrounds alike.
    SYN_KEYWORD = 0xd2a0dc, 0x86389c;
    SYN_TYPE = 0x8cbde6, 0x1c65a0;
    SYN_STRING = 0xa8d49a, 0x37742a;
    SYN_NUMBER = 0xe9b870, 0x9c5a06;
    SYN_COMMENT = 0x9a978f, 0x77736b;
    REN = 0x92b2e2, 0x2c5ba3;
    LOCAL_BG = 0x243127, 0xddefe1;
    LOCAL_FG = 0xa9d6b3, 0x1f5c36;
    REMOTE_BG = 0x252d3a, 0xdde6f3;
    REMOTE_FG = 0xb0c6e8, 0x284b81;
    TAG_BG = 0x37301f, 0xf5e9cf;
    TAG_FG = 0xe6c986, 0x74540e;
    STASH_BG = 0x2f2838, 0xeae1f4;
    STASH_FG = 0xcfb8e5, 0x5a3b7c;
    /// The conflict resolver's two sides: rows and their edge stripe.
    OURS_BG = 0x1f3046, 0xdbe6f6;
    OURS_FG = 0x92b2e2, 0x2c5ba3;
    THEIRS_BG = 0x2e2540, 0xe9e1f5;
    THEIRS_FG = 0xcfb8e5, 0x5a3b7c;
    /// Windows' own caption colours for close (hover, press).
    CLOSE_HOVER = 0xc42b1c, 0xc42b1c;
    CLOSE_PRESS = 0x9e2a1f, 0x9e2a1f;
}

/// Graph lanes: distinct, but at one muted saturation (darker on light).
pub const LANES: [u32; 10] =
    [0x6fa8dc, 0xe8a33e, 0x83c092, 0xc39bd3, 0xdc8f6c, 0x5fb3ad, 0xd88aa0, 0xadbb6b, 0x95a5e3, 0xcbb072];
const LANES_LIGHT: [u32; 10] =
    [0x2f6aa6, 0xc7790f, 0x2c8746, 0x86459f, 0xbb5528, 0x1b827b, 0xb2436a, 0x6f7e1e, 0x4a5dba, 0x93721a];

pub fn lane(i: u16) -> u32 {
    let l = if is_light() { &LANES_LIGHT } else { &LANES };
    l[i as usize % l.len()]
}

pub fn avatar(seed: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in seed.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    const AV: [u32; 8] = [0x5b7fb8, 0xb06d4f, 0x4f8f6f, 0x8a6db3, 0xa58a45, 0x4d8f9a, 0xa95f7d, 0x6f8a4a];
    AV[(h % AV.len() as u32) as usize]
}
