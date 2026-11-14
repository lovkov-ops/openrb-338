//! Copy, Cut and Paste of a pattern (this fork): the selected pattern of the row with the focus
//! (or of the row whose lit pattern button was right-clicked). Cmd+C / Cmd+X / Cmd+V on macOS,
//! Ctrl+C / Ctrl+X / Ctrl+V elsewhere: egui turns the platform's shortcuts into its Copy, Cut
//! and Paste events, which the panel takes while no text field has the keyboard.
//!
//! A pattern holds its 16 steps (a 303's notes and flags, or a drum machine's 12 instruments),
//! its length and its shuffle flag. A 303's pattern pastes into either 303; the 808's and the
//! 909's only into their own machine. Cut is Copy, then the steps are cleared (as the 303's
//! Clear button does); the length and the shuffle flag stay.
//!
//! The copied pattern is also put on the system clipboard as text ([`PatternClip::to_text`]):
//! egui sends a Paste event only when the clipboard holds text, and the text lets a pattern go
//! from one window of the app to another. A Paste of any other text does nothing. The pattern
//! menu's Paste uses the app's own copy.

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, Response, Ui};

use crate::app::App;
use crate::model::{Panel, Step303};

pub const COPY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::C);
pub const CUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::X);
pub const PASTE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::V);

/// The first line of a pattern's text.
const HEADER: &str = "OpenRB-338x pattern 1";

/// The steps of a pattern, by machine.
#[derive(Clone, Debug, PartialEq)]
pub enum Steps {
    Bass([Step303; 16]),
    Tr808([[bool; 16]; 12]),
    Tr909([[u8; 16]; 12]),
}

/// A copied pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternClip {
    pub steps: Steps,
    /// 1..=16.
    pub len: u8,
    pub shuffle: bool,
}

impl PatternClip {
    /// Whether it pastes into row `r` (0 and 1 the 303s, 2 the 808, 3 the 909).
    pub fn fits(&self, r: usize) -> bool {
        matches!(
            (&self.steps, r),
            (Steps::Bass(_), 0 | 1) | (Steps::Tr808(_), 2) | (Steps::Tr909(_), 3)
        )
    }

    /// The text form: a header, the kind, the length and the shuffle flag, then a line per
    /// step of a 303 (note slide accent up down rest) or per instrument of a drum machine (its
    /// 16 step values).
    pub fn to_text(&self) -> String {
        let kind = match self.steps {
            Steps::Bass(_) => "303",
            Steps::Tr808(_) => "808",
            Steps::Tr909(_) => "909",
        };
        let mut t = format!(
            "{HEADER}\nkind {kind}\nlen {}\nshuffle {}\n",
            self.len, self.shuffle as u8
        );
        let line = |vals: &mut dyn Iterator<Item = u8>| {
            vals.map(|v| v.to_string()).collect::<Vec<_>>().join(" ") + "\n"
        };
        match &self.steps {
            Steps::Bass(s) => {
                for st in s {
                    t += &line(
                        &mut [
                            st.note,
                            st.slide as u8,
                            st.accent as u8,
                            st.up as u8,
                            st.down as u8,
                            st.rest as u8,
                        ]
                        .into_iter(),
                    );
                }
            }
            Steps::Tr808(s) => {
                for inst in s {
                    t += &line(&mut inst.iter().map(|&b| b as u8));
                }
            }
            Steps::Tr909(s) => {
                for inst in s {
                    t += &line(&mut inst.iter().copied());
                }
            }
        }
        t
    }

    /// Reads [`PatternClip::to_text`]'s form; any other text is None.
    pub fn from_text(text: &str) -> Option<Self> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next()? != HEADER {
            return None;
        }
        let mut field = |name: &str| -> Option<String> {
            Some(lines.next()?.strip_prefix(name)?.trim().to_string())
        };
        let kind = field("kind")?;
        let len: u8 = field("len")?
            .parse()
            .ok()
            .filter(|l| (1..=16).contains(l))?;
        let shuffle = match field("shuffle")?.as_str() {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let rows: Vec<Vec<u8>> = lines
            .map(|l| l.split_whitespace().map(|v| v.parse().ok()).collect())
            .collect::<Option<_>>()?;
        let flag = |v: u8| (v <= 1).then_some(v == 1);
        let steps = match kind.as_str() {
            "303" if rows.len() == 16 => {
                let mut s = [Step303::default(); 16];
                for (st, r) in s.iter_mut().zip(&rows) {
                    let [note, slide, accent, up, down, rest] = r[..] else {
                        return None;
                    };
                    *st = Step303 {
                        note,
                        slide: flag(slide)?,
                        accent: flag(accent)?,
                        up: flag(up)?,
                        down: flag(down)?,
                        rest: flag(rest)?,
                    };
                }
                Steps::Bass(s)
            }
            "808" if rows.len() == 12 => {
                let mut s = [[false; 16]; 12];
                for (inst, r) in s.iter_mut().zip(&rows) {
                    if r.len() != 16 {
                        return None;
                    }
                    for (b, &v) in inst.iter_mut().zip(r) {
                        *b = flag(v)?;
                    }
                }
                Steps::Tr808(s)
            }
            "909" if rows.len() == 12 => {
                let mut s = [[0u8; 16]; 12];
                for (inst, r) in s.iter_mut().zip(&rows) {
                    if r.len() != 16 {
                        return None;
                    }
                    inst.copy_from_slice(r);
                }
                Steps::Tr909(s)
            }
            _ => return None,
        };
        Some(Self {
            steps,
            len,
            shuffle,
        })
    }
}

impl Panel {
    /// The selected pattern of row `r`.
    pub fn copy_pattern(&self, r: usize) -> PatternClip {
        let row = &self.rows[r];
        let p = row.pattern as usize;
        let steps = match r {
            0 | 1 => Steps::Bass(self.pattern_303(r)),
            2 => Steps::Tr808(self.steps808),
            _ => Steps::Tr909(self.steps909),
        };
        PatternClip {
            steps,
            len: row.len[p],
            shuffle: row.shuffle[p],
        }
    }

    /// Copies the selected pattern of row `r`, then clears its steps.
    pub fn cut_pattern(&mut self, r: usize) -> PatternClip {
        let clip = self.copy_pattern(r);
        match r {
            0 | 1 => self.press(&format!("tb303_{}.clear", r + 1), 0.0),
            2 => {
                for i in 0..12 {
                    for s in 0..16 {
                        self.put_step808(i, s, false);
                    }
                }
            }
            _ => {
                for i in 0..12 {
                    for s in 0..16 {
                        self.put_step909(i, s, 0);
                    }
                }
            }
        }
        clip
    }

    /// Writes `clip` into the selected pattern of row `r`, sending each change as the panel's
    /// own buttons would; false (and nothing changes) when it does not fit the row.
    pub fn paste_pattern(&mut self, r: usize, clip: &PatternClip) -> bool {
        if !clip.fits(r) {
            return false;
        }
        match &clip.steps {
            Steps::Bass(s) => self.set_pattern_303(r, *s),
            Steps::Tr808(s) => {
                for (i, inst) in s.iter().enumerate() {
                    for (step, &on) in inst.iter().enumerate() {
                        self.put_step808(i, step, on);
                    }
                }
            }
            Steps::Tr909(s) => {
                for (i, inst) in s.iter().enumerate() {
                    for (step, &v) in inst.iter().enumerate() {
                        self.put_step909(i, step, v);
                    }
                }
            }
        }
        let p = self.rows[r].pattern as usize;
        if self.rows[r].len[p] != clip.len {
            self.rows[r].len[p] = clip.len;
            self.set_event(&format!("pattern{}.steps", r + 1), clip.len as i32);
        }
        if self.rows[r].shuffle[p] != clip.shuffle {
            self.rows[r].shuffle[p] = clip.shuffle;
            self.set_event(&format!("pattern{}.shuffle", r + 1), clip.shuffle as i32);
        }
        true
    }
}

/// Which command of the three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipCmd {
    Copy,
    Cut,
    Paste,
}

impl App {
    /// Copy or Cut of row `r`'s selected pattern: kept by the app, and its text goes to the
    /// system clipboard at the end of the frame (`App::show`).
    pub fn pattern_copy(&mut self, r: usize, cut: bool) {
        let clip = if cut {
            self.panel.cut_pattern(r)
        } else {
            self.panel.copy_pattern(r)
        };
        self.clip_text = Some(clip.to_text());
        self.pattern_clip = Some(clip);
    }

    /// The pattern Copy or Cut took last.
    pub fn copied_pattern(&self) -> Option<&PatternClip> {
        self.pattern_clip.as_ref()
    }

    /// Paste into row `r`'s selected pattern: `clip`, or the app's own copy.
    pub fn pattern_paste(&mut self, r: usize, clip: Option<PatternClip>) {
        if let Some(c) = clip.or_else(|| self.pattern_clip.clone()) {
            self.panel.paste_pattern(r, &c);
        }
    }

    /// The Copy, Cut and Paste events of this frame, on the row with the focus. The caller
    /// calls it only while the panel takes keys and no text field has the keyboard.
    pub(crate) fn pattern_clipboard_keys(&mut self, ui: &Ui) {
        let events = ui.input(|i| i.events.clone());
        let r = self.panel.focus;
        for ev in events {
            match ev {
                egui::Event::Copy => self.pattern_copy(r, false),
                egui::Event::Cut => self.pattern_copy(r, true),
                // only a pattern's text pastes; other text on the clipboard does nothing
                egui::Event::Paste(text) => {
                    if let Some(clip) = PatternClip::from_text(&text) {
                        self.pattern_paste(r, Some(clip));
                    }
                }
                _ => {}
            }
        }
    }

    /// The menu of a right click on row `r`'s lit pattern button: Copy, Cut, Paste.
    pub(crate) fn pattern_menu(&mut self, response: &Response, r: usize) {
        let mut cmd = None;
        let can_paste = self.pattern_clip.as_ref().is_some_and(|c| c.fits(r));
        response.context_menu(|ui| {
            cmd = pattern_menu_items(ui, can_paste);
        });
        if let Some(c) = cmd {
            self.run_clip_cmd(r, c);
        }
    }

    /// Runs a command of the pattern menu or the Edit menu on row `r`.
    pub fn run_clip_cmd(&mut self, r: usize, c: ClipCmd) {
        self.panel.focus = r;
        match c {
            ClipCmd::Copy => self.pattern_copy(r, false),
            ClipCmd::Cut => self.pattern_copy(r, true),
            ClipCmd::Paste => self.pattern_paste(r, None),
        }
    }

    /// Whether the app's copy pastes into the row with the focus (the Edit menu's Paste).
    pub fn can_paste_pattern(&self) -> bool {
        self.pattern_clip
            .as_ref()
            .is_some_and(|c| c.fits(self.panel.focus))
    }
}

/// The three items with their shortcuts; the one clicked (the menu closes).
pub(crate) fn pattern_menu_items(ui: &mut Ui, can_paste: bool) -> Option<ClipCmd> {
    let mut cmd = None;
    for (label, sc, c, on) in [
        ("Copy Pattern", COPY, ClipCmd::Copy, true),
        ("Cut Pattern", CUT, ClipCmd::Cut, true),
        ("Paste Pattern", PASTE, ClipCmd::Paste, can_paste),
    ] {
        let b = egui::Button::new(label).shortcut_text(crate::shortcuts::text(ui.ctx(), &sc));
        if ui.add_enabled(on, b).clicked() {
            cmd = Some(c);
            ui.close();
        }
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_other_text_is_refused() {
        let mut s = [Step303::default(); 16];
        s[3] = Step303 {
            note: 12,
            slide: true,
            accent: true,
            up: false,
            down: true,
            rest: false,
        };
        let mut d = [[0u8; 16]; 12];
        d[8][2] = 3;
        let mut e = [[false; 16]; 12];
        e[0][15] = true;
        for steps in [Steps::Bass(s), Steps::Tr909(d), Steps::Tr808(e)] {
            let c = PatternClip {
                steps,
                len: 7,
                shuffle: true,
            };
            assert_eq!(PatternClip::from_text(&c.to_text()), Some(c.clone()));
            // Windows line ends
            let crlf = c.to_text().replace('\n', "\r\n");
            assert_eq!(PatternClip::from_text(&crlf), Some(c));
        }
        assert_eq!(PatternClip::from_text("hello"), None);
        assert_eq!(PatternClip::from_text(""), None);
        let c = PatternClip {
            steps: Steps::Tr808(e),
            len: 16,
            shuffle: false,
        };
        let bad = c.to_text().replace("len 16", "len 17");
        assert_eq!(PatternClip::from_text(&bad), None);
        let short: String = c.to_text().lines().take(10).collect::<Vec<_>>().join("\n");
        assert_eq!(PatternClip::from_text(&short), None);
    }
}
