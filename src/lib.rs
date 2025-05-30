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

struct Easing {
    dir: EaseDirection,
    shape: EaseShape,
}

#[derive(Enum, Debug, PartialEq)]
enum EaseDirection {
    In,
    Out,
}

#[derive(Enum, Debug, PartialEq)]
enum EaseShape {
    Sine,
    Circle,
    Exponent,
    OutBack,
    Elastic,
    Bounce,
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
                Sine => 1.0 - f32::cos((t * PI) / 2.0),
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
                Bounce => 1.0 - ease_out_bounce(1.0 - t),
            },
            Out => match self.shape {
                Sine => 1.0 - f32::sin((t * PI) / 2.0),
                Circle => 1.0 - f32::sqrt(1.0 - (t - 1.0).powi(2)),
                Exponent => {
                    if t == 1.0 {
                        1.0
                    } else {
                        1.0 - 2_f32.powf(-10.0 * t)
                    }
                }
                OutBack => {
                    let c1 = 1.70158;
                    let c3 = c1 + 1.0;

                    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
                }
                Elastic => {
                    let c4 = TAU / 3.0;

                    if t == 0.0 {
                        0.0
                    } else if t == 1.0 {
                        1.0
                    } else {
                        2_f32.powf(-10.0 * t) * f32::sin((t * 10.0 - 0.75) * c4) + 1.0
                    }
                }
                Bounce => ease_out_bounce(t),
            },
        }
    }
}

fn ease_out_bounce(t: f32) -> f32 {
    let n1 = 7.5625;
    let d1 = 2.75;

    if t < 1.0 / d1 {
        n1 * t * t
    } else if t < 2.0 / d1 {
        n1 * ((t - 1.5) / d1) * t + 0.75
    } else if t < 2.5 / d1 {
        n1 * ((t - 2.25) / d1) * t + 0.9375
    } else {
        n1 * ((t - 2.625) / d1) * t + 0.984375
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

    #[id = "attack"]
    pub attack: FloatParam,

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
            .with_unit(" dB")
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
                FloatRange::Skewed {
                    min: 0.0,
                    max: 50.0,
                    factor: 0.375,
                },
            )
            .with_unit("ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            attack: FloatParam::new(
                "Attack",
                1.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 1.0,
                    factor: 0.5,
                },
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
            atk_dir: EnumParam::new("Attack Shape", EaseDirection::In),

            rel_char_amt: FloatParam::new(
                "Release Character",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            rel_shp: EnumParam::new("Release Shape", EaseShape::Sine),
            rel_dir: EnumParam::new("Release Shape", EaseDirection::In),

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
        for channel_samples in buffer.iter_samples() {
            for sample in channel_samples {
                let lookahead = self.params.lookahead.value() * self.sample_rate * 0.001;

                if lookahead.ceil() != self.lookahead_len {
                    // in bitwig i have to set half the latency samples?
                    // is it like this in other DAWs?
                    // whyyyyyyyy
                    context.set_latency_samples((lookahead / 2.0).ceil() as u32);
                    self.lookahead_len = lookahead.ceil();
                    self.reset();
                    continue;
                }

                let (input, trim) = (
                    self.params.input.smoothed.next(),
                    self.params.trim.smoothed.next(),
                );

                let (attack_amount, atk_char_amt, atk_shp, atk_dir) = (
                    self.params.attack.smoothed.next(),
                    self.params.atk_char_amt.smoothed.next(),
                    self.params.atk_shp.value(),
                    self.params.atk_dir.value(),
                );

                let (release_sec, hold_sec, rel_char_amt, rel_shp, rel_dir) = (
                    self.params.release.smoothed.next() * 0.001,
                    self.params.hold.smoothed.next() * 0.001,
                    self.params.rel_char_amt.smoothed.next(),
                    self.params.rel_shp.value(),
                    self.params.rel_dir.value(),
                );

                let release_len = release_sec * self.sample_rate;
                let hold_len = hold_sec * self.sample_rate;
                let atk_easing = Easing::new(atk_dir, atk_shp);
                let rel_easing = Easing::new(rel_dir, rel_shp);

                self.buffer.push_back(AttackSample {
                    sample: *sample * input,
                    db: util::gain_to_db_fast(sample.abs() * input),
                });

                if self.lookahead_len > self.buffer.len() as f32 {
                    *sample = 0.0;
                    continue;
                }

                let atk_env = self
                    .buffer
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.db > 0.0)
                    .fold(0.0, |rv, (i, s)| {
                        if s.db + rv < 0.0 {
                            return rv;
                        }
                        let t = (self.lookahead_len - i as f32) / self.lookahead_len;
                        let lerp_env = lerp(0.0, -1.0 * s.db, t);
                        let ease_env = lerp(0.0, -1.0 * s.db, atk_easing.calc(t));
                        let env = lerp(lerp_env, ease_env, atk_char_amt) * attack_amount;
                        f32::min(env, rv)
                    });

                if atk_env < self.envelope {
                    self.target = atk_env;
                    self.envelope = atk_env;
                    if hold_len.round() >= 1.0 {
                        self.env_state = EnvState::Hold(0.0);
                    } else if release_len.round() >= 1.0 {
                        self.env_state = EnvState::Release(0.0);
                    } else {
                        self.env_state = EnvState::Off;
                    }
                }

                let AttackSample {
                    sample: dly_sample,
                    db: delay_db,
                } = self.buffer.pop_front().unwrap();

                match &mut self.env_state {
                    EnvState::Off if delay_db > 0.0 => {
                        self.target = -1.0 * delay_db;
                        self.envelope = self.target;
                        if hold_len.round() >= 1.0 {
                            self.env_state = EnvState::Hold(0.0);
                        } else if release_len.round() >= 1.0 {
                            self.env_state = EnvState::Release(0.0);
                        } else {
                            self.env_state = EnvState::Off;
                        }
                    }
                    EnvState::Hold(_) if delay_db + self.envelope > 0.0 => {
                        self.target = -1.0 * delay_db;
                        self.envelope = self.target;
                        if hold_len.round() >= 1.0 {
                            self.env_state = EnvState::Hold(0.0);
                        } else if release_len.round() >= 1.0 {
                            self.env_state = EnvState::Release(0.0);
                        } else {
                            self.env_state = EnvState::Off;
                        }
                    }
                    EnvState::Hold(elapsed) => {
                        *elapsed += 1.0;
                        if *elapsed >= hold_len {
                            if release_len.round() >= 1.0 {
                                self.env_state = EnvState::Release(0.0);
                            } else {
                                self.env_state = EnvState::Off;
                            }
                        }
                    }
                    EnvState::Release(elapsed) => {
                        *elapsed += 1.0;
                        let t = *elapsed / release_len;
                        let lerp_env = lerp(self.target, 0.0, t);
                        let ease_env = lerp(self.target, 0.0, rel_easing.calc(t));
                        self.envelope = lerp(lerp_env, ease_env, rel_char_amt);

                        if *elapsed >= release_len {
                            self.env_state = EnvState::Off;
                        }

                        if delay_db + self.envelope > 0.0 {
                            self.target = -1.0 * delay_db;
                            self.envelope = self.target;
                            if hold_len.round() >= 1.0 {
                                self.env_state = EnvState::Hold(0.0);
                            } else if release_len.round() >= 1.0 {
                                self.env_state = EnvState::Release(0.0);
                            } else {
                                self.env_state = EnvState::Off;
                            }
                        }
                    }
                    EnvState::Off if self.target != 0.0 || self.envelope == 0.0 => {
                        self.target = 0.0;
                        self.envelope = 0.0;
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
    let t = f32::clamp(t, 0.0, 1.0);
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
