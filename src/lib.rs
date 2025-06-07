#![feature(portable_simd)]
use core::f32;
use nih_plug::prelude::*;
use std::{
    collections::VecDeque,
    simd::{num::SimdFloat, Simd},
    sync::Arc,
};

struct Limit2zero {
    params: Arc<Limit2zeroParams>,
    lookahead_len: f32,
    sample_rate: f32,
    channels: usize,
    limiters: LimiterBuffer,
}

#[derive(Debug, Default, Clone, Copy)]
struct SampleDB {
    sample: f32,
    db: f32,
}

impl SampleDB {
    fn peak(&self) -> bool {
        self.db > 0.0
    }
}

#[derive(Default, Debug, PartialEq, Clone, Copy)]
enum EnvState {
    Hold(f32),
    Release(f32),
    #[default]
    Off,
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
            limiters: LimiterBuffer::new(2, 256),
        }
    }
}

struct LimiterBuffer {
    channels: usize,
    buffers: Vec<VecDeque<SampleDB>>,
    peaks: Vec<VecDeque<Peak>>,
    state: Vec<EnvState>,
    target: Vec<f32>,
    hold: Vec<f32>,
    envelope: Vec<f32>,
}

#[derive(Clone, Copy)]
struct Peak {
    db: f32,
    index: isize,
}

impl Peak {
    fn factor(&self) -> f32 {
        self.index as f32 * self.db
    }

    fn index_f32(&self) -> f32 {
        self.index as f32
    }
}

struct Limiter<'a> {
    buffer: &'a mut VecDeque<SampleDB>,
    peaks: &'a mut VecDeque<Peak>,
    state: &'a mut EnvState,
    target: &'a mut f32,
    hold: &'a mut f32,
    envelope: &'a mut f32,
}

impl LimiterBuffer {
    fn new(channels: usize, sample_len: usize) -> Self {
        let mut rv = LimiterBuffer {
            channels,
            buffers: vec![VecDeque::with_capacity(sample_len); channels],
            peaks: vec![VecDeque::with_capacity(sample_len); channels],
            state: vec![EnvState::Off; channels],
            target: vec![0.0; channels],
            hold: vec![0.0; channels],
            envelope: vec![0.0; channels],
        };

        for b in rv.buffers.iter_mut() {
            for _ in 0..sample_len {
                b.push_back(SampleDB {
                    sample: 0.0,
                    db: -100.0,
                });
            }
        }

        rv
    }

    fn get_mut(&mut self, channel: usize) -> Limiter {
        let channel = channel.clamp(0, self.channels - 1);
        Limiter {
            buffer: self.buffers.get_mut(channel).unwrap(),
            peaks: self.peaks.get_mut(channel).unwrap(),
            state: self.state.get_mut(channel).unwrap(),
            target: self.target.get_mut(channel).unwrap(),
            hold: self.hold.get_mut(channel).unwrap(),
            envelope: self.envelope.get_mut(channel).unwrap(),
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

fn simd_max_peak(buffer: &[f32]) -> f32 {
    const LANES: usize = 8;

    let len_vec = Simd::splat((buffer.len() + 1) as f32);
    let mut max_vec = Simd::splat(0.0);

    let chunks = buffer.chunks_exact(LANES);
    let remainder = chunks.remainder();

    let mut base_index = 1.0;

    for chunk in chunks {
        let vec_index =
            Simd::<f32, LANES>::from_array(core::array::from_fn(|i| base_index + i as f32));
        let values = Simd::from_slice(chunk);

        max_vec = max_vec.simd_max(values * vec_index / len_vec);

        base_index += LANES as f32;
    }

    if !remainder.is_empty() {
        let mut padded_vals = [0.0; LANES];
        let mut padded_idx = [0.0; LANES];

        for i in 0..remainder.len() {
            padded_vals[i] = remainder[i];
            padded_idx[i] = base_index + i as f32;
        }

        let values = Simd::from_array(padded_vals);
        let vec_index = Simd::from_array(padded_idx);

        max_vec = max_vec.simd_max(values * vec_index / len_vec)
    }

    max_vec.reduce_max()
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
        self.limiters = LimiterBuffer::new(channels, lookahead_len);
        true
    }

    fn reset(&mut self) {
        let la_len = self.lookahead_len.ceil() as usize;
        self.limiters = LimiterBuffer::new(self.channels, la_len);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let (input, trim) = (self.params.drive.value(), self.params.trim.value());

        let (lookahead, atk_amt, atk_bend) = (
            self.params.lookahead.value() * 0.001 * self.sample_rate,
            self.params.attack_amt.value(),
            self.params.atk_bend.value(),
        );

        let (hold, hold_amt) = (
            self.params.hold.value() * 0.001 * self.sample_rate,
            self.params.hold_amt.value(),
        );

        let (release, rel_bend) = (
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

        struct Samples {
            samples: Vec<f32>,
            reductions: Vec<f32>,
        }

        impl Samples {
            fn add(&mut self, sample: f32, reduction: f32) {
                self.samples.push(sample);
                self.reductions.push(reduction);
            }
        }

        let buffer_samples = buffer.samples();
        let raw_buffer = buffer.as_slice();

        for sample_id in 0..buffer_samples {
            let mut rv_samples = Samples {
                samples: Vec::with_capacity(raw_buffer.len()),
                reductions: Vec::with_capacity(raw_buffer.len()),
            };

            let mut most_reduction = 0.0;

            let channel_samples: Vec<_> = raw_buffer
                .iter_mut()
                .enumerate()
                .map(|(i, channel)| (i, channel.get_mut(sample_id).unwrap()))
                .collect();

            for (i, sample) in channel_samples {
                let mut limiter = self.limiters.get_mut(i);

                let new_sample = SampleDB {
                    sample: *sample * input,
                    db: util::gain_to_db_fast(sample.abs() * input),
                };

                if new_sample.peak() {
                    limiter.peaks.push_back(Peak {
                        db: new_sample.db,
                        index: 0,
                    });
                }

                limiter.buffer.push_back(new_sample);

                // do stuff based on envelope state
                match &mut limiter.state {
                    EnvState::Hold(elapsed) => {
                        if *elapsed == 0.0 {
                            *limiter.target = *limiter.hold;
                            *limiter.envelope = *limiter.hold;
                        }
                        *elapsed += 1.0;
                        if *elapsed >= hold {
                            if release.round() >= 1.0 {
                                *limiter.state = EnvState::Release(0.0);
                            } else {
                                *limiter.state = EnvState::Off;
                            }
                        }
                    }
                    EnvState::Release(elapsed) => {
                        if *elapsed == 0.0 {
                            *limiter.target = *limiter.hold;
                            *limiter.envelope = *limiter.hold;
                        }
                        *elapsed += 1.0;
                        let t = *elapsed / release;
                        *limiter.envelope = lerp(*limiter.target, 0.0, t.powf(rel_bend));

                        if *elapsed >= release {
                            *limiter.state = EnvState::Off;
                        }
                    }
                    EnvState::Off => {
                        if *limiter.envelope != 0.0
                            || *limiter.target != 0.0
                            || *limiter.hold != 0.0
                        {
                            *limiter.envelope = 0.0;
                            *limiter.target = 0.0;
                            *limiter.hold = 0.0;
                        }
                    }
                }

                let mut peak = Peak { db: 0.0, index: 0 };
                let mut peak_factor = 0.0;

                for p in limiter.peaks.iter_mut() {
                    let p_factor = p.factor();
                    if p_factor > peak_factor {
                        peak = *p;
                        peak_factor = p_factor
                    }
                    p.index += 1;
                }

                // calculate atk envelope
                if peak.db > 0.0 {
                    let t = peak.index_f32() / self.lookahead_len;
                    let atk_env = lerp(0.0, -1.0 * peak.db, t.powf(atk_bend)) * atk_amt;

                    if atk_env < *limiter.envelope {
                        *limiter.target = atk_env;
                        *limiter.hold = atk_env * hold_amt.sqrt();
                        *limiter.envelope = atk_env;
                        if hold.round() >= 1.0 {
                            *limiter.state = EnvState::Hold(0.0);
                        } else if release.round() >= 1.0 {
                            *limiter.state = EnvState::Release(0.0);
                        } else {
                            *limiter.state = EnvState::Off;
                        }
                    }
                }

                // grab delayed sample from buffer
                let delay = limiter.buffer.pop_front().unwrap();

                if delay.peak() {
                    let _ = limiter.peaks.pop_front().unwrap();
                }

                if delay.db + *limiter.envelope > 0.0 {
                    *limiter.target = -1.0 * delay.db;
                    *limiter.hold = *limiter.target * hold_amt.sqrt();
                    *limiter.envelope = *limiter.target;
                    if hold.round() >= 1.0 {
                        *limiter.state = EnvState::Hold(0.0);
                    } else if release.round() >= 1.0 {
                        *limiter.state = EnvState::Release(0.0);
                    } else {
                        *limiter.state = EnvState::Off;
                    }
                }

                most_reduction = f32::min(most_reduction, *limiter.envelope);

                rv_samples.add(delay.sample, *limiter.envelope);
            }

            let compensation = if self.params.compensate.value() {
                util::gain_to_db_fast(input) / -2.0
            } else {
                0.0
            };

            for (i, s) in rv_samples.samples.iter().enumerate() {
                let channel = raw_buffer.get_mut(i).unwrap();
                let reduce = rv_samples.reductions.get(i).unwrap();
                let reduce = lerp(*reduce, most_reduction, stereo_link);

                *channel.get_mut(sample_id).unwrap() =
                    s * util::db_to_gain_fast(reduce + trim + compensation)
            }
        }
        ProcessStatus::Normal
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

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
