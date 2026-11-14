//! The LFOs of a bass synth's five large knobs (this fork): CUTOFF, RESO, ENV MOD, DECAY and
//! ACCENT each have their own. An LFO moves its knob's value up and down around the value the
//! knob is set to, while the knob stays where the user put it.
//!
//! Two small knobs drive it: RATE and AMOUNT, how far the value moves (0: no movement, full:
//! the whole range above and below the knob's value, clipped to 0..=127). They sit under the
//! knob, left and right of two buttons that choose how: the note ties the rate to the song's
//! tempo (off: free, in hertz), and the wave button flips between the sine (a smooth up and
//! down) and the random sine, sample & hold (every half cycle a new random height on the other
//! side, glided to smoothly: an uneven sine, as fast as the sine).
//!
//! Free, the rate knob goes from `FREE_MIN_HZ` to `FREE_MAX_HZ` on a log scale. Tied to the
//! tempo it picks one cycle's length from `SYNC_BEATS`, and while the song plays the cycle is
//! locked to the song position, so it starts on the bar.

use eframe::egui::{self, Context, PointerButton, Rect, Sense, Ui};

use crate::app::{self, App};
use crate::behavior;
use crate::layout::Ctl;
use crate::random::Rng;

/// The free rate's range, in cycles a second.
pub const FREE_MIN_HZ: f64 = 0.05;
pub const FREE_MAX_HZ: f64 = 20.0;
/// The tempo-tied cycle lengths in beats, from the rate knob's bottom to its top: 4 bars, 2
/// bars, 1 bar, 1/2, 1/4, 1/8, 1/16, 1/32.
pub const SYNC_BEATS: [f64; 8] = [16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25, 0.125];
/// The song position's ticks in a beat (768 a bar).
pub const TICKS_PER_BEAT: f64 = 192.0;

/// The rate knob's default value (0..=127: the middle, 1 cycle a second free, a 1/4 or 1/2 note
/// tied to the tempo) and the amount's (no movement).
pub const RATE_DEFAULT: u8 = 64;
pub const AMOUNT_DEFAULT: u8 = 0;

/// The knobs with an LFO, by the name in their id (`tb303_N.<name>`), left to right.
pub const PARAMS: [&str; 5] = ["cutoff", "reso", "envmod", "decay", "accent"];

/// The synth (0 or 1) of a `tb303_N` section.
fn synth_of_section(sec: &str) -> Option<usize> {
    match sec {
        "tb303_1" => Some(0),
        "tb303_2" => Some(1),
        _ => None,
    }
}

/// The synth and the knob (an index into `PARAMS`) whose LFO a knob `id` is the target of
/// (`tb303_N.cutoff` .. `tb303_N.accent`).
pub fn target_of(id: &str) -> Option<(usize, usize)> {
    let (sec, name) = id.split_once('.')?;
    Some((
        synth_of_section(sec)?,
        PARAMS.iter().position(|p| *p == name)?,
    ))
}

/// The ids of a synth's knob `param` (an index into `PARAMS`), its LFO's RATE and AMOUNT knobs
/// and its mode buttons.
pub fn ids(synth: usize, param: usize) -> [String; 4] {
    let (n, p) = (synth + 1, PARAMS[param]);
    [
        format!("tb303_{n}.{p}"),
        format!("tb303_{n}.{p}_lfo"),
        format!("tb303_{n}.{p}_amt"),
        format!("tb303_{n}.{p}_lfo_mode"),
    ]
}

/// The value a double click sets on an LFO knob (`App::knob`).
pub fn knob_default(id: &str) -> Option<u8> {
    let (sec, name) = id.split_once('.')?;
    synth_of_section(sec)?;
    let (param, which) = name.rsplit_once('_')?;
    if !PARAMS.contains(&param) {
        return None;
    }
    match which {
        "lfo" => Some(RATE_DEFAULT),
        "amt" => Some(AMOUNT_DEFAULT),
        _ => None,
    }
}

/// The LFO's wave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Sine,
    /// Sample & hold, the random sine: a new random height every half cycle, glided to.
    SampleHold,
}

/// The buttons under the knob: tempo sync on or off, and the wave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    pub sync: bool,
    pub shape: Shape,
}

/// The random sine's peak of half cycle `half`: above the middle on the even halves, below on
/// the odd ones, at a random height from half the range to all of it.
fn peak(rng: &mut Rng, half: i64) -> f64 {
    let h = 0.5 + 0.5 * rng.unit();
    if half.rem_euclid(2) == 0 { h } else { -h }
}

/// The rate in cycles a second for the rate knob at `rate` (0..=1) when free.
pub fn free_hz(rate: f64) -> f64 {
    FREE_MIN_HZ * (FREE_MAX_HZ / FREE_MIN_HZ).powf(rate.clamp(0.0, 1.0))
}

/// The cycle's length in beats for the rate knob at `rate` (0..=1) when tied to the tempo.
pub fn sync_beats(rate: f64) -> f64 {
    let i = (rate.clamp(0.0, 1.0) * (SYNC_BEATS.len() - 1) as f64).round() as usize;
    SYNC_BEATS[i]
}

/// The cutoff value sent: `base` (0..=127) moved by `amount` (0..=1) times the wave's `v`
/// (-1..=1), the full amount reaching the whole range.
pub fn modulate(base: u8, amount: f64, v: f64) -> u8 {
    (base as f64 + amount.clamp(0.0, 1.0) * 127.0 * v.clamp(-1.0, 1.0))
        .round()
        .clamp(0.0, 127.0) as u8
}

/// One synth's LFO as it runs: where it is in its cycles, the sample & hold heights, and the
/// knob's value it moves around.
#[derive(Clone, Copy, Debug)]
pub struct Runner {
    /// Cycles run so far (the whole part counts the cycles, the fraction is the phase).
    pub cycles: f64,
    /// The sample & hold: the peak the half cycle starts from and the one it glides to.
    held: (f64, f64),
    /// The half cycle whose peaks `held` are.
    held_cycle: i64,
    rng: Rng,
    /// The knob's value the LFO moves around while it runs (None: not running).
    pub base: Option<u8>,
    /// The last value sent to the engine.
    pub sent: Option<u8>,
}

impl Runner {
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let first = peak(&mut rng, -1);
        let second = peak(&mut rng, 0);
        Self {
            cycles: 0.0,
            held: (first, second),
            held_cycle: 0,
            rng,
            base: None,
            sent: None,
        }
    }

    /// Moves the LFO on by `dt` seconds. Tied to the tempo (`tempo` in BPM * 1000) while the
    /// song plays at `position` (ticks), the cycles follow the song position instead.
    pub fn advance(&mut self, dt: f64, rate: f64, mode: Mode, tempo: i32, position: Option<i32>) {
        if mode.sync {
            let beats = sync_beats(rate);
            match position {
                Some(p) => self.cycles = p.max(0) as f64 / (TICKS_PER_BEAT * beats),
                None => self.cycles += dt * (tempo.max(1) as f64 / 1000.0 / 60.0) / beats,
            }
        } else {
            self.cycles += dt * free_hz(rate);
        }
    }

    /// The wave's height now, -1..=1.
    pub fn value(&mut self, shape: Shape) -> f64 {
        let phase = self.cycles.fract();
        match shape {
            Shape::Sine => (std::f64::consts::TAU * phase).sin(),
            Shape::SampleHold => {
                // two glides a cycle, as the sine goes up and down once each: every half cycle
                // a new peak on the other side, of a random height
                let half = (self.cycles * 2.0).floor() as i64;
                if half != self.held_cycle {
                    // one half on: glide on from where it was; a jump draws two new peaks
                    self.held = if half == self.held_cycle + 1 {
                        (self.held.1, peak(&mut self.rng, half))
                    } else {
                        (peak(&mut self.rng, half - 1), peak(&mut self.rng, half))
                    };
                    self.held_cycle = half;
                }
                let (a, b) = self.held;
                let t = (self.cycles * 2.0).fract();
                let k = (1.0 - (std::f64::consts::PI * t).cos()) / 2.0;
                a + (b - a) * k
            }
        }
    }
}

impl App {
    /// The mode buttons of a synth's knob `param`'s LFO.
    pub fn lfo_mode(&self, synth: usize, param: usize) -> Mode {
        self.lfo_mode[synth][param]
    }

    /// A press on an LFO's mode button: `button` 0 (the note) turns tempo sync on or off, 1
    /// (the wave) flips between the sine and the random sine (sample & hold).
    pub fn press_lfo_mode(&mut self, synth: usize, param: usize, button: usize) {
        let m = &mut self.lfo_mode[synth][param];
        let before = *m;
        match button {
            0 => m.sync = !m.sync,
            _ => {
                m.shape = match m.shape {
                    Shape::Sine => Shape::SampleHold,
                    Shape::SampleHold => Shape::Sine,
                }
            }
        }
        if *m != before {
            self.doc.modified = true;
        }
    }

    /// The value an LFO moves its knob's value around while it runs (the user's), and the value
    /// it sent last; None when it does not run.
    pub fn lfo_base(&self, synth: usize, param: usize) -> Option<u8> {
        self.lfo_run[synth][param].base
    }

    pub fn lfo_sent(&self, synth: usize, param: usize) -> Option<u8> {
        let run = &self.lfo_run[synth][param];
        run.base.and(run.sent)
    }

    /// All LFO knobs and buttons at their defaults, and the LFOs stopped (a new or an opened
    /// song; an .rbsx then sets its own, rbsx.rs).
    pub(crate) fn lfo_reset_knobs(&mut self) {
        let to = |v: u8| behavior::knob_from_engine(v as i32);
        for synth in 0..2 {
            for param in 0..PARAMS.len() {
                let [_, rate, amount, _] = ids(synth, param);
                self.panel.put_knob(&rate, to(RATE_DEFAULT));
                self.panel.put_knob(&amount, to(AMOUNT_DEFAULT));
                self.lfo_mode[synth][param] = Mode::default();
                self.lfo_run[synth][param].base = None;
                self.lfo_run[synth][param].sent = None;
            }
        }
    }

    /// The user moved knob `id`: when an LFO runs on it, its value is the new one to move
    /// around.
    pub(crate) fn lfo_knob_moved(&mut self, id: &str) {
        if let Some((k, p)) = target_of(id)
            && self.lfo_run[k][p].base.is_some()
        {
            self.lfo_run[k][p].base = self.panel.value(id);
        }
    }

    /// The knob position of the value an LFO sent last for knob `id`, while it runs on it: the
    /// arc shows it (the pointer keeps the user's value).
    pub(crate) fn lfo_arc(&self, id: &str) -> Option<i32> {
        let (k, p) = target_of(id)?;
        self.lfo_sent(k, p)
            .map(|v| behavior::knob_from_engine(v as i32))
    }

    /// The mode buttons under a knob: a click on a third of them presses that button.
    pub(crate) fn lfo_mode_control(&mut self, ui: &mut Ui, c: &Ctl, r: Rect) {
        let Some((k, p)) = target_of(c.id.strip_suffix("_lfo_mode").unwrap_or("")) else {
            return;
        };
        let resp = ui.interact(r, egui::Id::unique(("lfo_mode", &c.id)), Sense::click());
        let pressed = ui.input(|i| i.pointer.button_pressed(PointerButton::Primary));
        if pressed
            && resp.contains_pointer()
            && let Some(at) = ui.input(|i| i.pointer.press_origin())
        {
            let half = ((at.x - r.min.x) / r.width() * 2.0).floor().clamp(0.0, 1.0) as usize;
            self.press_lfo_mode(k, p, half);
        }
        let m = self.lfo_mode[k][p];
        let hold = m.shape == Shape::SampleHold;
        let name = format!("lfo_mode_{}{}", m.sync as u8, hold as u8);
        app::paint_part(ui, &name, r);
    }

    /// One step of every LFO, after the panel took the engine's values: a running LFO sends its
    /// knob's moved value to the engine and puts the user's value back on the knob. An LFO
    /// whose amount goes to 0 stops and sends the user's value back.
    pub(crate) fn lfo_step(&mut self, ctx: &Context, now: f64) {
        let dt = (now - self.lfo_time.unwrap_or(now)).clamp(0.0, 0.25);
        self.lfo_time = Some(now);
        let mut running = false;
        for k in 0..2 {
            for p in 0..PARAMS.len() {
                running |= self.lfo_step_one(k, p, dt);
            }
        }
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    /// [`App::lfo_step`] for synth `k`'s knob `p`: whether its LFO runs.
    fn lfo_step_one(&mut self, k: usize, p: usize, dt: f64) -> bool {
        let [knob, rate, amount, _] = ids(k, p);
        let unit = |v: Option<u8>, d: u8| v.unwrap_or(d) as f64 / 127.0;
        let amount = unit(self.panel.value(&amount), AMOUNT_DEFAULT);
        let rate = unit(self.panel.value(&rate), RATE_DEFAULT);
        let Some(link) = &mut self.link else {
            return false;
        };
        let run = &mut self.lfo_run[k][p];
        if amount <= 0.0 {
            if let Some(b) = run.base.take() {
                link.send_value(&knob, b);
                self.panel.adopt(&knob, b as i32);
            }
            run.sent = None;
            return false;
        }
        let base = match run.base {
            Some(b) => b,
            None => {
                let b = self.panel.value(&knob).unwrap_or(0);
                run.base = Some(b);
                b
            }
        };
        let t = &self.panel.transport;
        let position = t.playing.then_some(t.position);
        let mode = self.lfo_mode[k][p];
        run.advance(dt, rate, mode, t.tempo, position);
        let v = modulate(base, amount, run.value(mode.shape));
        if run.sent != Some(v) {
            link.send_value(&knob, v);
            run.sent = Some(v);
        }
        // the knob shows the user's value, not the engine's moved one
        self.panel.adopt(&knob, base as i32);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_free_rate_runs_from_slow_to_fast() {
        assert!(close(free_hz(0.0), FREE_MIN_HZ));
        assert!(close(free_hz(1.0), FREE_MAX_HZ));
        assert!(close(free_hz(0.5), (FREE_MIN_HZ * FREE_MAX_HZ).sqrt()));
        assert!(free_hz(0.3) < free_hz(0.6));
    }

    #[test]
    fn the_tempo_rate_takes_one_of_eight_lengths() {
        assert_eq!(sync_beats(0.0), 16.0);
        assert_eq!(sync_beats(1.0), 0.125);
        assert_eq!(sync_beats(4.0 / 7.0), 1.0);
        // a quarter note at 120 BPM is half a second
        let mut r = Runner::new(1);
        let mode = Mode {
            sync: true,
            shape: Shape::Sine,
        };
        r.advance(0.5, 4.0 / 7.0, mode, 120_000, None);
        assert!(close(r.cycles, 1.0));
        // while the song plays the cycles follow its position: a bar of 1/4 notes is 4
        r.advance(0.0, 4.0 / 7.0, mode, 120_000, Some(768));
        assert!(close(r.cycles, 4.0));
    }

    #[test]
    fn the_sine_goes_up_and_down_once_a_cycle() {
        let mut r = Runner::new(1);
        let mode = Mode::default();
        let rate = 0.0; // 0.05 Hz: 5 s to a quarter cycle
        assert!(close(r.value(Shape::Sine), 0.0));
        r.advance(5.0, rate, mode, 0, None);
        assert!(close(r.value(Shape::Sine), 1.0));
        r.advance(10.0, rate, mode, 0, None);
        assert!(close(r.value(Shape::Sine), -1.0));
    }

    #[test]
    fn sample_and_hold_glides_to_a_new_random_height_each_half_cycle() {
        let mut r = Runner::new(7);
        let mode = Mode {
            sync: false,
            shape: Shape::SampleHold,
        };
        let mut starts = vec![];
        for _ in 0..40 {
            // the start of each cycle, then its middle: always within -1..=1, and smooth
            starts.push(r.value(Shape::SampleHold));
            r.advance(10.0, 0.0, mode, 0, None); // half a cycle at 0.05 Hz
            let mid = r.value(Shape::SampleHold);
            assert!((-1.0..=1.0).contains(&mid));
            r.advance(10.0, 0.0, mode, 0, None);
        }
        // the heights differ from cycle to cycle
        assert!(starts.windows(2).filter(|w| !close(w[0], w[1])).count() > 30);
        // a cycle ends where the next starts: no jump
        let mut r = Runner::new(3);
        r.advance(19.999_999, 0.0, mode, 0, None);
        let before = r.value(Shape::SampleHold);
        r.advance(0.000_002, 0.0, mode, 0, None);
        assert!((r.value(Shape::SampleHold) - before).abs() < 1e-3);
    }

    /// How far a wave travels (up and down added) and how often it turns, over 20 cycles at
    /// 0.05 Hz in small steps.
    fn travel(shape: Shape) -> (f64, usize) {
        let mut r = Runner::new(7);
        let mode = Mode { sync: false, shape };
        let (mut dist, mut turns, mut last, mut dir) = (0.0, 0, r.value(shape), 0.0f64);
        for _ in 0..20 * 400 {
            r.advance(20.0 / 400.0, 0.0, mode, 0, None);
            let v = r.value(shape);
            let d = v - last;
            if d.abs() > 1e-12 {
                if dir != 0.0 && d.signum() != dir.signum() {
                    turns += 1;
                }
                dir = d;
            }
            dist += d.abs();
            last = v;
        }
        (dist, turns)
    }

    /// (this fork) The random sine moves about as fast as the sine: it turns twice a cycle and
    /// travels most of the sine's way (it was one random glide a cycle, several times slower).
    #[test]
    fn the_random_sine_is_about_as_fast_as_the_sine() {
        let (sine, sine_turns) = travel(Shape::Sine);
        let (random, random_turns) = travel(Shape::SampleHold);
        assert!((39..=41).contains(&sine_turns), "{sine_turns}");
        assert!((39..=41).contains(&random_turns), "{random_turns}");
        assert!(
            random > 0.6 * sine && random < 1.1 * sine,
            "{random} vs {sine}"
        );
    }

    #[test]
    fn the_amount_moves_the_value_around_the_knob_within_the_range() {
        assert_eq!(modulate(64, 0.0, 1.0), 64);
        assert_eq!(modulate(64, 0.5, 1.0), 127);
        assert_eq!(modulate(64, 0.25, -1.0), 32);
        assert_eq!(modulate(100, 1.0, 1.0), 127);
        assert_eq!(modulate(10, 1.0, -1.0), 0);
    }
}
