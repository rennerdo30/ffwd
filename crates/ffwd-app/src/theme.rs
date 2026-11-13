//! The built-in dark theme. Solid colours only (remote rule: codecs
//! compress flat fills well); depth comes from value steps, not shadows.

pub const BG: u32 = 0x111317;
pub const BG_SIDEBAR: u32 = 0x0c0e11;
pub const BG_BAR: u32 = 0x15181d;
pub const BG_PANEL: u32 = 0x13161a;
pub const BG_INPUT: u32 = 0x1b1f25;
pub const BG_HOVER: u32 = 0x191c22;
pub const BG_SELECT: u32 = 0x1f2836;
pub const BG_SELECT_DIM: u32 = 0x1a1f27;
pub const BORDER: u32 = 0x22262d;
pub const BORDER_SOFT: u32 = 0x1a1d23;

pub const TEXT: u32 = 0xe7eaf0;
pub const TEXT_2: u32 = 0xa3abb9;
pub const TEXT_3: u32 = 0x6b7381;
pub const TEXT_4: u32 = 0x474e5a;

/// ffwd's signature colour (the logo gradient's warm end).
pub const ACCENT: u32 = 0xff6a3d;
pub const ACCENT_TEXT: u32 = 0x1a0d08;

pub const ADD: u32 = 0x3ddc97;
pub const ADD_BG: u32 = 0x112219;
pub const ADD_TEXT: u32 = 0xb6efcf;
pub const DEL: u32 = 0xff6b6b;
pub const DEL_BG: u32 = 0x2a1417;
pub const DEL_TEXT: u32 = 0xffc4c4;
pub const HUNK_BG: u32 = 0x161c2a;
pub const HUNK_TEXT: u32 = 0x7f9fe6;
pub const MOD: u32 = 0xffc24b;
pub const REN: u32 = 0x8ab4ff;

pub const LOCAL_BG: u32 = 0x173126;
pub const LOCAL_FG: u32 = 0x74e8b0;
pub const REMOTE_BG: u32 = 0x182740;
pub const REMOTE_FG: u32 = 0x93bbff;
pub const TAG_BG: u32 = 0x35290f;
pub const TAG_FG: u32 = 0xffcd70;
pub const STASH_BG: u32 = 0x2b1d3d;
pub const STASH_FG: u32 = 0xcfa8ff;

pub const LANES: [u32; 10] =
    [0x4ea8ff, 0xff6a3d, 0x3ddc97, 0xc792ea, 0xffc24b, 0x2ed3e6, 0xff5c8a, 0xa3e635, 0xf78c6c, 0x82aaff];

pub fn lane(i: u16) -> u32 {
    LANES[i as usize % LANES.len()]
}

pub fn avatar(seed: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in seed.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    const AV: [u32; 8] = [0x3b6fd8, 0xc2553a, 0x2f9d6e, 0x8e5bd1, 0xb98a1f, 0x2a9fb0, 0xc04478, 0x6f8f2a];
    AV[(h % AV.len() as u32) as usize]
}
