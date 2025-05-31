use core::f32;
use nih_plug::prelude::*;
use std::{collections::VecDeque, sync::Arc};

struct Limit2zero {
    params: Arc<Limit2zeroParams>,
    sample_rate: f32,
    target: f32,
    envelope: f32,
    env_state: EnvState,
    buffer: VecDeque<AttackSample>,
    lookahead_len: f32,
}

#[derive(Debug, Default, Clone, Copy)]
struct AttackSample {
    sample: f32,
    db: f32,
}

#[derive(Default, Debug, PartialEq)]
enum EnvState {
    Hold(f32),
    Release(f32),
    #[default]
    Off,
}

#[derive(Debug, PartialEq, Clone, Copy)]
struct Easing {
    dir: EaseDirection,
    shape: EaseShape,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
enum EaseDirection {
    In,
    Out,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
enum EaseShape {
    Sine,
    Circle,
    Exponent,
    OutBack,
    Elastic,
}

impl Easing {
    fn new(dir: EaseDirection, shape: EaseShape) -> Self {
        Self { dir, shape }
    }

    fn calc(&self, t: f32) -> f32 {
        use f32::consts::{PI, TAU};
        use EaseDirection::*;
        use EaseShape::*;
        match self.dir {
            In => match self.shape {
                Sine => f32::sin((t * PI) / 2.0),
                Circle => 1.0 - f32::sqrt(1.0 - t.powi(2)),
                Exponent => {
                    if t == 0.0 {
                        0.0
                    } else {
                        2_f32.powf(10.0 * t - 10.0)
                    }
                }
                OutBack => {
                    let c1 = 1.70158;
                    let c3 = c1 + 1.0;

                    c3 * t * t * t - c1 * t * t
                }
                Elastic => {
                    let c4 = TAU / 3.0;

                    if t == 0.0 {
                        0.0
                    } else if t == 1.0 {
                        1.0
                    } else {
                        -2_f32.powf(10.0 * t - 10.0) * f32::sin((t * 10.0 - 10.75) * c4)
                    }
                }
            },
            Out => {
                let out = Easing::new(EaseDirection::In, self.shape);
                1.0 - out.calc(1.0 - t)
            }
        }
    }
}

#[derive(Params)]
struct Limit2zeroParams {
    #[id = "input"]
    pub input: FloatParam,

    #[id = "trim"]
    pub trim: FloatParam,

    #[id = "lookahead"]
    pub lookahead: FloatParam,

    #[id = "attack_amt"]
    pub attack_amt: FloatParam,

    #[id = "hold"]
    pub hold: FloatParam,

    #[id = "release"]
    pub release: FloatParam,

    #[id = "atk_char_amt"]
    pub atk_char_amt: FloatParam,

    #[id = "atk_shp"]
    pub atk_shp: EnumParam<EaseShape>,

    #[id = "atk_dir"]
    pub atk_dir: EnumParam<EaseDirection>,

    #[id = "rel_char_amt"]
    pub rel_char_amt: FloatParam,

    #[id = "rel_shp"]
    pub rel_shp: EnumParam<EaseShape>,

    #[id = "rel_dir"]
    pub rel_dir: EnumParam<EaseDirection>,

    #[id = "c2z"]
    pub c2z: BoolParam,
}

impl Default for Limit2zero {
    fn default() -> Self {
        Self {
            params: Arc::new(Limit2zeroParams::default()),
            sample_rate: 44100.0,
            target: 0.0, // db
            envelope: 0.0,
            env_state: EnvState::Off,
            buffer: VecDeque::new(),
            lookahead_len: 0.0,
        }
    }
}

impl Default for Limit2zeroParams {
    fn default() -> Self {
        Self {
            input: FloatParam::new(
                "Input",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-30.0),
                    max: util::db_to_gain(30.0),
                    factor: FloatRange::gain_skew_factor(-30.0, 30.0),
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
                1.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 50.0,
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
                100.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 1000.,
                    factor: 0.25,
                },
            )
            .with_unit("ms")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            release: FloatParam::new(
                "Release",
                100.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 5000.,
                    factor: 0.301,
                },
            )
            .with_unit("ms")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            atk_char_amt: FloatParam::new(
                "Attack Character",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            atk_shp: EnumParam::new("Attack Shape", EaseShape::Sine),
            atk_dir: EnumParam::new("Attack Dir", EaseDirection::In),

            rel_char_amt: FloatParam::new(
                "Release Character",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            rel_shp: EnumParam::new("Release Shape", EaseShape::Sine),
            rel_dir: EnumParam::new("Release Dir", EaseDirection::In),

            c2z: BoolParam::new("c2z", true),
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
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        true
    }

    fn reset(&mut self) {
        self.buffer = VecDeque::with_capacity(self.lookahead_len.ceil() as usize);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let (input, trim) = (self.params.input.value(), self.params.trim.value());

        let (lookahead, attack_amount, atk_char_amt, atk_shp, atk_dir) = (
            self.params.lookahead.value() * 0.001 * self.sample_rate,
            self.params.attack_amt.value(),
            self.params.atk_char_amt.value(),
            self.params.atk_shp.value(),
            self.params.atk_dir.value(),
        );

        let (release, hold, rel_char_amt, rel_shp, rel_dir) = (
            self.params.release.value() * 0.001 * self.sample_rate,
            self.params.hold.value() * 0.001 * self.sample_rate,
            self.params.rel_char_amt.value(),
            self.params.rel_shp.value(),
            self.params.rel_dir.value(),
        );

        let rel_easing = Easing::new(rel_dir, rel_shp);
        let atk_easing = Easing::new(atk_dir, atk_shp);

        if lookahead.ceil() != self.lookahead_len {
            // in bitwig i have to set half the latency samples?
            // is it like this in other DAWs?
            // whyyyyyyyy
            context.set_latency_samples((lookahead / 2.0).ceil() as u32);
            self.lookahead_len = lookahead.ceil();
            self.reset();
        }

        for channel_samples in buffer.iter_samples() {
            for sample in channel_samples {
                self.buffer.push_back(AttackSample {
                    sample: *sample * input,
                    db: util::gain_to_db_fast(sample.abs() * input),
                });

                if self.lookahead_len > self.buffer.len() as f32 {
                    *sample = 0.0;
                    continue;
                }

                let (_, i, s) = self
                    .buffer
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.db > 0.0)
                    .fold(
                        (0.0, 0_usize, AttackSample::default()),
                        |(f, i, s), sample| {
                            let s_factor = (self.lookahead_len - sample.0 as f32).sqrt();
                            if sample.1.db * s_factor > s.db * f {
                                (s_factor, sample.0, *sample.1)
                            } else {
                                (f, i, s)
                            }
                        },
                    );

                // if s.sample != 0.0 {
                let t = (self.lookahead_len - i as f32) / self.lookahead_len;
                let lerp_env = lerp(0.0, -1.0 * s.db, t);
                let ease_env = lerp(0.0, -1.0 * s.db, atk_easing.calc(t));
                let env = lerp(lerp_env, ease_env, atk_char_amt) * attack_amount;

                if env < self.envelope {
                    self.target = env;
                    self.envelope = env;
                    if hold.round() >= 1.0 {
                        self.env_state = EnvState::Hold(0.0);
                    } else if release.round() >= 1.0 {
                        self.env_state = EnvState::Release(0.0);
                    } else {
                        self.env_state = EnvState::Off;
                    }
                }
                // }

                let AttackSample {
                    sample: dly_sample,
                    db: delay_db,
                } = self.buffer.pop_front().unwrap();

                match &mut self.env_state {
                    EnvState::Off if delay_db > 0.0 => {
                        self.target = -1.0 * delay_db;
                        self.envelope = self.target;
                        if hold.round() >= 1.0 {
                            self.env_state = EnvState::Hold(0.0);
                        } else if release.round() >= 1.0 {
                            self.env_state = EnvState::Release(0.0);
                        } else {
                            self.env_state = EnvState::Off;
                        }
                    }
                    EnvState::Hold(_) if delay_db + self.envelope > 0.0 => {
                        self.target = -1.0 * delay_db;
                        self.envelope = self.target;
                        if hold.round() >= 1.0 {
                            self.env_state = EnvState::Hold(0.0);
                        } else if release.round() >= 1.0 {
                            self.env_state = EnvState::Release(0.0);
                        } else {
                            self.env_state = EnvState::Off;
                        }
                    }
                    EnvState::Hold(elapsed) => {
                        *elapsed += 1.0;
                        if *elapsed >= hold {
                            if release.round() >= 1.0 {
                                self.env_state = EnvState::Release(0.0);
                            } else {
                                self.env_state = EnvState::Off;
                            }
                        }
                    }
                    EnvState::Release(elapsed) => {
                        *elapsed += 1.0;
                        let t = *elapsed / release;
                        let lerp_env = lerp(self.target, 0.0, t);
                        let ease_env = lerp(self.target, 0.0, rel_easing.calc(t));
                        self.envelope = lerp(lerp_env, ease_env, rel_char_amt);

                        if *elapsed >= release {
                            self.env_state = EnvState::Off;
                        }

                        if delay_db + self.envelope > 0.0 {
                            self.target = -1.0 * delay_db;
                            self.envelope = self.target;
                            if hold.round() >= 1.0 {
                                self.env_state = EnvState::Hold(0.0);
                            } else if release.round() >= 1.0 {
                                self.env_state = EnvState::Release(0.0);
                            } else {
                                self.env_state = EnvState::Off;
                            }
                        }
                    }
                    EnvState::Off => (),
                }

                *sample = dly_sample * util::db_to_gain_fast(self.envelope + trim);

                if self.params.c2z.value() {
                    *sample = c2z(*sample);
                }
            }
        }

        ProcessStatus::Normal
    }
}

fn c2z(s: f32) -> f32 {
    if s.abs() > 1.0 {
        return s / s.abs();
    }
    s
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
