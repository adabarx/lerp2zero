use core::f32;
use nih_plug::prelude::*;
use std::{collections::VecDeque, sync::Arc};

// This is a shortened version of the gain example with most comments removed, check out
// https://github.com/robbert-vdh/nih-plug/blob/master/plugins/examples/gain/src/lib.rs to get
// started

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

// enum Ratio {
//     One,
//     Sqrt2,
//     Two,
//     Three,
//     Four,
//     Six,
//     Eight,
//     Twelve,
//     Sixteen,
//     Twentyfour,
//     Infinite,
// }

#[derive(Params)]
struct Limit2zeroParams {
    /// The parameter's ID is used to identify the parameter in the wrappred plugin API. As long as
    /// these IDs remain constant, you can rename and reorder these fields as you wish. The
    /// parameters are exposed to the host in the same order they were defined. In this case, this
    /// gain parameter is stored as linear gain while the values are displayed in decibels.
    #[id = "input"]
    pub input: FloatParam,

    #[id = "hold"]
    pub hold: FloatParam,

    #[id = "lookahead"]
    pub lookahead: FloatParam,

    #[id = "attack"]
    pub attack: FloatParam,

    #[id = "release"]
    pub release: FloatParam,

    #[id = "limit2"]
    pub limit2: FloatParam,
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
            // This gain is stored as linear gain. NIH-plug comes with useful conversion functions
            // to treat these kinds of parameters as if we were dealing with decibels. Storing this
            // as decibels is easier to work with, but requires a conversion for every sample.
            input: FloatParam::new(
                "Input",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-30.0),
                    max: util::db_to_gain(30.0),
                    // This makes the range appear as if it was linear when displaying the values as
                    // decibels
                    factor: FloatRange::gain_skew_factor(-30.0, 30.0),
                },
            )
            // Because the gain parameter is stored as linear gain instead of storing the value as
            // decibels, we need logarithmic smoothing
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            // There are many predefined formatters we can use here. If the gain was stored as
            // decibels instead of as a linear gain value, we could have also used the
            // `.with_step_size(0.1)` function to get internal rounding.
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),

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

            limit2: FloatParam::new(
                "limit2",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 0.0,
                },
            )
            .with_unit("db")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
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
                    context.set_latency_samples((lookahead / 2.0).ceil() as u32);
                    self.lookahead_len = lookahead.ceil();
                    self.reset();
                    continue;
                }

                let attack_amount = self.params.attack.smoothed.next();
                let input = self.params.input.smoothed.next();
                let limit2 = self.params.limit2.smoothed.next();
                let release_sec = self.params.release.smoothed.next() * 0.001;
                let hold_sec = self.params.hold.smoothed.next() * 0.001;

                let release_len = release_sec * self.sample_rate;
                let hold_len = hold_sec * self.sample_rate;

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
                        let t = (self.lookahead_len - i as f32) / self.lookahead_len;
                        let env = lerp(0.0, -1.0 * s.db, t) * attack_amount;
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
                        self.envelope = lerp(self.target, 0.0, t);

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

                *sample = dly_sample * util::db_to_gain_fast(self.envelope + limit2);
            }
        }

        ProcessStatus::Normal
    }
}

// fn c2z(s: f32) -> f32 {
//     if s.abs() > 1.0 {
//         return s / s.abs();
//     }
//     s
// }

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
