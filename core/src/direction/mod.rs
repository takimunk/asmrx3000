//! Direction: the scene's world keeps changing on its own, non-stop, and the listener steers
//! it instead of setting it. Where the director (director/) plays the animals, the direction
//! plays the place: the clock, the wind and where it blows from, rain, thunder, temperature,
//! humidity, how much the birds sing and how full the brook runs. It moves the very controls
//! the sliders show, the way automation moves the faders of a mixing desk.
//!
//! Four layers, slowest first:
//! * **Clock.** Scene time flows (`direction.clock` scene hours per real hour, 1 = real time).
//! * **Climate of the day.** Temperature and humidity follow the daily cycle around the scene's
//!   values (the same curve the controls follow on their own), the wind is calm at night and
//!   strongest in the afternoon, when the sun mixes the air.
//! * **Episodes.** Weather spells with their own inner course (catalogue.rs, episodes.json):
//!   a thunderstorm is distant thunder, the calm before the storm, the gust front (wind
//!   shift, cold outflow), the cell overhead, trailing rain, the storm moving away; then the
//!   time after the rain, when the birds sing more. Which spell comes next is drawn by weight
//!   × hour × season × steer × novelty, among the ones that may physically follow (rain sets
//!   in from an overcast sky, not from a clear one). The next spell is known in advance, so it
//!   can be shown as a forecast.
//! * **Weather texture.** Slow Ornstein–Uhlenbeck wander on wind, rain, birds and wind
//!   direction inside every segment, scaled by `direction.change`.
//!
//! Plus the slow consequences: the brook swells after heavy rain with a lag (two linear
//! reservoirs, ~15 and ~90 min) and recedes over hours.
//!
//! **Steering.** `direction.weather` (fair ↔ wet), `.energy` (still ↔ lively) and `.change`
//! (steady ↔ eventful) bias both the choice of spells and, at once, the values (stronger wind,
//! more birds, heavier rain). `direction.cue` asks for a spell: the current one wraps up
//! naturally (a downpour tapers off first) and, if the asked one cannot follow directly, a
//! short bridge comes between (overcast before rain). `direction.amount` scales the whole
//! direction (0 = off).
//!
//! **The listener has the last word.** A control the user (slider or prompt) sets is left to
//! them for `direction.hold` minutes, then taken back over ~2.5 min; double-click (reset) hands
//! it back at once. The clock adopts a time set by hand and goes on from there. Repeated
//! corrections the same way are remembered as a mild, slowly fading bias.
//!
//! Real-time safe after [`Direction::new`]: fixed arrays, indices into the catalogue.

pub mod bridge;
pub mod catalogue;
#[cfg(test)]
mod tests;

pub use catalogue::{DirectionDesc, Episode, Segment, Span, Step, BUILTIN_NAMES};

use crate::rng::Rng;
use crate::scene::{SceneDesc, Season};
use crate::sources::common::{smoothstep, wind_speed_ms, Ou};

/// Channels the direction drives, in this order (`CONTROL_IDS`, `CHANNEL_NAMES`).
pub const HOUR: usize = 0;
pub const WIND: usize = 1;
pub const WIND_DIR: usize = 2;
pub const GUST: usize = 3;
pub const RAIN: usize = 4;
pub const STORM: usize = 5;
pub const TEMP: usize = 6;
pub const HUMIDITY: usize = 7;
pub const BIRDS: usize = 8;
pub const WATER: usize = 9;
pub const CHANNELS: usize = 10;

pub const CHANNEL_NAMES: [&str; CHANNELS] = ["hour", "wind", "wind_direction", "gustiness", "rain", "storm", "temperature", "humidity", "birds", "water"];

/// The control each channel moves.
pub const CONTROL_IDS: [u32; CHANNELS] = {
    use crate::controls::id::*;
    [ENV_TIME, WIND_SPEED, ENV_WIND_DIRECTION, WIND_GUSTINESS, RAIN_RATE, RAIN_STORM, ENV_TEMPERATURE, ENV_HUMIDITY, BIRDS_ACTIVITY, WATER_FLOW]
};

/// The direction plans in steps of this many seconds.
pub const TICK: f32 = 0.1;
/// A channel handed back after a hold glides from the user's value to the direction's.
pub const RETURN_S: f32 = 150.0;
/// Time constant of each channel towards its target, s (a segment's `ease` overrides the
/// fast ones: wind, gustiness, rain, storm, birds).
const TAU: [f32; CHANNELS] = [0.0, 45.0, 90.0, 60.0, 60.0, 90.0, 600.0, 300.0, 60.0, 0.0];
/// Remembered corrections fade with this time constant, s.
const LEARN_TAU: f32 = 7200.0;

/// What the `direction.*` controls ask for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steer {
    /// 0 = off .. 1 = full: scales every deviation from the scene's own values.
    pub amount: f32,
    /// −1 fair .. +1 wet and stormy.
    pub weather: f32,
    /// −1 still .. +1 lively.
    pub energy: f32,
    /// 0 steady .. 1 eventful.
    pub change: f32,
    /// Scene hours per real hour (0 = the clock stands).
    pub clock: f32,
    /// Seconds a value set by hand is left to the user.
    pub hold_s: f32,
}

impl Steer {
    pub fn from_desc(d: &DirectionDesc) -> Self {
        Self { amount: 1.0, weather: d.weather, energy: d.energy, change: d.change, clock: d.clock, hold_s: d.hold_min * 60.0 }
    }
}

/// A request to change course.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// This episode (catalogue index), as soon as it can naturally come.
    Episode(usize),
    /// Wrap the current episode up and go on to the planned next one.
    Next,
}

/// One planning step's input.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub steer: Steer,
    /// Effective value of each channel's control right now.
    pub current: [f32; CHANNELS],
    /// The user set the channel since the previous step (or a glide of theirs is moving it).
    pub touched: [bool; CHANNELS],
    /// The user handed the channel back (reset).
    pub released: [bool; CHANNELS],
    pub cue: Option<Cue>,
}

impl Input {
    pub fn new(steer: Steer, current: [f32; CHANNELS]) -> Self {
        Self { steer, current, touched: [false; CHANNELS], released: [false; CHANNELS], cue: None }
    }
}

/// What to do with the controls after a step.
#[derive(Clone, Copy, Debug, Default)]
pub struct Output {
    pub value: [f32; CHANNELS],
    /// Write `value` into the channel's control.
    pub write: [bool; CHANNELS],
    /// Hand the control back to its own curve (the direction was switched off).
    pub release: [bool; CHANNELS],
    /// How lively the animals' encounters should be (1 = as directed by the scene).
    pub liveliness: f32,
}

/// Where the direction stands, for the UI and the prompt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Info {
    pub active: bool,
    pub episode: usize,
    pub segment: usize,
    pub segments: usize,
    pub segment_left_s: f32,
    pub episode_left_s: f32,
    pub next: usize,
    /// The next episode was asked for (cue), not drawn.
    pub cued: bool,
    pub hour: f32,
    pub liveliness: f32,
    /// Effective steer: the controls plus remembered corrections.
    pub weather: f32,
    pub energy: f32,
    pub change: f32,
    /// Bit per channel left to the user right now.
    pub held: u32,
}

/// The scene's own values: what the climate is anchored to and `amount` blends from.
#[derive(Clone, Copy, Debug)]
struct Base {
    hour: f32,
    temp: f32,
    humidity: f32,
    wind_ms: f32,
    wind_dir: f32,
    birds: f32,
    water: f32,
    season: Season,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Auto,
    Held(f32),
    Returning { from: f32, t: f32 },
}

/// The current segment's drawn targets.
#[derive(Clone, Copy, Debug, Default)]
struct Targets {
    wind: Option<f32>,
    gust: Option<f32>,
    rain: Option<f32>,
    storm: Option<f32>,
    temp: Option<f32>,
    humidity: Option<f32>,
    birds: Option<f32>,
    ease: Option<f32>,
}

pub struct Direction {
    eps: Vec<Episode>,
    /// `follows[i * n + j]`: episode j may come after i.
    follows: Vec<bool>,
    then: Vec<Option<usize>>,
    script: Vec<(usize, Option<f32>)>,
    script_pos: usize,
    repeat: bool,
    rng: Rng,
    base: Base,
    started: bool,
    acc: f32,
    hour: f64,
    cur: usize,
    seg: usize,
    seg_left: f32,
    /// Duration scale of the current episode (script minutes, a bridge, a steady direction).
    scale: f32,
    targets: Targets,
    /// The current episode is wrapping up for a cue: its last segment, short, then the next.
    wrapping: bool,
    next: usize,
    next_scale: f32,
    next_fixed: bool,
    cued: bool,
    /// After a bridge: the episode asked for.
    after_bridge: Option<usize>,
    since: Vec<f32>,
    auto: [f32; CHANNELS],
    mode: [Mode; CHANNELS],
    ou: [Ou; 4],
    wind_dir: f32,
    dir_ou: Ou,
    runoff: [f32; 2],
    learned_weather: f32,
    learned_energy: f32,
    planned_steer: Steer,
    liveliness: f32,
    bird_mult: f32,
    out: Output,
}

const OU_WIND: usize = 0;
const OU_RAIN: usize = 1;
const OU_BIRDS: usize = 2;
const OU_TEMP: usize = 3;

/// Daily cycle shape: 1 at 15:00, −1 at 03:00 (as in controls.rs).
fn diurnal(hour: f32) -> f32 {
    (core::f32::consts::TAU * (hour - 15.0) / 24.0).cos()
}

/// Mean wind relative to the daily mean: calm nights (a stable surface layer), strongest
/// mid-afternoon when the sun mixes the boundary layer.
fn wind_cycle(hour: f32) -> f32 {
    0.65 + 0.35 * diurnal(hour)
}

/// Gustiness the wind macro implies on its own (controls.rs).
fn gust_of(wind_ms: f32) -> f32 {
    1.15 - 0.3 * smoothstep(2.0, 16.0, wind_ms)
}

/// Thunder that heavy rain implies on its own (controls.rs).
fn storm_of(rain: f32) -> f32 {
    0.9 * smoothstep(10.0, 40.0, rain)
}

fn in_hours(h: f32, [a, b]: [f32; 2]) -> bool {
    if a <= b {
        h >= a && h <= b
    } else {
        h >= a || h <= b
    }
}

/// Shortest signed turn from `a` to `b`, degrees.
fn turn(a: f32, b: f32) -> f32 {
    (b - a + 540.0).rem_euclid(360.0) - 180.0
}

impl Direction {
    /// The scene's direction, if it has one.
    pub fn new(desc: &SceneDesc) -> Result<Option<Self>, String> {
        let Some(d) = desc.direction.as_ref() else { return Ok(None) };
        let eps = d.catalogue()?;
        let n = eps.len();
        let index = |name: &str| eps.iter().position(|e| e.name == name);
        let mut follows = vec![false; n * n];
        for (j, e) in eps.iter().enumerate() {
            for i in 0..n {
                follows[i * n + j] = e.after.is_empty() || e.after.iter().any(|a| a == &eps[i].name);
            }
        }
        let then = eps.iter().map(|e| e.then.as_deref().and_then(index)).collect();
        let script = d.script.iter().filter_map(|s| index(&s.episode).map(|i| (i, s.minutes))).collect();
        let env = &desc.environment;
        let base = Base {
            hour: env.time_of_day.rem_euclid(24.0),
            temp: env.temperature_c,
            humidity: env.humidity,
            wind_ms: wind_speed_ms(env.initial_wind().unwrap_or(0.45)),
            wind_dir: 270.0,
            birds: env.birds.unwrap_or(0.6),
            water: env.water.unwrap_or(0.6),
            season: env.season,
        };
        let start = d.start.as_deref().and_then(index);
        let mut me = Self {
            follows,
            then,
            script,
            script_pos: 0,
            repeat: d.repeat,
            rng: Rng::new(desc.seed ^ 0xD12E_C710),
            base,
            started: false,
            acc: 0.0,
            hour: base.hour as f64,
            cur: 0,
            seg: 0,
            seg_left: 0.0,
            scale: 1.0,
            targets: Targets::default(),
            wrapping: false,
            next: 0,
            next_scale: 1.0,
            next_fixed: false,
            cued: false,
            after_bridge: None,
            since: vec![f32::INFINITY; n],
            auto: [0.0; CHANNELS],
            mode: [Mode::Auto; CHANNELS],
            ou: [Ou::default(); 4],
            wind_dir: base.wind_dir,
            dir_ou: Ou::default(),
            runoff: [0.0; 2],
            learned_weather: 0.0,
            learned_energy: 0.0,
            planned_steer: Steer::from_desc(d),
            liveliness: 1.0,
            bird_mult: 1.0,
            out: Output { liveliness: 1.0, ..Default::default() },
            eps,
        };
        // The first episode: the script's, the scene's choice, or a gentle one for the hour.
        let steer = Steer::from_desc(d);
        let first = if let Some(&(i, m)) = me.script.first() {
            me.script_pos = 1;
            me.scale = me.script_scale(i, m);
            i
        } else if let Some(i) = start {
            i
        } else {
            me.draw(None, &steer, true)
        };
        me.begin(first, &steer);
        // Start somewhere inside it, not always at its very beginning.
        me.seg_left *= me.rng.range(0.3, 1.0);
        Ok(Some(me))
    }

    pub fn episodes(&self) -> &[Episode] {
        &self.eps
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.eps.iter().position(|e| e.name == name)
    }

    /// Scene time on the direction's clock, hours 0..24.
    pub fn hour(&self) -> f32 {
        self.hour as f32
    }

    pub fn output(&self) -> &Output {
        &self.out
    }

    fn may_follow(&self, i: usize, j: usize) -> bool {
        self.follows[i * self.eps.len() + j]
    }

    fn effective(&self, s: &Steer) -> (f32, f32, f32) {
        ((s.weather + self.learned_weather).clamp(-1.0, 1.0), (s.energy + self.learned_energy).clamp(-1.0, 1.0), s.change.clamp(0.0, 1.0))
    }

    /// Scale that makes episode `i` last `minutes` in all (its mean length otherwise).
    fn script_scale(&self, i: usize, minutes: Option<f32>) -> f32 {
        let Some(m) = minutes else { return 1.0 };
        let mean: f32 = self.eps[i].segments.iter().map(|s| s.minutes.map_or(20.0, |r| 0.5 * (r.0[0] + r.0[1]))).sum();
        (m / mean.max(0.1)).clamp(0.02, 50.0)
    }

    /// Draws the episode to follow `from` (None: the first one, gentle only).
    fn draw(&mut self, from: Option<usize>, steer: &Steer, gentle: bool) -> usize {
        let (w_axis, e_axis, c_axis) = self.effective(steer);
        let hour = self.hour as f32;
        let n = self.eps.len();
        let mut total = 0.0f32;
        let mut pick = None;
        for j in 0..n {
            let e = &self.eps[j];
            if from == Some(j) && n > 1 {
                continue;
            }
            if let Some(i) = from {
                if !self.may_follow(i, j) {
                    continue;
                }
            }
            if gentle && e.drama > 0.5 {
                continue;
            }
            let mut w = e.weight;
            if let Some(h) = e.hours {
                w *= if in_hours(hour, h) { 1.0 } else { 0.12 };
            }
            if !e.seasons.is_empty() && !e.seasons.contains(&self.base.season) {
                // Out of season; asked for wetter weather, a long rain still comes now and then.
                w *= (1.0f32 / 6.0).powf(1.0 - 0.5 * (w_axis * e.wet).max(0.0));
            }
            w *= (3.0 * w_axis * e.wet).exp() * (2.5 * e_axis * e.energy).exp() * (2.5 * (c_axis - 0.4) * (e.drama - 0.4)).exp();
            // Novelty: a spell seen within the last hour or so is less likely again.
            w *= 0.3 + 0.7 * smoothstep(0.0, 3600.0, self.since[j]);
            if w <= 0.0 {
                continue;
            }
            total += w;
            if self.rng.uniform() * total < w {
                pick = Some(j);
            }
        }
        // A catalogue with nowhere to go from here: anything else, then anything.
        pick.unwrap_or_else(|| {
            let i = from.unwrap_or(0);
            (1..=n).map(|k| (i + k) % n).find(|&j| self.eps[j].weight > 0.0).unwrap_or(i)
        })
    }

    /// How the next episode is decided: forced (`then`, a bridge's target, the script) or drawn.
    fn plan_next(&mut self, steer: &Steer) {
        self.next_scale = 1.0;
        self.cued = false;
        if let Some(j) = self.after_bridge.take() {
            self.next = j;
            self.next_fixed = true;
            self.cued = true;
            return;
        }
        if let Some(j) = self.then[self.cur] {
            self.next = j;
            self.next_fixed = true;
            return;
        }
        if self.script_pos >= self.script.len() && self.repeat && !self.script.is_empty() {
            self.script_pos = 0;
        }
        if let Some(&(j, m)) = self.script.get(self.script_pos) {
            self.script_pos += 1;
            self.next = j;
            self.next_scale = self.script_scale(j, m);
            self.next_fixed = true;
            return;
        }
        self.next = self.draw(Some(self.cur), steer, false);
        self.next_fixed = false;
        self.planned_steer = *steer;
    }

    fn begin(&mut self, i: usize, steer: &Steer) {
        self.cur = i;
        self.wrapping = false;
        // A steady direction lets the calm spells last; an eventful one moves on sooner.
        if self.eps[i].drama < 0.4 {
            self.scale *= 1.6 - steer.change.clamp(0.0, 1.0);
        }
        self.start_segment(0);
        self.plan_next(steer);
    }

    fn start_segment(&mut self, k: usize) {
        self.seg = k;
        let s = &self.eps[self.cur].segments[k];
        let draw = |r: &Option<Span>, rng: &mut Rng| r.map(|r| rng.range(r.0[0], r.0[1].max(r.0[0])));
        let rng = &mut self.rng;
        let minutes = draw(&s.minutes, rng).unwrap_or(20.0);
        self.targets = Targets {
            wind: draw(&s.wind, rng),
            gust: draw(&s.gust, rng),
            rain: draw(&s.rain, rng),
            storm: draw(&s.storm, rng),
            temp: draw(&s.temp, rng),
            humidity: draw(&s.humidity, rng),
            birds: draw(&s.birds, rng),
            ease: s.ease,
        };
        if let Some(v) = draw(&s.veer, rng) {
            self.wind_dir = (self.wind_dir + v).rem_euclid(360.0);
        }
        self.seg_left = (minutes * 60.0 * self.scale).max(TICK);
    }

    /// The episode ends: on to the planned next one.
    fn end_episode(&mut self, steer: &Steer) {
        self.since[self.cur] = 0.0;
        let next = self.next;
        self.scale = self.next_scale;
        self.begin(next, steer);
    }

    fn advance_segment(&mut self, steer: &Steer) {
        let last = self.eps[self.cur].segments.len() - 1;
        if self.seg >= last {
            self.end_episode(steer);
        } else if self.wrapping {
            // Wrapping up: straight to the closing stretch (a downpour tapers off), short.
            let s = self.scale;
            self.scale *= 0.4;
            self.start_segment(last);
            self.scale = s;
            self.seg_left = self.seg_left.min(180.0);
        } else {
            self.start_segment(self.seg + 1);
        }
    }

    /// Change course towards episode `j`: the current spell wraps up; if `j` cannot follow
    /// it directly, a short bridge comes first (overcast before rain sets in).
    fn cue(&mut self, j: usize, steer: &Steer) {
        if j == self.cur && !self.wrapping {
            return;
        }
        let bridge = if self.may_follow(self.cur, j) {
            None
        } else {
            // The likeliest episode that can follow now and lead to `j`.
            let n = self.eps.len();
            (0..n)
                .filter(|&b| b != self.cur && b != j && self.may_follow(self.cur, b) && self.may_follow(b, j))
                .max_by(|&a, &b| self.eps[a].weight.total_cmp(&self.eps[b].weight))
        };
        match bridge {
            Some(b) => {
                self.next = b;
                self.next_scale = 0.3;
                self.after_bridge = Some(j);
            }
            None => {
                self.next = j;
                self.next_scale = 1.0;
                self.after_bridge = None;
            }
        }
        self.next_fixed = true;
        self.cued = true;
        self.wrap_up(steer);
    }

    /// Bring the current spell to its natural close within a minute or two.
    fn wrap_up(&mut self, _steer: &Steer) {
        self.wrapping = true;
        let close = self.rng.range(30.0, 90.0);
        self.seg_left = self.seg_left.min(close);
    }

    /// One planning step (`TICK` seconds, or less on the first call). Returns what to write.
    pub fn step(&mut self, dt: f32, input: &Input) -> &Output {
        let s = input.steer;
        self.out.write = [false; CHANNELS];
        self.out.release = [false; CHANNELS];
        if s.amount <= 0.0 {
            if self.started {
                // Switched off: the children go back to their own curves (temperature and
                // humidity follow the clock again, storm follows the rain).
                self.out.release = [false, false, false, true, false, true, true, true, false, false];
                self.started = false;
            }
            self.out.liveliness = 1.0;
            return &self.out;
        }
        if !self.started {
            // Pick up from whatever is heard now: no jumps when the direction starts.
            self.auto = input.current;
            self.hour = input.current[HOUR].rem_euclid(24.0) as f64;
            self.wind_dir = input.current[WIND_DIR];
            self.mode = [Mode::Auto; CHANNELS];
            self.started = true;
        }
        let (w_axis, e_axis, c_axis) = self.effective(&s);

        // The listener's hand.
        for c in 0..CHANNELS {
            if c == HOUR {
                if input.touched[c] {
                    self.hour = input.current[c].rem_euclid(24.0) as f64;
                }
                continue;
            }
            if input.released[c] {
                self.mode[c] = Mode::Returning { from: input.current[c], t: 0.0 };
            } else if input.touched[c] {
                if matches!(self.mode[c], Mode::Auto | Mode::Returning { .. }) {
                    self.learn(c, input.current[c] - self.out.value[c]);
                }
                self.mode[c] = Mode::Held(s.hold_s.max(1.0));
            }
            self.mode[c] = match self.mode[c] {
                Mode::Held(left) if left - dt <= 0.0 => Mode::Returning { from: input.current[c], t: 0.0 },
                Mode::Held(left) => Mode::Held(left - dt),
                Mode::Returning { t, .. } if t + dt >= RETURN_S => Mode::Auto,
                Mode::Returning { from, t } => Mode::Returning { from, t: t + dt },
                m => m,
            };
        }
        let fade = (-dt / LEARN_TAU).exp();
        self.learned_weather *= fade;
        self.learned_energy *= fade;

        // Clock.
        if self.mode[HOUR] == Mode::Auto {
            self.hour = (self.hour + (dt * s.clock.max(0.0)) as f64 / 3600.0).rem_euclid(24.0);
        }
        let hour = self.hour as f32;

        // Course: cues, segments, episodes.
        for t in self.since.iter_mut() {
            *t += dt;
        }
        match input.cue {
            Some(Cue::Episode(j)) if j < self.eps.len() => self.cue(j, &s),
            Some(Cue::Next) => {
                self.cued = true;
                self.wrap_up(&s);
            }
            _ => {}
        }
        // A clearly different steer redraws the forecast (a cue or a script stays).
        let p = self.planned_steer;
        if !self.next_fixed && ((p.weather - s.weather).abs() > 0.15 || (p.energy - s.energy).abs() > 0.15 || (p.change - s.change).abs() > 0.15) {
            self.next = self.draw(Some(self.cur), &s, false);
            self.planned_steer = s;
        }
        self.seg_left -= dt;
        if self.seg_left <= 0.0 {
            self.advance_segment(&s);
        }

        // Targets: the day's climate, the segment, the steer, the texture.
        let b = self.base;
        let vary = 0.4 + 1.2 * c_axis;
        let rng = &mut self.rng;
        let ou_wind = self.ou[OU_WIND].step(dt, 180.0, 0.18 * vary, rng);
        let ou_rain = self.ou[OU_RAIN].step(dt, 240.0, 0.3 * vary, rng);
        let ou_birds = self.ou[OU_BIRDS].step(dt, 300.0, 0.1 * vary, rng);
        let ou_temp = self.ou[OU_TEMP].step(dt, 900.0, 0.4, rng);
        let dir_wander = self.dir_ou.step(dt, 1200.0, 12.0 * vary, rng);
        let t = self.targets;
        let clim_temp = b.temp + 5.0 * (diurnal(hour) - diurnal(b.hour));
        let clim_hum = (b.humidity - 0.15 * (diurnal(hour) - diurnal(b.hour))).clamp(0.0, 1.0);
        // The scene's wind is its typical daytime wind (whatever hour it starts at).
        let clim_wind = b.wind_ms * wind_cycle(hour) / wind_cycle(12.0);
        // A fast segment (a gust front) is one event, not weather texture: no wander on it.
        let texture = if t.ease.is_some() { 1.0 } else { (1.0 + ou_wind).clamp(0.6, 1.3) };
        let wind = (t.wind.unwrap_or(clim_wind) * (0.4 * e_axis).exp() * texture).clamp(0.0, 30.0);
        let rain = (t.rain.unwrap_or(0.0) * (0.5 * w_axis).exp() * (1.0 + ou_rain).clamp(0.3, 2.0)).clamp(0.0, 100.0);
        self.bird_mult = t.birds.unwrap_or(1.0);
        let mut target = [0.0; CHANNELS];
        target[WIND] = wind;
        target[GUST] = t.gust.unwrap_or_else(|| gust_of(wind));
        target[RAIN] = rain;
        target[STORM] = t.storm.unwrap_or_else(|| storm_of(rain)) * (0.5 * w_axis).exp().min(1.5);
        target[TEMP] = clim_temp + t.temp.unwrap_or(0.0) + ou_temp;
        target[HUMIDITY] = t.humidity.unwrap_or(clim_hum + 0.04 * w_axis).clamp(0.0, 1.0);
        target[BIRDS] = (b.birds * self.bird_mult * (1.0 + 0.3 * e_axis) * (1.0 + ou_birds)).clamp(0.0, 1.0);
        for c in [WIND, GUST, RAIN, STORM, TEMP, HUMIDITY, BIRDS] {
            let fast = matches!(c, WIND | GUST | RAIN | STORM | BIRDS);
            let tau = match t.ease {
                Some(e) if fast => e,
                Some(e) if c == TEMP => TAU[c].min(e * 20.0),
                _ => TAU[c],
            };
            self.auto[c] += (target[c] - self.auto[c]) * (1.0 - (-dt / tau).exp());
        }
        self.auto[STORM] = self.auto[STORM].clamp(0.0, 1.0);
        // Wind direction: the segment's veer plus a slow wander, turning the short way round,
        // and over hours back towards the prevailing wind.
        self.wind_dir = (self.wind_dir + turn(self.wind_dir, b.wind_dir) * dt / 10_800.0).rem_euclid(360.0);
        let goal = (self.wind_dir + dir_wander).rem_euclid(360.0);
        let d = turn(self.auto[WIND_DIR], goal);
        self.auto[WIND_DIR] = (self.auto[WIND_DIR] + d * (1.0 - (-dt / TAU[WIND_DIR]).exp())).rem_euclid(360.0);
        // The brook: rain heard (the user's too) runs off through two linear reservoirs.
        let k1 = 1.0 - (-dt / 900.0).exp();
        let k2 = 1.0 - (-dt / 5400.0).exp();
        self.runoff[0] += (input.current[RAIN].max(0.0) - self.runoff[0]) * k1;
        self.runoff[1] += (self.runoff[0] - self.runoff[1]) * k2;
        self.auto[WATER] = (b.water + 0.35 * (1.0 - (-self.runoff[1] / 4.0).exp())).clamp(0.0, 1.0);

        // Out: blended by `amount` from the scene's own values, unless the user holds it.
        let a = s.amount.clamp(0.0, 1.0);
        let own = [b.hour, b.wind_ms, b.wind_dir, gust_of(b.wind_ms), 0.0, 0.0, clim_temp, clim_hum, b.birds, b.water];
        for c in 0..CHANNELS {
            let v = match c {
                HOUR => self.hour as f32,
                WIND_DIR => (own[c] + a * turn(own[c], self.auto[c])).rem_euclid(360.0),
                _ => own[c] + a * (self.auto[c] - own[c]),
            };
            let (v, write) = match self.mode[c] {
                Mode::Auto => (v, true),
                Mode::Held(_) => (input.current[c], false),
                Mode::Returning { from, t } => {
                    let k = smoothstep(0.0, RETURN_S, t);
                    if c == WIND_DIR {
                        ((from + k * turn(from, v)).rem_euclid(360.0), true)
                    } else {
                        (from + k * (v - from), true)
                    }
                }
            };
            self.out.value[c] = v;
            self.out.write[c] = write;
        }
        // Encounters follow the birds' mood: fewer fly-bys in a downpour, more after it.
        self.liveliness = (self.bird_mult.max(0.05).powf(0.8) * (0.5 * e_axis).exp()).clamp(0.2, 3.0);
        self.out.liveliness = 1.0 + a * (self.liveliness - 1.0);
        &self.out
    }

    /// The user corrected channel `c` by `delta` (their value − the direction's): a mild,
    /// fading bias the same way (keeps turning the rain down → a fairer direction).
    fn learn(&mut self, c: usize, delta: f32) {
        let sign = if delta > 0.0 { 1.0 } else if delta < 0.0 { -1.0 } else { return };
        match c {
            RAIN | STORM => self.learned_weather = (self.learned_weather + 0.1 * sign).clamp(-0.4, 0.4),
            WIND | BIRDS => self.learned_energy = (self.learned_energy + 0.07 * sign).clamp(-0.4, 0.4),
            _ => {}
        }
    }

    /// Advance by `dt` seconds of audio, planning in `TICK` steps; `input` is read on each.
    /// Returns true when the output changed (a step ran).
    pub fn advance(&mut self, dt: f32, input: &Input) -> bool {
        self.acc += dt;
        if self.acc < TICK {
            return false;
        }
        let step = self.acc;
        self.acc = 0.0;
        self.step(step, input);
        true
    }

    pub fn info(&self, steer: &Steer) -> Info {
        let (weather, energy, change) = self.effective(steer);
        let segs = &self.eps[self.cur].segments;
        let rest: f32 = segs.iter().skip(self.seg + 1).map(|s| s.minutes.map_or(20.0, |r| 0.5 * (r.0[0] + r.0[1])) * 60.0 * self.scale).sum();
        let mut held = 0;
        for (c, m) in self.mode.iter().enumerate() {
            if matches!(m, Mode::Held(_)) {
                held |= 1 << c;
            }
        }
        Info {
            active: steer.amount > 0.0,
            episode: self.cur,
            segment: self.seg,
            segments: segs.len(),
            segment_left_s: self.seg_left.max(0.0),
            episode_left_s: self.seg_left.max(0.0) + if self.wrapping { 0.0 } else { rest },
            next: self.next,
            cued: self.cued,
            hour: self.hour as f32,
            liveliness: self.liveliness,
            weather,
            energy,
            change,
            held,
        }
    }

    /// Label of the current segment ("gust front").
    pub fn segment_label(&self) -> &str {
        &self.eps[self.cur].segments[self.seg].label
    }
}
