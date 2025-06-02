use core::f32;
use nih_plug::prelude::*;
use std::{collections::VecDeque, sync::Arc};

struct Limit2zero {
    params: Arc<Limit2zeroParams>,
    sample_rate: f32,
    target: f32,
    hold: f32,
    envelope: f32,
    env_state: EnvState,
    buffer: VecDeque<SampleDB>,
    lookahead_len: f32,
}

#[derive(Debug, Default, Clone, Copy)]
struct SampleDB {
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
}

impl Default for Limit2zero {
    fn default() -> Self {
        Self {
            params: Arc::new(Limit2zeroParams::default()),
            sample_rate: 44100.0,
            target: 0.0, // db
            hold: 0.0,   // db
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
                    min: util::db_to_gain(-36.0),
                    max: util::db_to_gain(36.0),
                    factor: FloatRange::gain_skew_factor(-36.0, 36.0),
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
                self.buffer.push_back(SampleDB {
                    sample: *sample * input,
                    db: util::gain_to_db_fast(sample.abs() * input),
                });

                if self.lookahead_len > self.buffer.len() as f32 {
                    *sample = 0.0;
                    continue;
                }

                match &mut self.env_state {
                    EnvState::Hold(elapsed) => {
                        if *elapsed == 0.0 {
                            self.target = self.hold;
                            self.envelope = self.hold;
                        }
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
                        if *elapsed == 0.0 {
                            self.target = self.hold;
                            self.envelope = self.hold;
                        }
                        *elapsed += 1.0;
                        let t = *elapsed / release;
                        self.envelope = lerp(self.target, 0.0, t.powf(rel_bend));

                        if *elapsed >= release {
                            self.env_state = EnvState::Off;
                        }
                    }
                    EnvState::Off => {
                        if self.envelope != 0.0 || self.target != 0.0 || self.hold != 0.0 {
                            self.envelope = 0.0;
                            self.target = 0.0;
                            self.hold = 0.0;
                        }
                    }
                }

                let (i, s) = self
                    .buffer
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.db > 0.0)
                    .fold((0_usize, SampleDB::default()), |highest, sample| {
                        if sample.1.db > highest.1.db {
                            (sample.0, *sample.1)
                        } else {
                            highest
                        }
                    });

                let t = (self.lookahead_len - i as f32) / self.lookahead_len;
                let atk_env = lerp(0.0, -1.0 * s.db, t.powf(atk_bend)) * atk_amt;

                if atk_env < self.envelope {
                    self.target = atk_env;
                    self.hold = atk_env;
                    self.envelope = atk_env;
                    if hold.round() >= 1.0 {
                        self.env_state = EnvState::Hold(0.0);
                    } else if release.round() >= 1.0 {
                        self.env_state = EnvState::Release(0.0);
                    } else {
                        self.env_state = EnvState::Off;
                    }
                }

                let delay = self.buffer.pop_front().unwrap();

                if delay.db + self.envelope > 0.0 {
                    self.target = -1.0 * delay.db;
                    self.hold = self.target * hold_amt.powf(0.5);
                    self.envelope = self.target;
                    if hold.round() >= 1.0 {
                        self.env_state = EnvState::Hold(0.0);
                    } else if release.round() >= 1.0 {
                        self.env_state = EnvState::Release(0.0);
                    } else {
                        self.env_state = EnvState::Off;
                    }
                }
                *sample = delay.sample * util::db_to_gain_fast(self.envelope + trim);
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
