//! Copy, Cut and Paste of a pattern (this fork, `pattern_clip.rs`), headless: synthetic events
//! run through `App::show` on the panel at 1:1 (panel pixels are screen points).

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use orb_ui::app::App;
use orb_ui::layout::{self, PANEL_H, PANEL_W};
use orb_ui::menu::Cmd;
use orb_ui::model::{Event as PanelEvent, Panel, Step303};
use orb_ui::pattern_clip::{ClipCmd, PatternClip, Steps};

struct Harness {
    ctx: egui::Context,
    app: App,
    time: f64,
    /// The text the last frames put on the system clipboard.
    copied: Option<String>,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut h = Self {
            ctx,
            app: App::new(Panel::new(), None),
            time: 0.0,
            copied: None,
        };
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.016;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(PANEL_W, PANEL_H))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.show(ui));
        out.textures_delta.clear();
        for c in out.platform_output.commands {
            if let egui::OutputCommand::CopyText(t) = c {
                self.copied = Some(t);
            }
        }
    }

    fn click(&mut self, id: &str, button: PointerButton) {
        let c = layout::controls().into_iter().find(|c| c.id == id).unwrap();
        let p = Pos2::new(c.x + c.w / 2.0, c.y + c.h / 2.0);
        self.time += 1.0;
        self.frame(vec![Event::PointerMoved(p)]);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos: p,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }
    }
}

/// A 303 pattern with something on every field.
fn bass_steps() -> [Step303; 16] {
    let mut s = [Step303::default(); 16];
    for (i, st) in s.iter_mut().enumerate() {
        *st = Step303 {
            note: (i % 13) as u8,
            slide: i % 3 == 0,
            accent: i % 4 == 0,
            up: i % 5 == 0,
            down: i % 7 == 0,
            rest: i % 2 == 1,
        };
    }
    s
}

#[test]
fn copy_then_paste_into_another_pattern_of_the_303() {
    let mut h = Harness::new();
    h.app.panel.set_pattern_303(0, bass_steps());
    h.app.panel.rows[0].len[0] = 7;
    h.app.panel.rows[0].shuffle[0] = true;
    h.click("pattern1.pattern1", PointerButton::Primary);
    assert_eq!(h.app.panel.focus, 0);

    // Cmd/Ctrl+C reaches the app as egui's Copy event
    h.frame(vec![Event::Copy]);
    let text = h
        .copied
        .clone()
        .expect("the pattern's text on the clipboard");
    assert_eq!(
        PatternClip::from_text(&text).as_ref(),
        h.app.copied_pattern()
    );

    h.click("pattern1.pattern2", PointerButton::Primary);
    assert_eq!(h.app.panel.rows[0].pattern, 1);
    h.app.panel.events.clear();
    h.frame(vec![Event::Paste(text)]);
    assert_eq!(h.app.panel.pattern_303(0), bass_steps());
    assert_eq!(h.app.panel.rows[0].len[1], 7);
    assert!(h.app.panel.rows[0].shuffle[1]);
    // the engine hears the steps, the length and the shuffle flag
    let ev = &h.app.panel.events;
    assert!(ev.iter().any(|e| matches!(
        e,
        PanelEvent::Step303 {
            synth: 0,
            pattern: 1,
            ..
        }
    )));
    assert!(ev.contains(&PanelEvent::Set {
        id: "pattern1.steps".into(),
        value: 7
    }));
    assert!(ev.contains(&PanelEvent::Set {
        id: "pattern1.shuffle".into(),
        value: 1
    }));
    // the first pattern is unchanged
    assert_eq!(h.app.panel.synth[0].patterns[0], bass_steps());
}

#[test]
fn a_303_pattern_pastes_into_the_other_303_but_not_into_a_drum_machine() {
    let mut h = Harness::new();
    h.app.panel.set_pattern_303(0, bass_steps());
    h.app.panel.focus = 0;
    h.frame(vec![Event::Copy]);
    let text = h.copied.clone().unwrap();

    h.app.panel.focus = 1;
    h.frame(vec![Event::Paste(text.clone())]);
    assert_eq!(h.app.panel.pattern_303(1), bass_steps());

    let drums = h.app.panel.steps808;
    h.app.panel.focus = 2;
    h.app.panel.events.clear();
    h.frame(vec![Event::Paste(text)]);
    assert_eq!(h.app.panel.steps808, drums);
    assert!(h.app.panel.events.is_empty());
}

#[test]
fn cut_clears_the_steps_and_keeps_the_length() {
    let mut h = Harness::new();
    h.app.panel.set_pattern_303(1, bass_steps());
    h.app.panel.rows[1].len[0] = 9;
    h.app.panel.focus = 1;
    h.app.panel.events.clear();
    h.frame(vec![Event::Cut]);
    assert_eq!(h.app.panel.pattern_303(1), [Step303::default(); 16]);
    assert_eq!(h.app.panel.rows[1].len[0], 9);
    assert!(h.app.panel.events.contains(&PanelEvent::Clear303 {
        synth: 1,
        pattern: 0
    }));
    // and the cut pattern pastes back
    let text = h.copied.clone().unwrap();
    h.frame(vec![Event::Paste(text)]);
    assert_eq!(h.app.panel.pattern_303(1), bass_steps());
}

#[test]
fn drum_patterns_copy_cut_and_paste_on_their_own_machine() {
    let mut h = Harness::new();
    let p = &mut h.app.panel;
    p.steps808[0][0] = true;
    p.steps808[5][3] = true;
    p.steps909[8][2] = 2;
    p.steps909[1][15] = 3;
    let (d808, d909) = (p.steps808, p.steps909);

    for r in [2, 3] {
        h.app.panel.focus = r;
        h.frame(vec![Event::Cut]);
        let text = h.copied.clone().unwrap();
        assert_eq!(h.app.panel.steps808 == [[false; 16]; 12], r == 2);
        assert_eq!(h.app.panel.steps909 == [[0; 16]; 12], r == 3);
        // the 808's pattern does not go into the 909 or a 303
        let other = 5 - r;
        h.app.panel.focus = other;
        h.app.panel.events.clear();
        h.frame(vec![Event::Paste(text.clone())]);
        assert!(h.app.panel.events.is_empty(), "row {r} into row {other}");
        h.app.panel.focus = r;
        h.frame(vec![Event::Paste(text)]);
    }
    assert_eq!(h.app.panel.steps808, d808);
    assert_eq!(h.app.panel.steps909, d909);
}

#[test]
fn a_paste_of_other_text_does_nothing() {
    let mut h = Harness::new();
    h.app.panel.focus = 0;
    h.frame(vec![Event::Copy]);
    h.app.panel.events.clear();
    h.frame(vec![Event::Paste("some text from another program".into())]);
    assert!(h.app.panel.events.is_empty());
}

#[test]
fn the_edit_menu_runs_on_the_focused_row() {
    let mut h = Harness::new();
    assert!(!h.app.can_paste_pattern());
    h.app.panel.set_pattern_303(0, bass_steps());
    h.app.panel.focus = 0;
    h.app.command(Cmd::Pattern(ClipCmd::Copy));
    assert!(h.app.can_paste_pattern());
    h.app.panel.focus = 2;
    // a 303 pattern does not fit the 808
    assert!(!h.app.can_paste_pattern());
    h.app.panel.focus = 1;
    h.app.command(Cmd::Pattern(ClipCmd::Paste));
    assert_eq!(h.app.panel.pattern_303(1), bass_steps());
    // the text went to the clipboard at the end of the next frame
    h.frame(vec![]);
    let clip = PatternClip::from_text(&h.copied.clone().unwrap()).unwrap();
    assert_eq!(clip.steps, Steps::Bass(bass_steps()));
}

#[test]
fn a_right_click_on_the_lit_pattern_button_opens_the_menu_and_selects_nothing() {
    let mut h = Harness::new();
    h.click("pattern2.pattern1", PointerButton::Secondary);
    assert!(egui::Popup::is_any_open(&h.ctx));
    assert_eq!(h.app.panel.rows[1].pattern, 0);
    h.frame(vec![Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    h.frame(vec![]);
    assert!(!egui::Popup::is_any_open(&h.ctx));

    // on a button that is not lit: no menu, and the pattern stays
    h.app.panel.events.clear();
    h.click("pattern2.pattern3", PointerButton::Secondary);
    assert!(!egui::Popup::is_any_open(&h.ctx));
    assert_eq!(h.app.panel.rows[1].pattern, 0);
    assert!(
        !h.app
            .panel
            .events
            .iter()
            .any(|e| matches!(e, PanelEvent::Pattern { .. }))
    );
}
