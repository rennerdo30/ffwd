//! ffwd's two themes (FF-57): dark, "graphite and amber", and light,
//! "paper and amber", plus live-tunable design tokens (`Palette` and `Metrics`).
//!
//! Colour getters are deliberately `inline(never)` in debug builds so tweaking
//! a colour only recompiles `theme.rs` instead of every UI module (sub-second
//! iteration), and live `--tokens <file.toml>` overrides update the active
//! atomic tables without recompiling at all.
//!
//! Dark theme notes:
//! Neutral, slightly warm greys carry the interface; one amber accent marks
//! what is active (the current tab, the focused field, HEAD, the primary
//! action) and nothing else. Text sits on four steps whose contrast against
//! the background stays readable without glare: TEXT ~11:1, TEXT_2 ~6.6:1,
//! TEXT_3 ~4.8:1, TEXT_4 ~3.3:1 on BG (calm matte readability; TEXT_4 is for
//! hints only). Subtle matte surface steps and whisper hairlines separate
//! panes without shadows. Solid colours only (remote rule: codecs compress
//! flat fills well); depth comes from value steps, not shadows or gradients.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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

/// Layout and typography metrics that can be tuned live via `--tokens`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub type_scale: f32,
    pub row_h: f32,
    pub side_row_h: f32,
    pub top_bar_h: f32,
    pub tab_bar_h: f32,
    pub status_bar_h: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
}

pub const DEFAULT_METRICS: Metrics = Metrics {
    type_scale: 1.04,
    row_h: 24.0,
    side_row_h: 24.0,
    top_bar_h: 40.0,
    tab_bar_h: 30.0,
    status_bar_h: 22.0,
    radius_sm: 4.0,
    radius_md: 6.0,
    radius_lg: 8.0,
};

const METRIC_KEYS: [&str; 9] = [
    "type_scale",
    "row_h",
    "side_row_h",
    "top_bar_h",
    "tab_bar_h",
    "status_bar_h",
    "radius_sm",
    "radius_md",
    "radius_lg",
];

static ACTIVE_METRICS: [AtomicU32; 9] = [
    AtomicU32::new(DEFAULT_METRICS.type_scale.to_bits()),
    AtomicU32::new(DEFAULT_METRICS.row_h.to_bits()),
    AtomicU32::new(DEFAULT_METRICS.side_row_h.to_bits()),
    AtomicU32::new(DEFAULT_METRICS.top_bar_h.to_bits()),
    AtomicU32::new(DEFAULT_METRICS.tab_bar_h.to_bits()),
    AtomicU32::new(DEFAULT_METRICS.status_bar_h.to_bits()),
    AtomicU32::new(DEFAULT_METRICS.radius_sm.to_bits()),
    AtomicU32::new(DEFAULT_METRICS.radius_md.to_bits()),
    AtomicU32::new(DEFAULT_METRICS.radius_lg.to_bits()),
];

#[cfg_attr(debug_assertions, inline(never))]
pub fn metrics() -> Metrics {
    let f = |i: usize| f32::from_bits(ACTIVE_METRICS[i].load(Ordering::Relaxed));
    Metrics {
        type_scale: f(0),
        row_h: f(1),
        side_row_h: f(2),
        top_bar_h: f(3),
        tab_bar_h: f(4),
        status_bar_h: f(5),
        radius_sm: f(6),
        radius_md: f(7),
        radius_lg: f(8),
    }
}

macro_rules! define_palette {
    ($($(#[$m:meta])* $name:ident ($field:ident) = $dark:literal, $light:literal;)*) => {
        /// Every colour role of one theme (`0x00RRGGBB`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Palette {
            $(pub $field: u32,)*
        }

        const TOKEN_COUNT: usize = 0 $(+ { let _ = stringify!($name); 1 })*;

        const DEFAULT_DARK: [u32; TOKEN_COUNT] = [$($dark,)*];
        const DEFAULT_LIGHT: [u32; TOKEN_COUNT] = [$($light,)*];
        const TOKEN_NAMES: [&str; TOKEN_COUNT] = [$(stringify!($field),)*];

        static ACTIVE_DARK: [AtomicU32; TOKEN_COUNT] = [$(AtomicU32::new($dark),)*];
        static ACTIVE_LIGHT: [AtomicU32; TOKEN_COUNT] = [$(AtomicU32::new($light),)*];

        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(usize)]
        enum Tok {
            $($name,)*
        }

        $(
            $(#[$m])*
            #[cfg_attr(debug_assertions, inline(never))]
            #[allow(non_snake_case, dead_code)]
            pub fn $name() -> u32 {
                let idx = Tok::$name as usize;
                if is_light() {
                    ACTIVE_LIGHT[idx].load(Ordering::Relaxed)
                } else {
                    ACTIVE_DARK[idx].load(Ordering::Relaxed)
                }
            }
        )*

        /// Snapshot of every colour in the currently active theme.
        #[cfg_attr(debug_assertions, inline(never))]
        pub fn pal() -> Palette {
            let table = if is_light() { &ACTIVE_LIGHT } else { &ACTIVE_DARK };
            Palette {
                $($field: table[Tok::$name as usize].load(Ordering::Relaxed),)*
            }
        }
    };
}

define_palette! {
    BG(bg) = 0x1f2023, 0xf8f8f6;
    BG_SIDEBAR(bg_sidebar) = 0x1a1b1e, 0xf1f0ec;
    BG_BAR(bg_bar) = 0x242529, 0xf4f3ef;
    BG_PANEL(bg_panel) = 0x222326, 0xfcfbfa;
    BG_INPUT(bg_input) = 0x1b1c1f, 0xffffff;
    /// Under the pointer.
    BG_HOVER(bg_hover) = 0x27282d, 0xebeae5;
    /// A button or field under the pointer (one step brighter than rows).
    CONTROL_HOVER(control_hover) = 0x2f3137, 0xe4e3dd;
    /// While the mouse button is down.
    BG_PRESS(bg_press) = 0x363840, 0xdcdad3;
    BG_SELECT(bg_select) = 0x2d3038, 0xe2e6ee;
    BG_SELECT_DIM(bg_select_dim) = 0x26282e, 0xebeae6;
    /// A chosen segment of a segmented control.
    BG_CHOSEN(bg_chosen) = 0x32343b, 0xe5e4de;
    /// Selected text in a field.
    BG_TEXT_SELECT(bg_text_select) = 0x334056, 0xc8d6eb;
    BORDER(border) = 0x2a2c31, 0xe1e0db;
    BORDER_SOFT(border_soft) = 0x242529, 0xeae9e5;
    /// Outline of floating panels (palette, settings, agents).
    BORDER_STRONG(border_strong) = 0x373940, 0xcac8c0;
    /// A control's border under the pointer.
    BORDER_HOVER(border_hover) = 0x484b54, 0xa8a59c;
    /// Menus, the palette and other floating panels: a step above everything.
    BG_FLOAT(bg_float) = 0x28292d, 0xffffff;
    /// Whitespace marks in diffs: present, but quieter than comments.
    WS_MARK(ws_mark) = 0x4a4d55, 0xbab8b0;
    TEXT(text) = 0xd5d4d0, 0x232427;
    TEXT_2(text_2) = 0xa4a5a8, 0x4b4d52;
    TEXT_3(text_3) = 0x898b90, 0x676970;
    TEXT_4(text_4) = 0x6d7077, 0x767880;
    /// ffwd's one accent (the logo's colour).
    ACCENT(accent) = 0xc9923e, 0xbd7d1f;
    /// Text on an accent fill.
    ACCENT_TEXT(accent_text) = 0x1b150a, 0x1b150a;
    /// Accent-coloured text on dark backgrounds.
    ACCENT_FG(accent_fg) = 0xd6aa66, 0x87530f;
    /// A faint accent tint (the primary button at rest).
    ACCENT_SOFT(accent_soft) = 0x2a241b, 0xf6ebd8;
    ACCENT_SOFT_HOVER(accent_soft_hover) = 0x332b20, 0xf0dec0;
    ACCENT_BORDER(accent_border) = 0x4d402b, 0xd5b37c;
    ADD(add) = 0x6aa67b, 0x2e7d45;
    ADD_BG(add_bg) = 0x1d2821, 0xe9f3ec;
    ADD_TEXT(add_text) = 0xc6dccb, 0x1a3822;
    ADD_GUTTER(add_gutter) = 0x5f8b6c, 0x478759;
    DEL(del) = 0xc77474, 0xad4040;
    DEL_BG(del_bg) = 0x2b2022, 0xf7eaea;
    DEL_TEXT(del_text) = 0xe3c7c7, 0x4a2020;
    DEL_GUTTER(del_gutter) = 0x996569, 0xac6565;
    DANGER_BORDER(danger_border) = 0x4a2f33, 0xdda8a8;
    DIFF_ADD_LINE(diff_add_line) = 0x1c251f, 0xe1efe5;
    DIFF_DEL_LINE(diff_del_line) = 0x281e20, 0xf3e1e1;
    HUNK_BG(hunk_bg) = 0x23252a, 0xedf0f5;
    HUNK_TEXT(hunk_text) = 0x8898b2, 0x435a82;
    HUNK_BORDER(hunk_border) = 0x384254, 0xb3c0d8;
    MOD(mod_color) = 0xbfa05c, 0x8c6816;
    /// The changed words inside a changed line (word highlights).
    ADD_WORD(add_word) = 0x284733, 0xc3e4cc;
    DEL_WORD(del_word) = 0x4e2d33, 0xecc4c4;
    /// Syntax colours: muted, readable on the plain, added and deleted
    /// backgrounds alike.
    SYN_KEYWORD(syn_keyword) = 0xb896c2, 0x7a428a;
    SYN_TYPE(syn_type) = 0x82a7c7, 0x296290;
    SYN_STRING(syn_string) = 0x92b58b, 0x3a6c30;
    SYN_NUMBER(syn_number) = 0xcca46a, 0x8c5714;
    SYN_COMMENT(syn_comment) = 0x7a7e86, 0x73767e;
    REN(ren) = 0x829ec4, 0x355c96;
    LOCAL_FG(local_fg) = 0x8cb397, 0x265c3a;
    REMOTE_BG(remote_bg) = 0x242830, 0xe4eaf3;
    REMOTE_FG(remote_fg) = 0x93a8c7, 0x305080;
    TAG_BG(tag_bg) = 0x2b2820, 0xf4ecdc;
    TAG_FG(tag_fg) = 0xc2ae7c, 0x735618;
    STASH_FG(stash_fg) = 0xa99bbd, 0x5c4478;
    /// The conflict resolver's two sides: rows and their edge stripe.
    OURS_BG(ours_bg) = 0x1f2a38, 0xe3ebf5;
    OURS_FG(ours_fg) = 0x84a0c4, 0x345b96;
    THEIRS_BG(theirs_bg) = 0x282334, 0xede7f5;
    THEIRS_FG(theirs_fg) = 0xa897be, 0x5c4478;
    /// Windows' own caption colours for close (hover, press).
    CLOSE_HOVER(close_hover) = 0xc42b1c, 0xc42b1c;
    CLOSE_PRESS(close_press) = 0x9e2a1f, 0x9e2a1f;
}

/// Graph lanes: calm muted hues at balanced saturation (darker on light).
pub const LANES: [u32; 10] =
    [0x6c92b4, 0xc2934e, 0x76a283, 0xa288b0, 0xb8846c, 0x629995, 0xb67e8f, 0x949e6b, 0x8491bc, 0xae9c71];
const LANES_LIGHT: [u32; 10] =
    [0x3b6a96, 0xb07524, 0x367b4b, 0x7a4f8c, 0xa35a38, 0x2c7872, 0x9c4c69, 0x697530, 0x5161a3, 0x876c2a];

#[cfg_attr(debug_assertions, inline(never))]
pub fn lane(i: u16) -> u32 {
    let l = if is_light() { &LANES_LIGHT } else { &LANES };
    l[i as usize % l.len()]
}

pub fn avatar(seed: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in seed.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    const AV: [u32; 8] = [0x526278, 0x7a5c50, 0x4d6b5c, 0x685d7a, 0x786b4a, 0x4d6b70, 0x785663, 0x5d6c4d];
    AV[(h % AV.len() as u32) as usize]
}

/// Restores every colour and metric token to its compiled-in default.
pub fn reset_tokens() {
    for i in 0..TOKEN_COUNT {
        ACTIVE_DARK[i].store(DEFAULT_DARK[i], Ordering::Relaxed);
        ACTIVE_LIGHT[i].store(DEFAULT_LIGHT[i], Ordering::Relaxed);
    }
    let defs = [
        DEFAULT_METRICS.type_scale,
        DEFAULT_METRICS.row_h,
        DEFAULT_METRICS.side_row_h,
        DEFAULT_METRICS.top_bar_h,
        DEFAULT_METRICS.tab_bar_h,
        DEFAULT_METRICS.status_bar_h,
        DEFAULT_METRICS.radius_sm,
        DEFAULT_METRICS.radius_md,
        DEFAULT_METRICS.radius_lg,
    ];
    for (i, v) in defs.iter().enumerate() {
        ACTIVE_METRICS[i].store(v.to_bits(), Ordering::Relaxed);
    }
}

/// Parses a `#rrggbb` or `0xrrggbb` colour literal.
fn parse_hex_color(raw: &str) -> Option<u32> {
    let s = raw.trim().trim_matches('"').trim_matches('\'');
    let hex = s.strip_prefix('#').or_else(|| s.strip_prefix("0x")).or_else(|| s.strip_prefix("0X"))?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// Applies design-token overrides from a TOML string (`[dark]`, `[light]`,
/// `[metrics]`). Returns how many tokens changed, or a parse error naming the
/// offending line.
pub fn apply_tokens_toml(text: &str) -> Result<usize, String> {
    let mut section = String::new();
    let mut applied = 0usize;
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        // Strip `#` comments only outside quotes (so `"#1c1d21"` stays intact).
        let mut in_quotes = false;
        let mut end = raw.len();
        for (i, c) in raw.char_indices() {
            if c == '"' {
                in_quotes = !in_quotes;
            } else if c == '#' && !in_quotes {
                end = i;
                break;
            }
        }
        let line = raw[..end].trim();
        if line.is_empty() {
            continue;
        }
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = sec.trim().to_ascii_lowercase();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(format!("tokens line {line_no}: expected key = value"));
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        match section.as_str() {
            "dark" | "light" => {
                let table = if section == "dark" { &ACTIVE_DARK } else { &ACTIVE_LIGHT };
                let Some(pos) = TOKEN_NAMES.iter().position(|&n| n == key || (key == "mod" && n == "mod_color")) else {
                    return Err(format!("tokens line {line_no}: unknown colour token `{key}`"));
                };
                let Some(c) = parse_hex_color(val) else {
                    return Err(format!("tokens line {line_no}: expected #rrggbb or 0xrrggbb for `{key}`"));
                };
                table[pos].store(c, Ordering::Relaxed);
                applied += 1;
            }
            "metrics" => {
                let Some(pos) = METRIC_KEYS.iter().position(|&n| n == key) else {
                    return Err(format!("tokens line {line_no}: unknown metric `{key}`"));
                };
                let v: f32 = val
                    .trim_matches('"')
                    .parse()
                    .map_err(|_| format!("tokens line {line_no}: expected number for `{key}`"))?;
                if !v.is_finite() || v <= 0.0 {
                    return Err(format!("tokens line {line_no}: `{key}` must be positive"));
                }
                let clamped = match pos {
                    0 => v.clamp(0.75, 1.6),
                    1 | 2 => v.clamp(16.0, 48.0),
                    3..=5 => v.clamp(18.0, 72.0),
                    _ => v.clamp(0.0, 24.0),
                };
                ACTIVE_METRICS[pos].store(clamped.to_bits(), Ordering::Relaxed);
                applied += 1;
            }
            "" => return Err(format!("tokens line {line_no}: expected [dark], [light] or [metrics] section first")),
            other => return Err(format!("tokens line {line_no}: unknown section `[{other}]`")),
        }
    }
    Ok(applied)
}

/// Reads a tokens TOML file from disk, resets to defaults, and applies it.
pub fn load_tokens_file(path: &Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    reset_tokens();
    apply_tokens_toml(&text)
}

/// Formats all active colour and metric tokens as a valid TOML document.
pub fn dump_tokens_toml() -> String {
    let mut out = String::new();
    out.push_str("# ffwd design tokens (pass with --tokens <file.toml> or reload over --serve)\n\n[metrics]\n");
    for (i, key) in METRIC_KEYS.iter().enumerate() {
        let v = f32::from_bits(ACTIVE_METRICS[i].load(Ordering::Relaxed));
        out.push_str(&format!("{key} = {v}\n"));
    }
    out.push_str("\n[dark]\n");
    for (i, name) in TOKEN_NAMES.iter().enumerate() {
        let c = ACTIVE_DARK[i].load(Ordering::Relaxed);
        out.push_str(&format!("{name} = \"#{c:06x}\"\n"));
    }
    out.push_str("\n[light]\n");
    for (i, name) in TOKEN_NAMES.iter().enumerate() {
        let c = ACTIVE_LIGHT[i].load(Ordering::Relaxed);
        out.push_str(&format!("{name} = \"#{c:06x}\"\n"));
    }
    out
}

/// Relative luminance (WCAG 2.1) of an sRGB `0x00RRGGBB` colour.
pub fn luminance(rgb: u32) -> f64 {
    fn chan(v: u32) -> f64 {
        let s = (v & 0xff) as f64 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * chan(rgb >> 16) + 0.7152 * chan(rgb >> 8) + 0.0722 * chan(rgb)
}

/// WCAG 2.1 contrast ratio between `a` and `b` (1.0 to 21.0).
pub fn contrast_ratio(a: u32, b: u32) -> f64 {
    let la = luminance(a);
    let lb = luminance(b);
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_roundtrip_and_contrast() {
        let dumped = dump_tokens_toml();
        assert!(dumped.contains("[dark]"));
        assert!(dumped.contains("[light]"));
        assert!(dumped.contains("[metrics]"));
        // Verify parsing our own dump succeeds with zero errors (without mutating globals permanently).
        assert!(apply_tokens_toml(&dumped).is_ok());
        assert!(apply_tokens_toml("[dark]\nno_such_key = \"#ffffff\"\n").is_err());
        assert!(apply_tokens_toml("[dark]\nbg = \"not-a-color\"\n").is_err());
        // Contrast math sanity check: white on black = 21:1.
        assert!((contrast_ratio(0xffffff, 0x000000) - 21.0).abs() < 0.01);
        assert!(contrast_ratio(DEFAULT_DARK[Tok::TEXT as usize], DEFAULT_DARK[Tok::BG as usize]) > 10.0);
        assert!(contrast_ratio(DEFAULT_LIGHT[Tok::TEXT as usize], DEFAULT_LIGHT[Tok::BG as usize]) > 10.0);
    }
}
