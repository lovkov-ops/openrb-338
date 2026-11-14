//! The LFOs of the bass synths' large knobs (this fork's `lfo.rs`), headless with a real player: it
//! moves the cutoff in the engine around the knob's value while the knob shows the user's
//! value, the user's move of the knob is its new value, its buttons take clicks, and an .rbsx
//! keeps its settings.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use orb_engine::samples::SampleSet;
use orb_engine::tables::Tables;
use orb_player::{Audio, CALL_FRAMES};
use orb_ui::app::App;
use orb_ui::behavior;
use orb_ui::browser::Mode;
use orb_ui::layout::{self, PANEL_H, PANEL_W};
use orb_ui::lfo::{self, Shape};
use orb_ui::link::{self, Link, Msg};
use orb_ui::menu::Cmd;
use orb_ui::model::Panel;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tables() -> Arc<Tables> {
    static T: OnceLock<Arc<Tables>> = OnceLock::new();
    T.get_or_init(|| Arc::new(Tables::new())).clone()
}

fn samples() -> Arc<SampleSet> {
    rbm::standard::builtin_samples()
}

struct Harness {
    ctx: egui::Context,
    app: App,
    audio: Audio,
    time: f64,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let (link, audio) = Link::new(tables(), Some(samples()));
        let mut app = App::with_link(Panel::new(), link, None);
        app.samples = Some(samples());
        let mut h = Self {
            ctx,
            app,
            audio,
            time: 0.0,
        };
        h.calls(1);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.016;
        let mut ev = vec![Event::ModifiersChanged(Modifiers::NONE)];
        ev.extend(events);
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(PANEL_W, PANEL_H))),
            time: Some(self.time),
            events: ev,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.show(ui));
        out.textures_delta.clear();
    }

    fn calls(&mut self, n: usize) {
        let mut out = vec![0i16; 2 * CALL_FRAMES];
        for _ in 0..n {
            self.audio.render_i16(&mut out);
            self.frame(vec![]);
        }
    }

    fn open(&mut self, path: &Path) {
        self.app.open_song(path);
        assert!(self.app.errors.is_empty(), "{:?}", self.app.errors);
        self.calls(2);
    }

    fn start_log(&mut self) {
        self.app.link.as_mut().unwrap().log = Some(vec![]);
    }

    /// The cutoff values the link sent the engine since the log started (synth `k`).
    fn cutoffs_sent(&mut self, k: usize) -> Vec<u8> {
        self.sent(k, 0)
    }

    /// The values of synth `k`'s knob `param` (`lfo::PARAMS`) the link sent since the log
    /// started.
    fn sent(&mut self, k: usize, param: usize) -> Vec<u8> {
        let (d, ctrl) = link::controls()[&lfo::ids(k, param)[0]];
        let log = self
            .app
            .link
            .as_mut()
            .unwrap()
            .log
            .take()
            .unwrap_or_default();
        self.start_log();
        log.into_iter()
            .filter_map(|m| match m {
                Msg::Device { dev, data, .. } if dev == d.id() && data.first() == Some(&ctrl) => {
                    data.get(1).copied()
                }
                _ => None,
            })
            .collect()
    }

    fn center(id: &str) -> Pos2 {
        let c = layout::controls().into_iter().find(|c| c.id == id).unwrap();
        Pos2::new(c.x + c.w / 2.0, c.y + c.h / 2.0)
    }

    fn click_at(&mut self, p: Pos2) {
        self.frame(vec![Event::PointerMoved(p)]);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }
    }

    /// Drags the knob `id` up by `dy` panel pixels.
    fn drag_up(&mut self, id: &str, dy: f32) {
        let p = Self::center(id);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(p)]);
        self.frame(vec![button(p, true)]);
        let q = Pos2::new(p.x, p.y - dy);
        self.frame(vec![Event::PointerMoved(q)]);
        self.frame(vec![button(q, false)]);
    }

    /// Sets an LFO knob as a turn of it would (its value 0..=127).
    fn set(&mut self, id: &str, v: u8) {
        self.app
            .panel
            .set_knob(id, behavior::knob_from_engine(v as i32));
    }
}

fn song() -> PathBuf {
    root().join("tests/songs/diag/songs/303_saw_lowcut.rbs")
}

#[test]
fn an_lfo_at_no_amount_sends_nothing_and_its_knobs_start_at_their_defaults() {
    let mut h = Harness::new();
    h.open(&song());
    let [_, rate, amount, _] = lfo::ids(0, 0);
    assert_eq!(h.app.panel.value(&rate), Some(lfo::RATE_DEFAULT));
    assert_eq!(h.app.panel.value(&amount), Some(lfo::AMOUNT_DEFAULT));
    assert_eq!(h.app.lfo_mode(0, 0), lfo::Mode::default());
    h.start_log();
    h.calls(40);
    assert!(h.cutoffs_sent(0).is_empty());
    assert_eq!(h.app.lfo_base(0, 0), None);
    assert!(h.app.gen_state().is_empty());
}

#[test]
fn the_lfo_moves_the_engines_cutoff_while_the_knob_keeps_the_users_value() {
    let mut h = Harness::new();
    h.open(&song());
    let [cutoff, rate, amount, _] = lfo::ids(0, 0);
    let base = h.app.panel.value(&cutoff).unwrap();
    h.set(&rate, 127); // 20 Hz free: a cycle in 3 frames
    h.set(&amount, 32);
    h.start_log();
    h.calls(60);
    let sent = h.cutoffs_sent(0);
    assert!(sent.len() > 10, "{sent:?}");
    let (lo, hi) = (*sent.iter().min().unwrap(), *sent.iter().max().unwrap());
    assert!(lo < base && hi > base, "{lo} {base} {hi}");
    // AMOUNT 32 of 127 moves it at most 32 either way
    assert!(base - lo <= 32 && hi - base <= 32, "{lo} {base} {hi}");
    // the knob and the saved song keep the user's value
    assert_eq!(h.app.panel.value(&cutoff), Some(base));
    assert_eq!(h.app.lfo_base(0, 0), Some(base));
    // the other synth's LFO does not run
    assert!(h.cutoffs_sent(1).is_empty());

    // the user's move of the knob is the new value to move around
    h.drag_up(&cutoff, 20.0);
    h.calls(2);
    let moved = h.app.lfo_base(0, 0).unwrap();
    assert!(moved > base, "{moved} {base}");
    assert_eq!(h.app.panel.value(&cutoff), Some(moved));

    // no amount: the LFO stops and the engine gets the user's value back
    h.set(&amount, 0);
    h.calls(2);
    assert_eq!(h.cutoffs_sent(0).last(), Some(&moved));
    assert_eq!(h.app.lfo_base(0, 0), None);
    h.calls(20);
    assert!(h.cutoffs_sent(0).is_empty());
}

#[test]
fn the_mode_buttons_take_a_click_on_each_half() {
    // (this fork) two cells: the note (tempo sync) and the wave, one button that flips the
    // sine and the random sine
    let mut h = Harness::new();
    let id = &lfo::ids(1, 0)[3];
    let c = layout::controls()
        .into_iter()
        .find(|c| &c.id == id)
        .unwrap();
    let at = |half: f32| Pos2::new(c.x + c.w * (half + 0.5) / 2.0, c.y + c.h / 2.0);
    assert_eq!(h.app.lfo_mode(1, 0), lfo::Mode::default());
    assert_eq!(h.app.lfo_mode(1, 0).shape, Shape::Sine);
    // the note: tempo sync on, then off again
    h.click_at(at(0.0));
    assert!(h.app.lfo_mode(1, 0).sync);
    // the wave: the random sine, then the sine again
    h.click_at(at(1.0));
    assert_eq!(h.app.lfo_mode(1, 0).shape, Shape::SampleHold);
    assert!(h.app.lfo_mode(1, 0).sync, "the wave leaves the note alone");
    h.click_at(at(1.0));
    assert_eq!(h.app.lfo_mode(1, 0).shape, Shape::Sine);
    h.click_at(at(0.0));
    assert!(!h.app.lfo_mode(1, 0).sync);
    // the upper synth's buttons were left alone; the clicks count as edits
    assert_eq!(h.app.lfo_mode(0, 0), lfo::Mode::default());
    assert!(h.app.doc.modified);
}

#[test]
fn an_rbsx_keeps_the_lfo_and_an_rbs_save_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new();
    h.open(&song());
    let [_, rate, amount, _] = lfo::ids(1, 0);
    h.set(&rate, 90);
    h.set(&amount, 40);
    h.app.press_lfo_mode(1, 0, 0);
    h.app.press_lfo_mode(1, 0, 2);
    h.calls(3);
    let kept = h.app.gen_state();
    assert!(!kept.is_empty());
    let out = dir.path().join("lfo.rbsx");
    h.app.command(Cmd::SaveAs);
    h.app.chosen(Mode::SaveSong, out.clone());
    assert!(h.app.errors.is_empty(), "{:?}", h.app.errors);

    let mut h2 = Harness::new();
    h2.open(&out);
    assert_eq!(h2.app.gen_state(), kept);
    assert_eq!(h2.app.panel.value(&rate), Some(90));
    assert_eq!(h2.app.panel.value(&amount), Some(40));
    assert_eq!(
        h2.app.lfo_mode(1, 0),
        lfo::Mode {
            sync: true,
            shape: Shape::SampleHold
        }
    );

    // an .rbs of a song with an LFO set: Save asks first, as for the GENERATE menu
    let src = dir.path().join("old.rbs");
    std::fs::copy(song(), &src).unwrap();
    let mut h3 = Harness::new();
    h3.open(&src);
    h3.set(&lfo::ids(0, 0)[2], 10);
    h3.calls(1);
    h3.app.command(Cmd::Save);
    assert!(h3.app.format_prompting());
}

#[test]
fn each_knobs_lfo_moves_only_its_own_knob() {
    let mut h = Harness::new();
    h.open(&song());
    // the lower synth's DECAY: its own RATE, AMOUNT and buttons
    let decay = lfo::PARAMS.iter().position(|p| *p == "decay").unwrap();
    let [knob, rate, amount, mode] = lfo::ids(1, decay);
    assert_eq!(knob, "tb303_2.decay");
    let base = h.app.panel.value(&knob).unwrap();
    h.set(&rate, 127);
    h.set(&amount, 20);
    // sample & hold, by a click on its third of DECAY's buttons
    let c = layout::controls()
        .into_iter()
        .find(|c| c.id == mode)
        .unwrap();
    h.click_at(Pos2::new(c.x + c.w * 2.5 / 3.0, c.y + c.h / 2.0));
    assert_eq!(h.app.lfo_mode(1, decay).shape, Shape::SampleHold);
    assert_eq!(h.app.lfo_mode(1, 0).shape, Shape::Sine);
    h.start_log();
    h.calls(80);
    let sent = h.sent(1, decay);
    assert!(sent.len() > 10, "{sent:?}");
    assert!(
        sent.iter().all(|v| v.abs_diff(base) <= 20),
        "{sent:?} around {base}"
    );
    assert_eq!(h.app.panel.value(&knob), Some(base));
    // no other knob of either synth was moved
    for k in 0..2 {
        for p in 0..lfo::PARAMS.len() {
            if (k, p) != (1, decay) {
                assert!(h.sent(k, p).is_empty(), "{}", lfo::ids(k, p)[0]);
                assert_eq!(h.app.lfo_base(k, p), None);
            }
        }
    }
    // every large knob of both synths has its LFO's controls on the panel
    let ids: Vec<String> = layout::controls().into_iter().map(|c| c.id).collect();
    for k in 0..2 {
        for p in 0..lfo::PARAMS.len() {
            for id in lfo::ids(k, p) {
                assert!(ids.contains(&id), "{id}");
            }
        }
    }
}
