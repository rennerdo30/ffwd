//! The shared icon set: small vector glyphs drawn with the canvas's
//! strokes and fills, on a unit grid (-1..1) scaled to the requested
//! half-size. Solid colours only, so remote sessions compress them well.
//! Menus, toolbar buttons, bar buttons and ref labels use these.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyph {
    Branch,
    Tag,
    Remote,
    Head,
    Commit,
    Merge,
    Rebase,
    /// The interactive rebase plan: a list with a move arrow.
    Plan,
    CherryPick,
    Revert,
    Reset,
    Checkout,
    Copy,
    Patch,
    Save,
    Edit,
    Trash,
    Folder,
    Terminal,
    Editor,
    Search,
    Up,
    Down,
    Undo,
    Fetch,
    Pull,
    Push,
    Stash,
    Plus,
    Gear,
    Link,
    DiffTool,
    Update,
    Ignore,
    Clock,
    PullRequest,
    Submodule,
    Close,
    Check,
    Text,
    /// A filled colour dot (tab colours).
    Swatch(u32),
}

impl Ui {
    /// Draws `g` centred at (cx, cy), `s` pixels from the centre to the
    /// edge of its box.
    pub(super) fn glyph(&self, cv: &mut Canvas, g: Glyph, cx: f32, cy: f32, s: f32, color: u32) {
        let p = |u: f32, v: f32| (cx + u * s, cy + v * s);
        let w = self.pf(1.5).max(s * 0.2);
        let line = |cv: &mut Canvas, pts: &[(f32, f32)]| {
            let v: Vec<(f32, f32)> = pts.iter().map(|&(u, v)| p(u, v)).collect();
            cv.stroke(&v, w, color);
        };
        let dot = |cv: &mut Canvas, u: f32, v: f32, r: f32| {
            let (x, y) = p(u, v);
            cv.fill_circle(x, y, r * s, color);
        };
        let ring = |cv: &mut Canvas, u: f32, v: f32, r: f32| {
            let (x, y) = p(u, v);
            cv.ring(x, y, r * s, w, color);
        };
        // An arrow from (u0, v0) to (u1, v1) with a head at the end.
        let arrow = |cv: &mut Canvas, u0: f32, v0: f32, u1: f32, v1: f32| {
            line(cv, &[(u0, v0), (u1, v1)]);
            let (du, dv) = (u1 - u0, v1 - v0);
            let len = (du * du + dv * dv).sqrt().max(0.001);
            let (ux, uy) = (du / len, dv / len);
            let h = 0.45;
            line(
                cv,
                &[(u1 - ux * h - uy * h, v1 - uy * h + ux * h), (u1, v1), (u1 - ux * h + uy * h, v1 - uy * h - ux * h)],
            );
        };
        let rect = |cv: &mut Canvas, u0: f32, v0: f32, u1: f32, v1: f32| {
            line(cv, &[(u0, v0), (u1, v0), (u1, v1), (u0, v1), (u0, v0)]);
        };
        match g {
            Glyph::Branch => {
                line(cv, &[(-0.5, -0.9), (-0.5, 0.9)]);
                let (a, b, c, d) = (p(0.55, -0.45), p(0.55, 0.15), p(-0.5, 0.1), p(-0.5, 0.6));
                cv.bezier(a, b, c, d, w, color);
                dot(cv, 0.55, -0.6, 0.3);
                dot(cv, -0.5, 0.9, 0.22);
            }
            Glyph::Tag => {
                line(cv, &[(-0.95, 0.0), (-0.35, -0.7), (0.9, -0.7), (0.9, 0.7), (-0.35, 0.7), (-0.95, 0.0)]);
                dot(cv, -0.3, 0.0, 0.16);
            }
            Glyph::Remote => {
                ring(cv, 0.0, 0.0, 0.85);
                line(cv, &[(-0.85, 0.0), (0.85, 0.0)]);
                let (a, b, c, d) = (p(0.0, -0.85), p(0.55, -0.4), p(0.55, 0.4), p(0.0, 0.85));
                cv.bezier(a, b, c, d, w, color);
                let (a, b, c, d) = (p(0.0, -0.85), p(-0.55, -0.4), p(-0.55, 0.4), p(0.0, 0.85));
                cv.bezier(a, b, c, d, w, color);
            }
            Glyph::Head => {
                ring(cv, 0.0, 0.0, 0.8);
                dot(cv, 0.0, 0.0, 0.38);
            }
            Glyph::Commit => {
                ring(cv, 0.0, 0.0, 0.45);
                line(cv, &[(-1.0, 0.0), (-0.45, 0.0)]);
                line(cv, &[(0.45, 0.0), (1.0, 0.0)]);
            }
            Glyph::Merge => {
                // Two lines joining into one.
                let (a, b, c, d) = (p(-0.6, -0.75), p(-0.6, -0.1), p(0.0, -0.1), p(0.0, 0.3));
                cv.bezier(a, b, c, d, w, color);
                let (a, b, c, d) = (p(0.6, -0.75), p(0.6, -0.1), p(0.0, -0.1), p(0.0, 0.3));
                cv.bezier(a, b, c, d, w, color);
                line(cv, &[(0.0, 0.3), (0.0, 0.9)]);
                dot(cv, -0.6, -0.8, 0.24);
                dot(cv, 0.6, -0.8, 0.24);
            }
            Glyph::Plan => {
                for v in [-0.6, 0.0, 0.6] {
                    line(cv, &[(-0.1, v), (0.9, v)]);
                }
                arrow(cv, -0.65, 0.75, -0.65, -0.75);
            }
            Glyph::Rebase => {
                line(cv, &[(-0.55, -0.8), (-0.55, 0.8)]);
                dot(cv, -0.55, -0.8, 0.22);
                dot(cv, -0.55, 0.8, 0.22);
                arrow(cv, 0.5, 0.8, 0.5, -0.75);
            }
            Glyph::CherryPick => {
                dot(cv, -0.45, 0.55, 0.32);
                dot(cv, 0.4, 0.65, 0.32);
                let (a, b, c, d) = (p(-0.45, 0.3), p(-0.3, -0.3), p(0.0, -0.7), p(0.2, -0.85));
                cv.bezier(a, b, c, d, w, color);
                line(cv, &[(0.4, 0.4), (0.2, -0.85)]);
            }
            Glyph::Revert | Glyph::Undo => {
                let (a, b, c, d) = (p(0.75, 0.75), p(0.95, -0.2), p(0.2, -0.6), p(-0.65, -0.35));
                cv.bezier(a, b, c, d, w, color);
                line(cv, &[(-0.25, -0.8), (-0.7, -0.35), (-0.2, 0.05)]);
            }
            Glyph::Reset => {
                line(cv, &[(-0.85, -0.8), (-0.85, 0.8)]);
                arrow(cv, 0.9, 0.0, -0.45, 0.0);
            }
            Glyph::Checkout => {
                ring(cv, 0.45, 0.0, 0.42);
                arrow(cv, -1.0, 0.0, -0.1, 0.0);
            }
            Glyph::Copy => {
                rect(cv, -0.85, -0.45, 0.35, 0.9);
                line(cv, &[(-0.35, -0.45), (-0.35, -0.9), (0.85, -0.9), (0.85, 0.4), (0.35, 0.4)]);
            }
            Glyph::Patch | Glyph::Text => {
                line(cv, &[(-0.7, -0.9), (0.3, -0.9), (0.7, -0.5), (0.7, 0.9), (-0.7, 0.9), (-0.7, -0.9)]);
                line(cv, &[(-0.35, -0.2), (0.35, -0.2)]);
                line(cv, &[(-0.35, 0.2), (0.35, 0.2)]);
                if g == Glyph::Patch {
                    line(cv, &[(0.0, -0.55), (0.0, 0.15)]);
                } else {
                    line(cv, &[(-0.35, 0.55), (0.1, 0.55)]);
                }
            }
            Glyph::Save => {
                arrow(cv, 0.0, -0.9, 0.0, 0.35);
                line(cv, &[(-0.85, 0.3), (-0.85, 0.85), (0.85, 0.85), (0.85, 0.3)]);
            }
            Glyph::Edit => {
                line(cv, &[(-0.75, 0.75), (0.55, -0.55)]);
                line(cv, &[(0.35, -0.75), (0.75, -0.35)]);
                line(cv, &[(-0.85, 0.85), (-0.75, 0.45), (-0.45, 0.75), (-0.85, 0.85)]);
            }
            Glyph::Trash => {
                line(cv, &[(-0.85, -0.6), (0.85, -0.6)]);
                line(cv, &[(-0.3, -0.6), (-0.3, -0.9), (0.3, -0.9), (0.3, -0.6)]);
                line(cv, &[(-0.6, -0.6), (-0.5, 0.9), (0.5, 0.9), (0.6, -0.6)]);
            }
            Glyph::Folder => {
                line(
                    cv,
                    &[(-0.9, -0.7), (-0.25, -0.7), (0.0, -0.45), (0.9, -0.45), (0.9, 0.75), (-0.9, 0.75), (-0.9, -0.7)],
                );
            }
            Glyph::Terminal => {
                rect(cv, -0.9, -0.75, 0.9, 0.75);
                line(cv, &[(-0.5, -0.3), (-0.15, 0.0), (-0.5, 0.3)]);
                line(cv, &[(0.05, 0.35), (0.5, 0.35)]);
            }
            Glyph::Editor => {
                line(cv, &[(-0.3, -0.6), (-0.85, 0.0), (-0.3, 0.6)]);
                line(cv, &[(0.3, -0.6), (0.85, 0.0), (0.3, 0.6)]);
                line(cv, &[(0.15, -0.8), (-0.15, 0.8)]);
            }
            Glyph::Search => {
                ring(cv, -0.2, -0.2, 0.55);
                line(cv, &[(0.22, 0.22), (0.85, 0.85)]);
            }
            Glyph::Up => line(cv, &[(-0.7, 0.35), (0.0, -0.35), (0.7, 0.35)]),
            Glyph::Down => line(cv, &[(-0.7, -0.35), (0.0, 0.35), (0.7, -0.35)]),
            Glyph::Fetch => {
                arrow(cv, 0.0, -0.9, 0.0, 0.55);
                line(cv, &[(-0.5, 0.9), (0.5, 0.9)]);
            }
            Glyph::Pull => {
                arrow(cv, 0.0, -0.9, 0.0, 0.35);
                line(cv, &[(-0.85, 0.2), (-0.85, 0.85), (0.85, 0.85), (0.85, 0.2)]);
            }
            Glyph::Push => {
                arrow(cv, 0.0, 0.5, 0.0, -0.9);
                line(cv, &[(-0.85, 0.2), (-0.85, 0.85), (0.85, 0.85), (0.85, 0.2)]);
            }
            Glyph::Stash => {
                rect(cv, -0.85, -0.3, 0.85, 0.85);
                line(cv, &[(-0.95, -0.65), (0.95, -0.65)]);
                line(cv, &[(-0.3, 0.2), (0.3, 0.2)]);
            }
            Glyph::Plus => {
                line(cv, &[(-0.75, 0.0), (0.75, 0.0)]);
                line(cv, &[(0.0, -0.75), (0.0, 0.75)]);
            }
            Glyph::Close => {
                line(cv, &[(-0.6, -0.6), (0.6, 0.6)]);
                line(cv, &[(-0.6, 0.6), (0.6, -0.6)]);
            }
            Glyph::Check => line(cv, &[(-0.75, 0.05), (-0.25, 0.55), (0.75, -0.55)]),
            Glyph::Gear => {
                let (ro, ri) = (0.95 * s, 0.6 * s);
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4;
                    let (sn, c) = a.sin_cos();
                    cv.stroke(&[(cx + c * ri, cy + sn * ri), (cx + c * ro, cy + sn * ro)], w * 1.4, color);
                }
                ring(cv, 0.0, 0.0, 0.62);
            }
            Glyph::Link => {
                line(cv, &[(0.1, -0.8), (-0.8, -0.8), (-0.8, 0.8), (0.8, 0.8), (0.8, -0.1)]);
                arrow(cv, -0.1, 0.1, 0.85, -0.85);
            }
            Glyph::DiffTool => {
                rect(cv, -0.9, -0.75, -0.1, 0.75);
                rect(cv, 0.1, -0.75, 0.9, 0.75);
                line(cv, &[(-0.65, -0.2), (-0.35, -0.2)]);
                line(cv, &[(0.35, 0.2), (0.65, 0.2)]);
            }
            Glyph::Update => {
                ring(cv, 0.0, 0.0, 0.9);
                arrow(cv, 0.0, -0.5, 0.0, 0.45);
            }
            Glyph::Ignore => {
                ring(cv, 0.0, 0.0, 0.8);
                line(cv, &[(-0.55, 0.55), (0.55, -0.55)]);
            }
            Glyph::Clock => {
                ring(cv, 0.0, 0.0, 0.85);
                line(cv, &[(0.0, -0.5), (0.0, 0.0), (0.4, 0.3)]);
            }
            Glyph::PullRequest => {
                dot(cv, -0.55, -0.7, 0.25);
                dot(cv, -0.55, 0.75, 0.25);
                line(cv, &[(-0.55, -0.7), (-0.55, 0.75)]);
                dot(cv, 0.55, 0.75, 0.25);
                line(cv, &[(0.55, 0.75), (0.55, -0.4), (0.1, -0.75)]);
            }
            Glyph::Submodule => {
                rect(cv, -0.85, -0.85, 0.85, 0.85);
                dot(cv, 0.0, 0.0, 0.25);
            }
            Glyph::Swatch(c) => cv.fill_circle(cx, cy, 0.6 * s, c),
        }
    }
}

/// The icon a menu action shows (FF-57): chosen from the action, so new
/// menu items get one without extra code.
pub(super) fn for_act(act: &super::menu::MenuAct) -> Option<Glyph> {
    use super::menu::{MenuAct as M, NameAct as N};
    use crate::work::RefOp as R;
    let op = |o: &R| -> Option<Glyph> {
        Some(match o {
            R::Checkout(_) | R::PrCheckout(..) => Glyph::Checkout,
            R::CreateBranch(..) => Glyph::Branch,
            R::Rename(..) | R::StashRename(..) | R::RemoteRename(..) => Glyph::Edit,
            R::DeleteBranch(..)
            | R::DeleteTag(_)
            | R::StashDrop(_)
            | R::RemoteRemove(_)
            | R::DeleteRemoteTag(..)
            | R::DeleteRemoteBranch(..) => Glyph::Trash,
            R::CreateTag(..) => Glyph::Tag,
            R::Merge(..) => Glyph::Merge,
            R::Rebase(_) => Glyph::Rebase,
            R::RebaseInteractive(..) => Glyph::Plan,
            R::CherryPick(_) => Glyph::CherryPick,
            R::Revert(_) => Glyph::Revert,
            R::Reset(..) => Glyph::Reset,
            R::Abort(_) => Glyph::Close,
            R::StashSave(..) => Glyph::Stash,
            R::StashApply(..) => Glyph::Pull,
            R::RemoteAdd(..) => Glyph::Plus,
            R::RemoteSetUrl(..) => Glyph::Link,
            R::RemoteFetch(_) => Glyph::Fetch,
            R::RemoteTest(_) => Glyph::Check,
            R::Sync(o) => match o {
                ffwd_core::ops::Op::Fetch => Glyph::Fetch,
                ffwd_core::ops::Op::Push => Glyph::Push,
                _ => Glyph::Pull,
            },
            R::TakeSideAll(..) => Glyph::Check,
            R::PushTag(..) | R::PushBranch(..) => Glyph::Push,
            R::ApplyPatch(..) => Glyph::Patch,
            R::SubUpdate(_) => Glyph::Fetch,
            R::SubAdd(..) => Glyph::Plus,
            R::SubMove(..) => Glyph::Folder,
            R::SubRelative(_) | R::SubAbsolute(_) => Glyph::Link,
        })
    };
    match act {
        M::Run(o) => op(o),
        M::Ask(a) => a.choices.first().and_then(|c| op(&c.1)),
        M::Name(n) => Some(match n {
            N::CreateBranch { .. } => Glyph::Branch,
            N::CreateTag { .. } => Glyph::Tag,
            N::StashSave => Glyph::Stash,
            N::AddRemoteUrl | N::AddRemoteName { .. } | N::AddSubmoduleUrl | N::AddSubmodulePath { .. } => Glyph::Plus,
            N::RemoteUrl { .. } => Glyph::Link,
            N::MoveSubmodule { .. } => Glyph::Folder,
            N::CloneUrl | N::CloneDest { .. } => Glyph::Fetch,
            N::InitDir => Glyph::Plus,
            N::SavePatch { .. } => Glyph::Save,
            N::ApplyPatchFile => Glyph::Patch,
            _ => Glyph::Edit,
        }),
        M::Copy(_) => Some(Glyph::Copy),
        M::RebaseFrom(_) => Some(Glyph::Plan),
        M::OpenSubmodule(_) => Some(Glyph::Submodule),
        M::CloseTab(_) | M::CloseOtherTabs(_) => Some(Glyph::Close),
        M::OpenFolder(_) | M::ShowFile(_) => Some(Glyph::Folder),
        M::OpenTerminal(_) => Some(Glyph::Terminal),
        M::OpenUrl(_) => Some(Glyph::Link),
        M::CopyPatch(_) => Some(Glyph::Patch),
        M::CreatePr => Some(Glyph::PullRequest),
        M::ShowCommit(_) => Some(Glyph::Commit),
        M::OpenFile(_, true) | M::OpenFileAt(..) => Some(Glyph::Editor),
        M::OpenFile(_, false) => Some(Glyph::Link),
        M::DiffTool(..) | M::MergeTool(_) => Some(Glyph::DiffTool),
        M::TabColor(_, Some(c)) => super::TAB_COLORS.get(*c as usize).map(|t| Glyph::Swatch(t.1)),
        M::TabColor(_, None) => Some(Glyph::Close),
        M::DiffShowWs(_) => Some(Glyph::Text),
        M::CompareMark(_) => Some(Glyph::Check),
        M::Compare(..) => Some(Glyph::DiffTool),
        M::StageToggle(true) => Some(Glyph::Plus),
        M::StageToggle(false) => Some(Glyph::Undo),
        M::Discard => Some(Glyph::Trash),
        M::Ignore => Some(Glyph::Ignore),
        _ => None,
    }
}
