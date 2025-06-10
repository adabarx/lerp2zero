#![feature(portable_simd)]
use core::f32;
use nih_plug::prelude::*;
use std::{
    array,
    collections::VecDeque,
    simd::{
        cmp::{SimdPartialEq, SimdPartialOrd},
        f32x2,
        num::SimdFloat,
        LaneCount, Simd, StdFloat, SupportedLaneCount,
    },
    sync::Arc,
};

mod simd;

struct Limit2zero {
    params: Arc<Limit2zeroParams>,
    lookahead_len: f32,
    sample_rate: f32,
    channels: usize,
    limiter: LimiterBufferSimd<2>,
}

#[derive(Default, Debug, PartialEq, Clone, Copy)]
enum EnvState {
    Hold,
    Release,
    #[default]
    Off,
}

impl From<u32> for EnvState {
    fn from(value: u32) -> Self {
        match value {
            2 => EnvState::Hold,
            1 => EnvState::Release,
            0 => EnvState::Off,
            _ => panic!("lol"),
        }
    }
}

impl From<EnvState> for u32 {
    fn from(value: EnvState) -> Self {
        match value {
            EnvState::Hold => 2,
            EnvState::Release => 1,
            EnvState::Off => 0,
        }
    }
}

#[derive(Params)]
struct Limit2zeroParams {
    #[id = "drive"]
    pub drive: FloatParam,

    #[id = "trim"]
    pub trim: FloatParam,

    #[id = "lookahead"]
    pub lookahead: FloatParam,

    #[id = "attack_amt"]
    pub attack_amt: FloatParam,

    #[id = "atk_bend"]
    pub atk_bend: FloatParam,

    #[id = "hold"]
    pub hold: FloatParam,

    #[id = "hold_amt"]
    pub hold_amt: FloatParam,

    #[id = "release"]
    pub release: FloatParam,

    #[id = "rel_bend"]
    pub rel_bend: FloatParam,

    #[id = "stereo_link"]
    pub stereo_link: FloatParam,

    #[id = "compensate"]
    pub compensate: BoolParam,
}

impl Default for Limit2zero {
    fn default() -> Self {
        Self {
            params: Arc::new(Limit2zeroParams::default()),
            sample_rate: 44100.0,
            channels: 2,
            lookahead_len: 0.0,
            limiter: LimiterBufferSimd::<2>::new(256),
        }
    }
}

struct LimiterBufferSimd<const L: usize>
where
    LaneCount<L>: SupportedLaneCount,
{
    buffer: SampleBuf<L>,
    state: Simd<u32, L>,
    elapsed: Simd<f32, L>,
    target: Simd<f32, L>,
    hold: Simd<f32, L>,
    envelope: Simd<f32, L>,
}

impl<const L: usize> LimiterBufferSimd<L>
where
    LaneCount<L>: SupportedLaneCount,
{
    fn new(length: usize) -> Self {
        Self {
            buffer: SampleBuf::new(length),
            state: Simd::from_array([EnvState::Off.into(); L]),
            elapsed: Simd::from_array([0.0; L]),
            target: Simd::from_array([0.0; L]),
            hold: Simd::from_array([0.0; L]),
            envelope: Simd::from_array([0.0; L]),
        }
    }
}

struct SampleBuf<const L: usize>
where
    LaneCount<L>: SupportedLaneCount,
{
    samples: [VecDeque<f32>; L],
    dbs: [VecDeque<f32>; L],
}

impl<const L: usize> SampleBuf<L>
where
    LaneCount<L>: SupportedLaneCount,
{
    fn new(capacity: usize) -> Self {
        Self {
            samples: std::array::from_fn(|_| VecDeque::with_capacity(capacity)),
            dbs: std::array::from_fn(|_| VecDeque::with_capacity(capacity)),
        }
    }

    fn len(&self) -> usize {
        self.dbs[0].len()
    }

    fn push_back(&mut self, sample: [f32; L]) {
        for i in 0..L {
            self.samples[i].push_back(sample[i]);
            self.samples[i].push_back(util::gain_to_db_fast(sample[i]));
        }
    }

    fn pop_front(&mut self) -> Option<Sample<L>> {
        if self.samples.len() == 0 {
            return None;
        }
        let mut sample = [0.0; L];
        let mut db = [0.0; L];
        for i in 0..L {
            sample[i] = self.samples[i].pop_front().unwrap();
            db[i] = self.dbs[i].pop_front().unwrap();
        }
        Some(Sample::from_array(sample, db))
    }

    fn dbs_simd(&self, channel: usize) -> Vec<Simd<f32, L>> {
        if channel >= L {
            panic!("invalid channel")
        }
        let db = &self.dbs[channel];
        let mut output = Vec::with_capacity((db.len() + L - 1) / L);

        let mut end = false;
        let mut index = 0;
        loop {
            let mut padded = [0.0; L];
            for i in 0..L {
                if let Some(s) = db.get(index + i) {
                    padded[i] = *s;
                } else {
                    end = true;
                }
            }
            output.push(Simd::from_array(padded));
            if end {
                break;
            }
            index += L;
        }

        output
    }
}

struct Sample<const L: usize>
where
    LaneCount<L>: SupportedLaneCount,
{
    sample: Simd<f32, L>,
    db: Simd<f32, L>,
}

impl<const L: usize> Sample<L>
where
    LaneCount<L>: SupportedLaneCount,
{
    fn from_array(s: [f32; L], d: [f32; L]) -> Self {
        Self {
            sample: Simd::from_array(s),
            db: Simd::from_array(d),
        }
    }
}

impl Default for Limit2zeroParams {
    fn default() -> Self {
        Self {
            drive: FloatParam::new(
                "Drive",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(0.0),
                    max: util::db_to_gain(60.0),
                    factor: FloatRange::gain_skew_factor(0.0, 60.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit("dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),

            trim: FloatParam::new(
                "Trim",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 0.0,
                },
            )
            .with_unit("db")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            lookahead: FloatParam::new(
                "Lookahead",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 10.0,
                    factor: 0.75,
                },
            )
            .with_unit("ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            attack_amt: FloatParam::new(
                "Attack Amount",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            hold: FloatParam::new(
                "Hold",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 1000.,
                    factor: 0.375,
                },
            )
            .with_unit("ms")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            hold_amt: FloatParam::new(
                "Hold Amount",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            release: FloatParam::new(
                "Release",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 3000.,
                    factor: 0.375,
                },
            )
            .with_unit("ms")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            atk_bend: FloatParam::new(
                "Attack Bend",
                0.0,
                FloatRange::Skewed {
                    min: 0.25, // 0.5:-6 0.25:-12
                    max: 8.0,  // 2:6 4:12 8:18 16:24
                    factor: FloatRange::gain_skew_factor(-12.0, 18.0),
                },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            rel_bend: FloatParam::new(
                "Release bend",
                0.0,
                FloatRange::Skewed {
                    min: 0.25, // 0.5:-6 0.25:-12
                    max: 8.0,  // 2:6 4:12 8:18 16:24
                    factor: FloatRange::gain_skew_factor(-12.0, 18.0),
                },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            stereo_link: FloatParam::new(
                "Stereo Link",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            compensate: BoolParam::new("Gain Compensation", false),
        }
    }
}

impl Plugin for Limit2zero {
    const NAME: &'static str = "limit2zero";
    const VENDOR: &'static str = "Adamina Barx";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "adaminabarx@gmail.com";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),

        aux_input_ports: &[],
        aux_output_ports: &[],

        names: PortNames::const_default(),
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let channels = audio_io_layout.main_input_channels.unwrap().get() as usize;
        let lookahead_len =
            (self.params.lookahead.value() * 0.001 * buffer_config.sample_rate).ceil() as usize;
        self.sample_rate = buffer_config.sample_rate;
        self.channels = channels;
        self.limiter = LimiterBufferSimd::<2>::new(lookahead_len);
        true
    }

    fn reset(&mut self) {
        let la_len = self.lookahead_len.ceil() as usize;
        self.limiter = LimiterBufferSimd::<2>::new(la_len);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let (input, trim) = (
            self.params.drive.value(),
            Simd::splat(self.params.trim.value()),
        );

        let (lookahead, _atk_amt, _atk_bend) = (
            self.params.lookahead.value() * 0.001 * self.sample_rate,
            self.params.attack_amt.value(),
            self.params.atk_bend.value(),
        );

        let (hold, hold_amt) = (
            self.params.hold.value() * 0.001 * self.sample_rate,
            Simd::splat(self.params.hold_amt.value().sqrt()),
        );

        let (release, _rel_bend) = (
            self.params.release.value() * 0.001 * self.sample_rate,
            self.params.rel_bend.value(),
        );

        let stereo_link = self.params.stereo_link.value();

        if lookahead.ceil() != self.lookahead_len {
            // in bitwig i have to set half the latency samples?
            // is it like this in other DAWs?
            // whyyyyyyyy
            context.set_latency_samples((lookahead / 2.0).ceil() as u32);
            self.lookahead_len = lookahead.ceil();
            self.reset();
        }

        let buffer_samples = buffer.samples();
        let raw_buffer = buffer.as_slice();

        for s_id in 0..buffer_samples {
            let samples = array::from_fn(|i| {
                if let Some(channel) = raw_buffer.get(i) {
                    channel[s_id]
                } else {
                    0.0
                }
            });

            let sample_vec = f32x2::from_array(samples);
            let sample_vec = sample_vec * Simd::splat(input);

            self.limiter.buffer.push_back(sample_vec.into());

            let hold_mask = self
                .limiter
                .state
                .simd_eq(Simd::splat(EnvState::Hold.into()));
            let release_mask = self
                .limiter
                .state
                .simd_eq(Simd::splat(EnvState::Release.into()));

            if hold_mask.any() {
                let mut elapsed = self.limiter.elapsed;

                let elapsed_0 = elapsed.simd_eq(Simd::splat(0.0));
                self.limiter.target = elapsed_0.select(self.limiter.hold, self.limiter.target);
                self.limiter.envelope = elapsed_0.select(self.limiter.hold, self.limiter.envelope);

                elapsed += Simd::splat(1.0);

                self.limiter.elapsed = hold_mask.select(elapsed, self.limiter.elapsed);

                let elapsed_100 = elapsed.simd_ge(Simd::splat(hold));
                if elapsed_100.any() {
                    if release.round() >= 1.0 {
                        self.limiter.state = elapsed_100
                            .select(Simd::splat(EnvState::Release.into()), self.limiter.state);
                    } else {
                        self.limiter.state = elapsed_100
                            .select(Simd::splat(EnvState::Off.into()), self.limiter.state);
                    }
                    self.limiter.elapsed =
                        elapsed_100.select(Simd::splat(0.0), self.limiter.elapsed);
                }
            }

            if release_mask.any() {
                let mut elapsed = self.limiter.elapsed;

                let elapsed_0 = elapsed.simd_eq(Simd::splat(0.0));
                self.limiter.target = elapsed_0.select(self.limiter.hold, self.limiter.target);
                self.limiter.envelope = elapsed_0.select(self.limiter.hold, self.limiter.envelope);

                elapsed += Simd::splat(1.0);
                self.limiter.elapsed = hold_mask.select(elapsed, self.limiter.elapsed);

                let t = elapsed / Simd::splat(release);
                self.limiter.envelope = simd_lerp(self.limiter.target, Simd::splat(0.0), t);

                let elapsed_100 = elapsed.simd_ge(Simd::splat(hold));
                if elapsed_100.any() {
                    self.limiter.state =
                        elapsed_100.select(Simd::splat(EnvState::Off.into()), self.limiter.state);
                    self.limiter.elapsed =
                        elapsed_100.select(Simd::splat(0.0), self.limiter.elapsed);
                }
            }

            let delayed_sample = self.limiter.buffer.pop_front().unwrap();

            let attack = Simd::from_array(array::from_fn(|i| {
                let dbs = self.limiter.buffer.dbs_simd(i);
                let len = self.limiter.buffer.len() as f32;
                simd::simd_max_reduction(dbs, len)
            }));

            let atk_mask = attack.simd_gt(self.limiter.envelope);

            if atk_mask.any() {
                self.limiter.target = atk_mask.select(attack, self.limiter.target);
                self.limiter.hold = atk_mask.select(attack * hold_amt, self.limiter.hold);
                self.limiter.envelope = atk_mask.select(attack, self.limiter.envelope);
                self.limiter.elapsed = atk_mask.select(Simd::splat(0.0), self.limiter.elapsed);
                if hold.round() >= 1.0 {
                    self.limiter.state =
                        atk_mask.select(Simd::splat(EnvState::Hold.into()), self.limiter.state);
                } else if release.round() >= 1.0 {
                    self.limiter.state =
                        atk_mask.select(Simd::splat(EnvState::Release.into()), self.limiter.state);
                } else {
                    self.limiter.state =
                        atk_mask.select(Simd::splat(EnvState::Off.into()), self.limiter.state);
                }
            }

            let reduction = delayed_sample.db + self.limiter.envelope;
            let clip_mask = reduction.simd_lt(Simd::splat(0.0));

            if clip_mask.any() {
                let clip = delayed_sample.db * Simd::splat(-1.0);
                let clip_hold = clip * hold_amt;

                self.limiter.target = clip_mask.select(clip, self.limiter.target);
                self.limiter.hold = clip_mask.select(clip_hold, self.limiter.hold);
                self.limiter.envelope = clip_mask.select(clip, self.limiter.envelope);

                self.limiter.elapsed = atk_mask.select(Simd::splat(0.0), self.limiter.elapsed);
                if hold.round() >= 1.0 {
                    self.limiter.state =
                        atk_mask.select(Simd::splat(EnvState::Hold.into()), self.limiter.state);
                } else if release.round() >= 1.0 {
                    self.limiter.state =
                        atk_mask.select(Simd::splat(EnvState::Release.into()), self.limiter.state);
                } else {
                    self.limiter.state =
                        atk_mask.select(Simd::splat(EnvState::Off.into()), self.limiter.state);
                }
            }

            let max_reduction = Simd::splat(self.limiter.envelope.reduce_min());

            let stereo_link = Simd::splat(stereo_link);

            let stereo_lerp = simd_lerp(self.limiter.envelope, max_reduction, stereo_link);

            let compensation = Simd::from_array(array::from_fn(|_| {
                if self.params.compensate.value() {
                    util::gain_to_db_fast(input) / -2.0
                } else {
                    0.0
                }
            }));

            let final_reduction = stereo_lerp + trim + compensation;

            let sample: [f32; 2] =
                (delayed_sample.sample * db_to_gain_fast_simd(final_reduction)).into();

            for i in 0..self.channels {
                raw_buffer[i][s_id] = sample[i];
            }
        }
        ProcessStatus::Normal
    }
}

fn db_to_gain_fast_simd<const L: usize>(dbs: Simd<f32, L>) -> Simd<f32, L>
where
    LaneCount<L>: SupportedLaneCount,
{
    let conversion_factor = Simd::splat(std::f32::consts::LN_10 / 20.0);
    (dbs * conversion_factor).exp()
}

fn simd_lerp<const L: usize>(a: Simd<f32, L>, b: Simd<f32, L>, t: Simd<f32, L>) -> Simd<f32, L>
where
    LaneCount<L>: SupportedLaneCount,
{
    a + (b - a) * t
}

// fn lerp(a: f32, b: f32, t: f32) -> f32 {
//     a + (b - a) * t
// }

impl ClapPlugin for Limit2zero {
    const CLAP_ID: &'static str = "com.your-domain.limit2zero";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("basic limiter");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;

    // Don't forget to change these features
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo];
}

impl Vst3Plugin for Limit2zero {
    const VST3_CLASS_ID: [u8; 16] = *b"Exactly16Chars!!";

    // And also don't forget to change these categories
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
}

nih_export_clap!(Limit2zero);
nih_export_vst3!(Limit2zero);
