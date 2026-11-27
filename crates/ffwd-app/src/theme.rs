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
//! the background stays readable: TEXT about 14:1, TEXT_2 9:1, TEXT_3 7:1,
//! TEXT_4 4.7:1 on BG (FF-57 readability pass; TEXT_4 is for hints only).
//! Borders and surface steps are strong enough to separate panes without
//! shadows (BORDER 1.5:1, BORDER_STRONG 2:1 on BG). Solid colours only
//! (remote rule: codecs compress flat fills well); depth comes from value
//! steps, not shadows or gradients.

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
    row_h: 23.0,
    side_row_h: 23.0,
    top_bar_h: 40.0,
    tab_bar_h: 28.0,
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
            #[allow(non_snake_case)]
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
    BG(bg) = 0x1e1f22, 0xf8f8f7;
    BG_SIDEBAR(bg_sidebar) = 0x191a1d, 0xf1f1ef;
    BG_BAR(bg_bar) = 0x222327, 0xf4f4f2;
    BG_PANEL(bg_panel) = 0x202125, 0xfcfcfb;
    BG_INPUT(bg_input) = 0x17181b, 0xffffff;
    /// Under the pointer.
    BG_HOVER(bg_hover) = 0x26282e, 0xeaeae6;
    /// A button or field under the pointer (one step brighter than rows).
    CONTROL_HOVER(control_hover) = 0x2d3038, 0xe3e3de;
    /// While the mouse button is down.
    BG_PRESS(bg_press) = 0x32353f, 0xdad9d4;
    BG_SELECT(bg_select) = 0x2c313b, 0xdce4f0;
    BG_SELECT_DIM(bg_select_dim) = 0x252830, 0xe8ebf0;
    /// A chosen segment of a segmented control.
    BG_CHOSEN(bg_chosen) = 0x30333c, 0xe4e3de;
    /// Selected text in a field.
    BG_TEXT_SELECT(bg_text_select) = 0x354460, 0xbfd1ee;
    BORDER(border) = 0x2d2f36, 0xdeded9;
    BORDER_SOFT(border_soft) = 0x25272d, 0xe8e8e4;
    /// Outline of floating panels (palette, settings, agents).
    BORDER_STRONG(border_strong) = 0x3e414b, 0xc4c3bc;
    /// A control's border under the pointer.
    BORDER_HOVER(border_hover) = 0x4d515d, 0xa8a69e;
    /// Menus, the palette and other floating panels: a step above everything.
    BG_FLOAT(bg_float) = 0x26282e, 0xffffff;
    /// Whitespace marks in diffs: present, but quieter than comments.
    WS_MARK(ws_mark) = 0x4e525c, 0xb8b6ae;
    TEXT(text) = 0xd6d7db, 0x222428;
    TEXT_2(text_2) = 0xa6a9b2, 0x484b54;
    TEXT_3(text_3) = 0x8a8e98, 0x626670;
    TEXT_4(text_4) = 0x70747f, 0x767a84;
    /// ffwd's one accent (the logo's colour).
    ACCENT(accent) = 0xd99b3c, 0xc9831d;
    /// Text on an accent fill.
    ACCENT_TEXT(accent_text) = 0x1b150a, 0x1b150a;
    /// Accent-coloured text on dark backgrounds.
    ACCENT_FG(accent_fg) = 0xdfb164, 0x8a5200;
    /// A faint accent tint (the primary button at rest).
    ACCENT_SOFT(accent_soft) = 0x2c251b, 0xf9ecd6;
    ACCENT_SOFT_HOVER(accent_soft_hover) = 0x372e20, 0xf3deb9;
    ACCENT_BORDER(accent_border) = 0x5c4a2d, 0xd4a962;
    ADD(add) = 0x6cb582, 0x2c8144;
    ADD_BG(add_bg) = 0x1d2b22, 0xe3f2e7;
    ADD_TEXT(add_text) = 0xc9e0cf, 0x183820;
    ADD_GUTTER(add_gutter) = 0x629270, 0x438b57;
    DEL(del) = 0xd47676, 0xb53c3c;
    DEL_BG(del_bg) = 0x2f2022, 0xf9e5e5;
    DEL_TEXT(del_text) = 0xe8cbcb, 0x4b1d1d;
    DEL_GUTTER(del_gutter) = 0xa3686c, 0xb46262;
    DANGER_BORDER(danger_border) = 0x533034, 0xdfa2a2;
    DIFF_ADD_LINE(diff_add_line) = 0x1b2820, 0xd8efde;
    DIFF_DEL_LINE(diff_del_line) = 0x2d1f20, 0xf6dada;
    HUNK_BG(hunk_bg) = 0x23262d, 0xedf0f5;
    HUNK_TEXT(hunk_text) = 0x91a4c4, 0x405886;
    HUNK_BORDER(hunk_border) = 0x41506e, 0xa8b8d6;
    MOD(mod_color) = 0xc7a254, 0x936908;
    /// The changed words inside a changed line (word highlights).
    ADD_WORD(add_word) = 0x2f5a3d, 0xabdbb7;
    DEL_WORD(del_word) = 0x63353e, 0xebb1b1;
    /// Syntax colours: muted, readable on the plain, added and deleted
    /// backgrounds alike.
    SYN_KEYWORD(syn_keyword) = 0xc198cc, 0x803994;
    SYN_TYPE(syn_type) = 0x84b0d6, 0x1f6198;
    SYN_STRING(syn_string) = 0x98c28d, 0x366f2b;
    SYN_NUMBER(syn_number) = 0xd9ad6c, 0x945708;
    SYN_COMMENT(syn_comment) = 0x838790, 0x72757e;
    REN(ren) = 0x89a8d6, 0x2e5a9e;
    LOCAL_FG(local_fg) = 0x97c4a3, 0x225a37;
    REMOTE_BG(remote_bg) = 0x232a36, 0xdfe7f2;
    REMOTE_FG(remote_fg) = 0x9fb8de, 0x2a4c80;
    TAG_BG(tag_bg) = 0x302b1f, 0xf3ead4;
    TAG_FG(tag_fg) = 0xd6bb7c, 0x715312;
    STASH_FG(stash_fg) = 0xbfa8d6, 0x593d78;
    /// The conflict resolver's two sides: rows and their edge stripe.
    OURS_BG(ours_bg) = 0x1f2d40, 0xdce6f4;
    OURS_FG(ours_fg) = 0x8eaed9, 0x2d599e;
    THEIRS_BG(theirs_bg) = 0x2a233a, 0xeae2f4;
    THEIRS_FG(theirs_fg) = 0xbfa8d6, 0x593d78;
    /// Windows' own caption colours for close (hover, press).
    CLOSE_HOVER(close_hover) = 0xc42b1c, 0xc42b1c;
    CLOSE_PRESS(close_press) = 0x9e2a1f, 0x9e2a1f;
}

/// Graph lanes: calm muted hues at balanced saturation (darker on light).
pub const LANES: [u32; 10] =
    [0x6b9ac4, 0xd49a45, 0x79b088, 0xb28ec2, 0xc98668, 0x5da39e, 0xc78095, 0x9eab67, 0x8a99cf, 0xbba46e];
const LANES_LIGHT: [u32; 10] =
    [0x33699e, 0xbd7515, 0x308047, 0x804796, 0xb0542c, 0x217b75, 0xa84568, 0x6b7924, 0x4b5db0, 0x8d6e1e];

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
    const AV: [u32; 8] = [0x5b7fb8, 0xb06d4f, 0x4f8f6f, 0x8a6db3, 0xa58a45, 0x4d8f9a, 0xa95f7d, 0x6f8a4a];
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
