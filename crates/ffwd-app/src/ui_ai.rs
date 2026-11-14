//! AI commit messages (FF-50) in Local Changes. The AI draft button builds
//! the request on a worker (the staged diff without secret-looking files,
//! capped, plus recent subjects for style) and shows it in place of the
//! diff: exactly what will be sent, and where. Only Send sends it; the
//! reply streams in, can be stopped, and is a suggestion the user takes
//! with "Use this message". AI is off until an endpoint is set.

use super::*;
use crate::text_area::TextArea;
use changes_view::CFocus;
use ffwd_ai::{Endpoint, Message};
use std::sync::atomic::{AtomicBool, Ordering};

/// The most staged diff sent.
const MAX_DIFF: usize = 24 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Building,
    Preview,
    Streaming,
    Done,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Head,
    Note,
    Add,
    Del,
    Mono,
    Subject,
    Reply,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiHit {
    Send,
    Stop,
    Use,
    Again,
    Close,
}

pub struct AiPane {
    pub stage: Stage,
    pub endpoint: Endpoint,
    pub messages: Vec<Message>,
    /// Files left out (secret-looking paths).
    pub excluded: Vec<String>,
    pub truncated: bool,
    /// The reply so far.
    pub reply: String,
    top: usize,
    pub seq: u64,
    cancel: Option<Arc<AtomicBool>>,
    /// The wrapped text and what it was made from.
    view: Vec<(Kind, String)>,
    view_key: (i32, u8, usize, usize),
}

/// What the worker prepared: messages, left-out files, cut or not.
pub type Prepared = Result<(Vec<Message>, Vec<String>, bool), String>;

/// The request for a commit message from the staged changes.
pub fn prepare(git: &Git) -> Prepared {
    let names = git.output(&["diff", "--cached", "--name-only", "-z"]).map_err(|e| e.to_string())?;
    let files: Vec<String> =
        names.split(|&b| b == 0).filter(|n| !n.is_empty()).map(|n| String::from_utf8_lossy(n).into_owned()).collect();
    if files.is_empty() {
        return Err("Nothing is staged: stage the changes the message should describe first.".into());
    }
    let excluded: Vec<String> = files.iter().filter(|f| ffwd_ai::is_secret_path(f)).cloned().collect();
    let mut args: Vec<String> =
        ["diff", "--cached", "--no-color", "--no-ext-diff", "-U3", "--", "."].iter().map(|s| s.to_string()).collect();
    args.extend(excluded.iter().map(|f| format!(":(exclude,literal){f}")));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let diff = git.output_limited(&refs, MAX_DIFF).map_err(|e| e.to_string())?;
    let truncated = diff.len() >= MAX_DIFF;
    let mut diff = String::from_utf8_lossy(&diff).into_owned();
    if truncated {
        // Cut at a line end and say so.
        if let Some(p) = diff.rfind('\n') {
            diff.truncate(p + 1);
        }
        diff.push_str("[… the rest of the diff is not included]\n");
    }
    let recent = git.output_limited(&["log", "-8", "--no-merges", "--format=%s"], 8 * 1024).unwrap_or_default();
    let recent = String::from_utf8_lossy(&recent).into_owned();
    let mut user = String::new();
    if !recent.trim().is_empty() {
        user.push_str("Recent commit subjects in this repository (match their style):\n");
        user.push_str(&recent);
        user.push('\n');
    }
    if !excluded.is_empty() {
        user.push_str(&format!("Also changed, content not shown: {}\n\n", excluded.join(", ")));
    }
    user.push_str("Staged changes:\n");
    user.push_str(&diff);
    let system = "You write Git commit messages. Reply with the message only, no quotes or code fences: \
a subject line of at most 72 characters in the imperative mood, then a blank line and a short body \
that says why, only when it adds something the subject does not.";
    Ok((
        vec![Message { role: "system", content: system.into() }, Message { role: "user", content: user }],
        excluded,
        truncated,
    ))
}

/// Splits a reply into subject and body (drops code fences and quotes).
pub fn split_message(reply: &str) -> (String, String) {
    let text: Vec<&str> = reply.lines().filter(|l| !l.trim_start().starts_with("```")).collect();
    let mut it = text.iter().map(|l| l.trim_end()).skip_while(|l| l.trim().is_empty());
    let subject = it.next().unwrap_or("").trim().trim_matches('"').trim_matches('`').to_string();
    let subject = subject.strip_prefix("Subject:").map_or(subject.clone(), |s| s.trim().to_string());
    let body: Vec<&str> = it.collect();
    let body = body.join("\n").trim().to_string();
    (subject, body)
}

impl Ui {
    /// The endpoint from the repository's settings, then the user's.
    pub(super) fn ai_endpoint(&self) -> Option<Endpoint> {
        let repo = self.current_repo_settings();
        let get = |k: &str| {
            repo.and_then(|r| r.get(k)).or_else(|| self.settings.get(k)).map(str::trim).unwrap_or("").to_string()
        };
        let base = get("ai.endpoint");
        if base.is_empty() {
            return None;
        }
        let key_env = get("ai.key_env");
        let key = (!key_env.is_empty()).then(|| std::env::var(&key_env).ok()).flatten().filter(|k| !k.is_empty());
        Some(Endpoint { base, model: get("ai.model"), key })
    }

    /// AI draft: build the request and show it (nothing is sent yet).
    pub(super) fn ai_draft(&mut self) {
        let Some(endpoint) = self.ai_endpoint() else {
            self.set_notice(
                "AI is off: set an AI endpoint (and model) in Settings → General. Local servers such as Ollama work."
                    .into(),
            );
            return self.open_settings();
        };
        if !self.r.changes_view {
            self.enter_changes();
        }
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let seq = self.r.changes.ai.as_ref().map_or(1, |a| a.seq + 1);
        self.r.changes.ai = Some(AiPane {
            stage: Stage::Building,
            endpoint,
            messages: Vec::new(),
            excluded: Vec::new(),
            truncated: false,
            reply: String::new(),
            top: 0,
            seq,
            cancel: None,
            view: Vec::new(),
            view_key: (0, 0, 0, 0),
        });
        let sink = (self.sinks)(t.id);
        crate::work::spawn("ai-prepare", &sink, move |sink| sink(Msg::AiPrepared { seq, result: prepare(&g) }));
        self.ai_dirty();
    }

    /// Snapshot mode: the preview against a dummy local endpoint.
    pub fn snapshot_ai_preview(&mut self) {
        self.settings.set_string("ai.endpoint", "http://localhost:11434/v1");
        self.settings.set_string("ai.model", "qwen2.5-coder:14b");
        self.enter_changes();
        self.ai_draft();
    }

    fn ai_font(&self, k: Kind) -> (FontId, f32) {
        match k {
            Kind::Head | Kind::Subject => (self.f.semi, self.pf(13.0)),
            Kind::Note | Kind::Reply => (self.f.ui, self.pf(13.0)),
            Kind::Add | Kind::Del | Kind::Mono => (self.f.mono, self.pf(11.5)),
        }
    }

    fn ai_dirty(&mut self) {
        let r = self.changes_layout().diff;
        self.damage.rects.push(r);
    }

    pub(super) fn on_ai_prepared(&mut self, seq: u64, r: Prepared) {
        let Some(a) = self.r.changes.ai.as_mut().filter(|a| a.seq == seq) else { return };
        match r {
            Ok((m, ex, cut)) => {
                a.messages = m;
                a.excluded = ex;
                a.truncated = cut;
                a.stage = Stage::Preview;
            }
            Err(e) => a.stage = Stage::Failed(e),
        }
        self.ai_dirty();
    }

    fn ai_send(&mut self) {
        let Some(a) = self.r.changes.ai.as_mut() else { return };
        if a.stage != Stage::Preview {
            return;
        }
        let Some(t) = self.tabs.get(self.cur_tab) else { return };
        let cancel = Arc::new(AtomicBool::new(false));
        a.cancel = Some(cancel.clone());
        a.stage = Stage::Streaming;
        a.reply.clear();
        a.top = 0;
        let (ep, msgs, seq) = (a.endpoint.clone(), a.messages.clone(), a.seq);
        let sink = (self.sinks)(t.id);
        crate::work::spawn("ai-chat", &sink, move |sink| {
            let s2 = sink.clone();
            let r = ffwd_ai::chat_stream(&ep, &msgs, &cancel, &mut |t| s2(Msg::AiToken { seq, text: t.to_string() }));
            sink(Msg::AiDone { seq, result: r });
        });
        self.ai_dirty();
    }

    pub(super) fn on_ai_token(&mut self, seq: u64, t: String) {
        let Some(a) = self.r.changes.ai.as_mut().filter(|a| a.seq == seq && a.stage == Stage::Streaming) else {
            return;
        };
        if a.reply.len() < 64 * 1024 {
            a.reply.push_str(&t);
        }
        self.ai_dirty();
    }

    pub(super) fn on_ai_done(&mut self, seq: u64, r: Result<String, String>) {
        let Some(a) = self.r.changes.ai.as_mut().filter(|a| a.seq == seq) else { return };
        a.cancel = None;
        a.stage = match r {
            Ok(full) if !full.trim().is_empty() => {
                a.reply = full;
                Stage::Done
            }
            Ok(_) => Stage::Failed("The endpoint sent an empty reply.".into()),
            Err(e) if e == "cancelled" => Stage::Done,
            Err(e) => Stage::Failed(e),
        };
        self.ai_dirty();
    }

    /// Stops a running request, or closes the pane.
    pub(super) fn ai_close(&mut self) {
        if let Some(a) = &self.r.changes.ai
            && let Some(c) = &a.cancel
        {
            c.store(true, Ordering::Relaxed);
        }
        self.r.changes.ai = None;
        self.ai_dirty();
    }

    fn ai_use(&mut self) {
        let Some(a) = &self.r.changes.ai else { return };
        let (subject, body) = split_message(&a.reply);
        if subject.is_empty() {
            return self.set_notice("The reply has no message to use.".into());
        }
        self.r.changes.subject = TextField::new(&subject);
        self.r.changes.desc = TextArea::new(&body);
        self.r.changes.focus = CFocus::Subject;
        self.r.changes.ai = None;
        self.set_notice("Took the AI's message: read it, edit it, then commit (Ctrl+Enter).".into());
        // The message box changed and the diff comes back where the pane was.
        self.changes_dirty();
        self.ai_dirty();
    }

    pub(super) fn ai_button(&mut self, hit: AiHit) {
        match hit {
            AiHit::Send => self.ai_send(),
            AiHit::Stop => {
                if let Some(c) = self.r.changes.ai.as_ref().and_then(|a| a.cancel.clone()) {
                    c.store(true, Ordering::Relaxed);
                }
            }
            AiHit::Use => self.ai_use(),
            AiHit::Again => self.ai_draft(),
            AiHit::Close => self.ai_close(),
        }
    }

    /// Keys while the pane is open (before the view's own keys).
    pub(super) fn ai_key(&mut self, k: Key) -> bool {
        let Some(a) = &self.r.changes.ai else { return false };
        let typing = matches!(self.r.changes.focus, CFocus::Subject | CFocus::Desc);
        match (k, &a.stage) {
            (Key::Esc, _) => self.ai_close(),
            (Key::Enter, Stage::Preview) if !typing => self.ai_send(),
            (Key::Enter, Stage::Done) if !typing => self.ai_use(),
            _ => return false,
        }
        true
    }

    pub(super) fn ai_wheel(&mut self, n: i64) {
        if let Some(a) = &mut self.r.changes.ai {
            a.top = (a.top as i64 + n).max(0) as usize;
        }
        self.ai_dirty();
    }

    pub(super) fn paint_ai(&mut self, cv: &mut Canvas, r: Rect, hits: &mut Vec<(Rect, changes_view::CHit)>) {
        let (ui, semi) = (self.f.ui, self.f.semi);
        let Some(a) = self.r.changes.ai.take() else { return };
        cv.fill(r, BG());
        let x0 = r.x as f32 + self.pf(18.0);
        let w = r.w as f32 - self.pf(36.0);
        // Header: what this is and where it goes.
        let local = a.endpoint.is_local();
        let title = "AI commit message";
        let e = self.te.draw(cv, semi, self.pf(14.0), x0, r.y as f32 + self.pf(28.0), title, TEXT(), w);
        let model = if a.endpoint.model.is_empty() { "no model set".to_string() } else { a.endpoint.model.clone() };
        let place = if local {
            format!("{} · {model} · stays on this machine", a.endpoint.host())
        } else {
            format!("{} · {model} · leaves this machine", a.endpoint.host())
        };
        self.te.draw(
            cv,
            ui,
            self.pf(12.0),
            e + self.pf(14.0),
            r.y as f32 + self.pf(28.0),
            &place,
            if local { ADD() } else { MOD() },
            w,
        );
        let body = Rect::new(r.x, r.y + self.px(44.0), r.w, r.h - self.px(44.0) - self.px(58.0));
        cv.fill(Rect::new(r.x, body.y - 1, r.w, 1), BORDER_SOFT());
        // Body: the request (preview) or the reply.
        let intro: Option<String> = match &a.stage {
            Stage::Building => Some("Preparing the request from the staged changes…".into()),
            Stage::Failed(e) => Some(e.clone()),
            _ => None,
        };
        // The text, wrapped to the pane (cached per width and content).
        let key =
            (body.w, u8::from(matches!(a.stage, Stage::Streaming | Stage::Done)), a.reply.len(), a.messages.len());
        let mut a = a;
        if a.view_key != key {
            let mut src: Vec<(Kind, String)> = Vec::new();
            match &a.stage {
                Stage::Preview => {
                    src.push((Kind::Note, "This is exactly what Send sends:".into()));
                    src.push((Kind::Note, String::new()));
                    for m in &a.messages {
                        src.push((Kind::Head, format!("Sent as the {} message", m.role)));
                        for l in m.content.lines() {
                            let k = if l.starts_with('+') && !l.starts_with("+++") {
                                Kind::Add
                            } else if l.starts_with('-') && !l.starts_with("---") {
                                Kind::Del
                            } else {
                                Kind::Mono
                            };
                            src.push((k, l.replace('\t', "    ")));
                        }
                        src.push((Kind::Note, String::new()));
                    }
                }
                Stage::Streaming | Stage::Done => {
                    for (i, l) in a.reply.lines().enumerate() {
                        src.push((if i == 0 { Kind::Subject } else { Kind::Reply }, l.to_string()));
                    }
                }
                _ => {}
            }
            let mut view = Vec::with_capacity(src.len());
            for (k, l) in src {
                let (font, size) = self.ai_font(k);
                // Lines that fit stay exactly as sent (indentation too).
                if l.is_empty() || self.te.measure(font, size, &l) <= w {
                    view.push((k, l));
                } else {
                    view.extend(wrap(&mut self.te, font, size, &l, w).into_iter().map(|p| (k, p)));
                }
            }
            a.view = view;
            a.view_key = key;
        }
        let old = cv.set_clip(cv.clip().intersect(body));
        let lh = self.px(19.0);
        let mut y = body.y as f32 + self.pf(22.0);
        if let Some(t) = intro {
            let c = if matches!(a.stage, Stage::Failed(_)) { DEL() } else { TEXT_3() };
            let size = self.pf(12.5);
            for l in wrap(&mut self.te, ui, size, &t, w) {
                self.te.draw(cv, ui, size, x0, y, &l, c, w);
                y += lh as f32;
            }
        }
        let fit = (body.h / lh.max(1)) as usize + 1;
        a.top = a.top.min(a.view.len().saturating_sub(1));
        for (k, (kind, l)) in a.view.iter().enumerate().skip(a.top).take(fit) {
            let yy = y + (k - a.top) as f32 * lh as f32;
            let (font, size) = self.ai_font(*kind);
            let color = match kind {
                Kind::Head => ACCENT_FG(),
                Kind::Add => ADD(),
                Kind::Del => DEL(),
                Kind::Mono => TEXT_2(),
                Kind::Note => TEXT_3(),
                Kind::Subject | Kind::Reply => TEXT(),
            };
            self.te.draw(cv, font, size, x0, yy, l, color, w + 4.0);
        }
        let lines = &a.view;
        if a.stage == Stage::Streaming && lines.is_empty() {
            self.te.draw(cv, ui, self.pf(12.5), x0, y, "Waiting for the first words…", TEXT_3(), w);
        }
        cv.set_clip(old);
        // Footer: notes and buttons.
        let fy = r.bottom() - self.px(46.0);
        cv.fill(Rect::new(r.x, fy - self.px(10.0), r.w, 1), BORDER_SOFT());
        let buttons: Vec<(String, AiHit, bool)> = match &a.stage {
            Stage::Building => vec![("Cancel".into(), AiHit::Close, false)],
            Stage::Preview => vec![
                (format!("Send to {}", a.endpoint.host()), AiHit::Send, true),
                ("Cancel".into(), AiHit::Close, false),
            ],
            Stage::Streaming => vec![("Stop".into(), AiHit::Stop, false)],
            Stage::Done => vec![
                ("Use this message".into(), AiHit::Use, true),
                ("Try again".into(), AiHit::Again, false),
                ("Close".into(), AiHit::Close, false),
            ],
            Stage::Failed(_) => vec![("Try again".into(), AiHit::Again, false), ("Close".into(), AiHit::Close, false)],
        };
        let mut bx = r.right() - self.px(18.0);
        for (label, hit, primary) in buttons.iter().rev() {
            let bw = (self.te.measure(semi, self.pf(12.5), label) + self.pf(28.0)) as i32;
            let b = Rect::new(bx - bw, fy, bw, self.px(30.0));
            let hot = self.is_hot(Hot::Change(hits.len()));
            if *primary {
                cv.fill_round(b, self.pf(7.0), if hot { ACCENT_FG() } else { ACCENT() });
                self.te.draw(
                    cv,
                    semi,
                    self.pf(12.5),
                    b.x as f32 + self.pf(14.0),
                    (b.y as f32 + self.pf(20.0)).round(),
                    label,
                    ACCENT_TEXT(),
                    bw as f32,
                );
            } else {
                let (bg, border) = if hot { (CONTROL_HOVER(), BORDER_HOVER()) } else { (BG_BAR(), BORDER()) };
                cv.round_border(b, self.pf(7.0), border, bg);
                self.te.draw(
                    cv,
                    semi,
                    self.pf(12.5),
                    b.x as f32 + self.pf(14.0),
                    (b.y as f32 + self.pf(20.0)).round(),
                    label,
                    TEXT(),
                    bw as f32,
                );
            }
            hits.push((b, changes_view::CHit::AiButton(*hit)));
            bx = b.x - self.px(8.0);
        }
        let mut note = Vec::new();
        if a.stage == Stage::Preview {
            if !a.excluded.is_empty() {
                note.push(format!("Left out (secret-looking): {}", a.excluded.join(", ")));
            }
            if a.truncated {
                note.push("The diff is cut at 24 KB".into());
            }
            if note.is_empty() {
                note.push("Enter sends · Esc cancels".into());
            }
        } else if a.stage == Stage::Done {
            note.push("A suggestion: Enter or Use puts it in the message box to edit".into());
        }
        let nt = note.join(" · ");
        self.te.draw(
            cv,
            ui,
            self.pf(11.5),
            x0,
            (fy as f32 + self.pf(20.0)).round(),
            &nt,
            TEXT_3(),
            (bx - r.x) as f32 - self.pf(30.0),
        );
        self.r.changes.ai = Some(a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_split_into_subject_and_body() {
        assert_eq!(
            split_message("Add the AI pane\n\nIt shows the request first.\n"),
            ("Add the AI pane".into(), "It shows the request first.".into())
        );
        assert_eq!(split_message("```\nFix the crash\n```"), ("Fix the crash".into(), String::new()));
        assert_eq!(split_message("\n\"Subject: Tidy up\"\n"), ("Tidy up".into(), String::new()));
    }

    #[test]
    fn the_request_leaves_secrets_out() {
        let dir = std::env::temp_dir().join(format!("ffwd-ai-prepare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        assert!(prepare(&g).is_err(), "nothing staged");
        std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join(".env"), "TOKEN=hunter2\n").unwrap();
        g.output(&["add", "."]).unwrap();
        let (m, ex, cut) = prepare(&g).unwrap();
        assert_eq!(ex, [".env"]);
        assert!(!cut);
        let user = &m[1].content;
        assert!(user.contains("fn main() {}"));
        assert!(!user.contains("hunter2"), "secret content is never sent");
        assert!(user.contains(".env"), "the name is mentioned, the content is not");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
