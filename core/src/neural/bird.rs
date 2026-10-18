//! Neural songbird generator: song model picks syllables, the contour bank shapes them, the
//! DDSP voice sings them. Same shape as `sources::Bird` (`render(out, controls)` -> mono).
//!
//! Real-time contract: no allocation after `new`; idle birds cost almost nothing (zeros), and
//! the network runs once per 5 ms frame only while a song is in progress.

use std::sync::Arc;

use super::song::{at, Contour, SongState};
use super::voice::{Frame, VoiceState};
use super::NeuralModel;
use crate::rng::Rng;

/// What the director (or the engine's sliders) can ask of a bird.
#[derive(Clone, Copy, Debug)]
pub struct BirdControls {
    /// 0..1: how often songs start (0 = silent after the current song).
    pub activity: f32,
    /// Pitch shift in semitones (individual voices differ by about +-1).
    pub pitch: f32,
    /// Tempo factor: >1 sings faster (shorter syllables and gaps).
    pub tempo: f32,
    /// Added to the effort contour: >0 buzzier / more strained, <0 purer.
    pub effort: f32,
    /// Loudness offset in dB (also changes timbre a little, as in a real voice).
    pub loudness: f32,
    /// 0..2: spread of syllable shapes and sampling temperature of the song model.
    pub variation: f32,
}

impl Default for BirdControls {
    fn default() -> Self {
        Self { activity: 0.5, pitch: 0.0, tempo: 1.0, effort: 0.0, loudness: 0.0, variation: 1.0 }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Seg {
    Syllable,
    Gap,
    Tail,
}

struct Phrase {
    song: SongState,
    cur: Contour,
    next: Contour,
    next_tok: Option<usize>,
    tok: usize,
    seg: Seg,
    frame: u32,
    frames: u32,
    count: u32,
    done: bool,
    f0_now: f32,
}

pub struct NeuralBird {
    model: Arc<NeuralModel>,
    voice: VoiceState,
    phrase: Phrase,
    rng: Rng,
    singing: bool,
    timer: f32,
    sample_rate: f32,
    /// Per-bird personality: small fixed pitch and tempo offsets.
    pitch_offset: f32,
    tempo_scale: f32,
    controls: BirdControls,
}

const MAX_SYLLABLES: u32 = 64;
const TAIL_FRAMES: u32 = 30;

impl NeuralBird {
    pub fn new(model: Arc<NeuralModel>, sample_rate: f32, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let voice = model.voice.new_state(sample_rate, rng.next_u64());
        let phrase = Phrase {
            song: model.song.new_state(),
            cur: model.song.new_contour(),
            next: model.song.new_contour(),
            next_tok: None,
            tok: 0,
            seg: Seg::Tail,
            frame: 0,
            frames: 0,
            count: 0,
            done: true,
            f0_now: 0.0,
        };
        let timer = sample_rate * rng.range(0.3, 3.0);
        let pitch_offset = rng.range(-0.7, 0.7);
        let tempo_scale = rng.range(0.92, 1.08);
        Self {
            model,
            voice,
            phrase,
            rng,
            singing: false,
            timer,
            sample_rate,
            pitch_offset,
            tempo_scale,
            controls: BirdControls::default(),
        }
    }

    /// Instantaneous pitch while a syllable sounds, 0 otherwise (UI / telemetry).
    pub fn freq(&self) -> f32 {
        if self.singing && self.phrase.seg == Seg::Syllable {
            self.phrase.f0_now
        } else {
            0.0
        }
    }

    pub fn is_singing(&self) -> bool {
        self.singing
    }

    pub fn model(&self) -> &NeuralModel {
        &self.model
    }

    fn start_song(&mut self) {
        let m = &self.model;
        let c = self.controls;
        let temp = 0.6 + 0.4 * c.variation;
        let p = &mut self.phrase;
        m.song.reset(&mut p.song);
        let (tok, _) = m.song.next(&mut p.song, None, temp, &mut self.rng).unwrap_or((0, 0.0));
        m.song.contour(tok, c.variation, &mut self.rng, &mut p.cur);
        p.tok = tok;
        p.seg = Seg::Syllable;
        p.frame = 0;
        p.frames = frames_for(p.cur.dur_s, m.voice.hop_s, c.tempo * self.tempo_scale);
        p.count = 1;
        p.done = false;
        p.next_tok = None;
        let first = frame_of(p, &c, self.pitch_offset, m);
        m.voice.start(&mut self.voice, first);
        self.singing = true;
    }

    /// Renders one block of mono audio (overwrites `out`).
    pub fn render(&mut self, out: &mut [f32], controls: &BirdControls) {
        self.controls = *controls;
        let mut i = 0;
        while i < out.len() {
            if !self.singing {
                let n = (out.len() - i).min(self.timer.max(0.0).ceil() as usize).max(0);
                out[i..i + n].iter_mut().for_each(|o| *o = 0.0);
                i += n;
                self.timer -= n as f32;
                if self.timer <= 0.0 {
                    if controls.activity > 0.02 {
                        self.start_song();
                    } else {
                        self.timer = self.sample_rate * 0.5;
                    }
                }
                continue;
            }
            // Sing until the song's tail has finished (checked per 64-sample chunk).
            let n = (out.len() - i).min(64);
            let m = &*self.model;
            let (phrase, rng, c, po) = (&mut self.phrase, &mut self.rng, &self.controls, self.pitch_offset);
            let ts = self.tempo_scale;
            m.voice.render(&mut self.voice, &mut out[i..i + n], || advance(m, phrase, rng, c, po, ts));
            let g = m.out_gain;
            out[i..i + n].iter_mut().for_each(|o| *o *= g);
            i += n;
            if self.phrase.done {
                self.singing = false;
                self.voice.active = false;
                let a = controls.activity.clamp(0.0, 1.0);
                let mean = m.song_interval_s * (0.6 + 4.0 * (1.0 - a));
                self.timer = mean * self.rng.range(0.5, 1.5) * self.sample_rate;
            }
        }
    }
}

fn frames_for(dur_s: f32, hop_s: f32, tempo: f32) -> u32 {
    ((dur_s / tempo.max(0.25)) / hop_s).round().max(2.0) as u32
}

fn frame_of(p: &mut Phrase, c: &BirdControls, pitch_offset: f32, m: &NeuralModel) -> Frame {
    let (silence_db, silence_effort) = (m.silence_db, m.silence_effort);
    let u = p.frame as f32 / (p.frames.max(2) - 1) as f32;
    let shift = (c.pitch + pitch_offset) / 12.0;
    let f = match p.seg {
        Seg::Syllable => Frame {
            f0: (at(&p.cur.f0, u) + shift).exp2(),
            loud: at(&p.cur.loud, u) + c.loudness,
            effort: (at(&p.cur.effort, u) + c.effort).clamp(0.0, 1.0),
        },
        Seg::Gap => {
            // Glide f0 from the end of this syllable to the start of the next one.
            let a = p.cur.f0[p.cur.f0.len() - 1];
            let b = p.next.f0[0];
            Frame { f0: (a + (b - a) * u + shift).exp2(), loud: silence_db, effort: silence_effort }
        }
        Seg::Tail => {
            let a = p.cur.f0[p.cur.f0.len() - 1];
            Frame { f0: (a + shift).exp2(), loud: silence_db, effort: silence_effort }
        }
    };
    p.f0_now = f.f0;
    f
}

/// Produces the next frame of the song, moving syllable -> gap -> syllable ... -> tail.
fn advance(m: &NeuralModel, p: &mut Phrase, rng: &mut Rng, c: &BirdControls, pitch_offset: f32, tempo_scale: f32) -> Frame {
    p.frame += 1;
    if p.frame >= p.frames {
        let tempo = c.tempo * tempo_scale;
        let temp = 0.6 + 0.4 * c.variation;
        match p.seg {
            Seg::Syllable => {
                let next = if p.count < MAX_SYLLABLES { m.song.next(&mut p.song, Some(p.tok), temp, rng) } else { None };
                match next {
                    Some((tok, gap)) => {
                        m.song.contour(tok, c.variation, rng, &mut p.next);
                        p.next_tok = Some(tok);
                        p.seg = Seg::Gap;
                        p.frames = frames_for(gap, m.voice.hop_s, tempo);
                    }
                    None => {
                        p.seg = Seg::Tail;
                        p.frames = TAIL_FRAMES;
                    }
                }
            }
            Seg::Gap => {
                core::mem::swap(&mut p.cur, &mut p.next);
                p.tok = p.next_tok.take().unwrap_or(0);
                p.seg = Seg::Syllable;
                p.frames = frames_for(p.cur.dur_s, m.voice.hop_s, tempo);
                p.count += 1;
            }
            Seg::Tail => {
                p.done = true;
            }
        }
        p.frame = 0;
    }
    frame_of(p, c, pitch_offset, m)
}
