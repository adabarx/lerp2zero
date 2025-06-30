use core::f32;
use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use std::{collections::VecDeque, sync::Arc};

mod easing;
mod editor;

use easing::{Ease, EaseIn, EaseOut, Linear, LinearBlend, SCurve};

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
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,

    #[id = "drive"]
    pub drive: FloatParam,

    #[id = "trim"]
    pub trim: FloatParam,

    #[id = "lookahead"]
    pub lookahead: FloatParam,

    #[id = "lookahead_accuracy"]
    pub lookahead_accuracy: IntParam,

    #[id = "attack_amt"]
    pub attack_amt: FloatParam,

    #[id = "atk_env_linearity"]
    pub atk_env_linearity: FloatParam,

    #[id = "atk_env_s_center"]
    pub atk_env_center: FloatParam,

    #[id = "atk_env_polarity_in"]
    pub atk_env_polarity_in: FloatParam,

    #[id = "atk_env_polarity_out"]
    pub atk_env_polarity_out: FloatParam,

    #[id = "atk_env_power_in"]
    pub atk_env_power_in: FloatParam,

    #[id = "atk_env_power_out"]
    pub atk_env_power_out: FloatParam,

    #[id = "atk_smooth_amt"]
    pub atk_smooth_amt: FloatParam,

    #[id = "atk_env_smooth_polarity_in"]
    pub atk_env_sm_polarity_in: FloatParam,

    #[id = "atk_env_smooth_polarity_out"]
    pub atk_env_sm_polarity_out: FloatParam,

    #[id = "atk_env_smooth_power_in"]
    pub atk_env_sm_power_in: FloatParam,

    #[id = "atk_env_smooth_power_out"]
    pub atk_env_sm_power_out: FloatParam,

    #[id = "hold"]
    pub hold: FloatParam,

    #[id = "release"]
    pub release: FloatParam,

    #[id = "release_amt"]
    pub release_amt: FloatParam,

    #[id = "rel_linearity"]
    pub rel_env_linearity: FloatParam,

    #[id = "rel_env_s_center"]
    pub rel_env_center: FloatParam,

    #[id = "rel_env_polarity_in"]
    pub rel_env_polarity_in: FloatParam,

    #[id = "rel_env_polarity_out"]
    pub rel_env_polarity_out: FloatParam,

    #[id = "rel_env_power_in"]
    pub rel_env_power_in: FloatParam,

    #[id = "rel_env_power_out"]
    pub rel_env_power_out: FloatParam,

    #[id = "rel_smooth_amt"]
    pub rel_smooth_amt: FloatParam,

    #[id = "rel_env_smooth_polarity_in"]
    pub rel_env_sm_polarity_in: FloatParam,

    #[id = "rel_env_smooth_polarity_out"]
    pub rel_env_sm_polarity_out: FloatParam,

    #[id = "rel_env_smooth_power_in"]
    pub rel_env_sm_power_in: FloatParam,

    #[id = "rel_env_smooth_power_out"]
    pub rel_env_sm_power_out: FloatParam,

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
    state: Vec<EnvState>,
    target: Vec<f32>,
    hold: Vec<f32>,
    envelope: Vec<f32>,
    current_peaks: CurrentPeaks,
}

struct CurrentPeaks {
    db: Vec<f32>,
    position: Vec<f32>,
    lerp_len: Vec<f32>,
}

struct CurrentPeakSingleMut<'a> {
    db: &'a mut f32,
    position: &'a mut f32,
    lerp_len: &'a mut f32,
}

impl CurrentPeaks {
    fn get_mut(&mut self, channel: usize) -> CurrentPeakSingleMut<'_> {
        if channel >= self.db.len() {
            panic!("outta bounds");
        }
        CurrentPeakSingleMut {
            db: self.db.get_mut(channel).unwrap(),
            position: self.position.get_mut(channel).unwrap(),
            lerp_len: self.lerp_len.get_mut(channel).unwrap(),
        }
    }
}

impl<'a> CurrentPeakSingleMut<'a> {
    fn read(&mut self, ease: impl Ease) -> Option<f32> {
        *self.position += 1.0;
        let progress = (*self.position + 1.0) / (*self.lerp_len + 1.0);
        if progress > 1.0 {
            *self.position -= 1.0;
            return None;
        }
        Some(calc_atk_reduction(*self.db, ease.process(progress)))
    }
}

struct Limiter<'a> {
    buffer: &'a mut VecDeque<SampleDB>,
    state: &'a mut EnvState,
    target: &'a mut f32,
    hold: &'a mut f32,
    envelope: &'a mut f32,
    current_peak: CurrentPeakSingleMut<'a>,
}

impl LimiterBuffer {
    fn new(channels: usize, sample_len: usize) -> Self {
        let mut rv = LimiterBuffer {
            channels,
            buffers: vec![VecDeque::with_capacity(sample_len); channels],
            state: vec![EnvState::Off; channels],
            target: vec![0.0; channels],
            hold: vec![0.0; channels],
            envelope: vec![0.0; channels],
            current_peaks: CurrentPeaks {
                db: vec![0.0; channels],
                position: vec![2.0; channels],
                lerp_len: vec![1.0; channels],
            },
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

    fn get_mut(&'_ mut self, channel: usize) -> Limiter<'_> {
        let channel = channel.clamp(0, self.channels - 1);
        Limiter {
            buffer: self.buffers.get_mut(channel).unwrap(),
            state: self.state.get_mut(channel).unwrap(),
            target: self.target.get_mut(channel).unwrap(),
            hold: self.hold.get_mut(channel).unwrap(),
            envelope: self.envelope.get_mut(channel).unwrap(),
            current_peak: self.current_peaks.get_mut(channel),
        }
    }
}

impl Default for Limit2zeroParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),

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
                    max: 50.0,
                    factor: 0.5,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                if value < 1.01 {
                    format!("{} samples", (value * 48.0).ceil() as usize)
                } else {
                    format!("{:.1}ms", value)
                }
            })),

            lookahead_accuracy: IntParam::new(
                "Lookahead Accuracy",
                1,
                IntRange::Linear { min: 1, max: 16 },
            )
            .with_value_to_string(Arc::new(move |value| match value {
                1 => "every sample".to_string(),
                2 => "every other sample".to_string(),
                _ => format!("every {} samples", value),
            })),

            attack_amt: FloatParam::new(
                "Attack Amount",
                1.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_value_to_string(Arc::new(move |value| {
                if value <= 4.0 {
                    let value = value.exp2();
                    if value < 10.0 {
                        format!("{:.1}:1", value)
                    } else {
                        format!("{:.0}:1", value)
                    }
                } else {
                    let diff = (value - 4.0).powi(3);
                    let value = (value + diff).exp2();

                    if value > 50.0 {
                        format!("inf:1")
                    } else {
                        format!("{:.0}:1", value)
                    }
                }
            })),

            atk_env_linearity: FloatParam::new(
                "Attack Linearity",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            atk_env_polarity_in: FloatParam::new(
                "Attack Polarity In",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            atk_env_polarity_out: FloatParam::new(
                "Attack Polarity Out",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            atk_env_power_in: FloatParam::new(
                "Attack Power In",
                2.0,
                FloatRange::Skewed {
                    min: 16_f32.recip(),
                    max: 16.0,
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let one_over_value = value.recip();
                if one_over_value.round() > 1.0 {
                    if one_over_value >= 10.0 {
                        format!("1/{:.0}", one_over_value)
                    } else {
                        format!("1/{:.1}", one_over_value)
                    }
                } else {
                    if value >= 10.0 {
                        format!("{:.0}", value)
                    } else {
                        format!("{:.1}", value)
                    }
                }
            })),

            atk_env_power_out: FloatParam::new(
                "Attack Power Out",
                2.0,
                FloatRange::Skewed {
                    min: 16_f32.recip(),
                    max: 16.0,
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let one_over_value = value.recip();
                if one_over_value.round() > 1.0 {
                    if one_over_value >= 10.0 {
                        format!("1/{:.0}", one_over_value)
                    } else {
                        format!("1/{:.1}", one_over_value)
                    }
                } else {
                    if value >= 10.0 {
                        format!("{:.0}", value)
                    } else {
                        format!("{:.1}", value)
                    }
                }
            })),

            atk_env_center: FloatParam::new(
                "Atk S Center",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            atk_smooth_amt: FloatParam::new(
                "Attack Smooth Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            atk_env_sm_polarity_in: FloatParam::new(
                "Attack Smooth Polarity In",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            atk_env_sm_polarity_out: FloatParam::new(
                "Attack Smooth Polarity Out",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            atk_env_sm_power_in: FloatParam::new(
                "Attack Smooth Power In",
                2.0,
                FloatRange::Skewed {
                    min: 16_f32.recip(),
                    max: 16.0,
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let one_over_value = value.recip();
                if one_over_value.round() > 1.0 {
                    if one_over_value >= 10.0 {
                        format!("1/{:.0}", one_over_value)
                    } else {
                        format!("1/{:.1}", one_over_value)
                    }
                } else {
                    if value >= 10.0 {
                        format!("{:.0}", value)
                    } else {
                        format!("{:.1}", value)
                    }
                }
            })),

            atk_env_sm_power_out: FloatParam::new(
                "Attack Smooth Power Out",
                2.0,
                FloatRange::Skewed {
                    min: 16_f32.recip(),
                    max: 16.0,
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let one_over_value = value.recip();
                if one_over_value.round() > 1.0 {
                    if one_over_value >= 10.0 {
                        format!("1/{:.0}", one_over_value)
                    } else {
                        format!("1/{:.1}", one_over_value)
                    }
                } else {
                    if value >= 10.0 {
                        format!("{:.0}", value)
                    } else {
                        format!("{:.1}", value)
                    }
                }
            })),

            hold: FloatParam::new(
                "Hold",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 1000.,
                    factor: 0.375,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                if value < 1.0 {
                    format!("{} samples", (value * 48.0).ceil() as usize)
                } else if value < 10.0 {
                    format!("{:.2}ms", value)
                } else if value < 100.0 {
                    format!("{:.1}ms", value)
                } else if value < 1000.0 {
                    format!("{:.0}ms", value)
                } else {
                    let value = value / 1000.0;
                    format!("{:.0}s", value)
                }
            })),

            release_amt: FloatParam::new(
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
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                if value < 1.0 {
                    format!("{} samples", (value * 48.0).ceil() as usize)
                } else if value < 10.0 {
                    format!("{:.2}ms", value)
                } else if value < 100.0 {
                    format!("{:.1}ms", value)
                } else if value < 1000.0 {
                    format!("{:.0}ms", value)
                } else {
                    let value = value / 1000.0;
                    format!("{:.2}s", value)
                }
            })),

            rel_env_linearity: FloatParam::new(
                "Release Linearity",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            rel_env_polarity_in: FloatParam::new(
                "Release Polarity In",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            rel_env_polarity_out: FloatParam::new(
                "Release Polarity Out",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            rel_env_power_in: FloatParam::new(
                "Release Power In",
                2.0,
                FloatRange::Skewed {
                    min: 16_f32.recip(),
                    max: 16.0,
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let one_over_value = value.recip();
                if one_over_value.round() > 1.0 {
                    if one_over_value >= 10.0 {
                        format!("1/{:.0}", one_over_value)
                    } else {
                        format!("1/{:.1}", one_over_value)
                    }
                } else {
                    if value >= 10.0 {
                        format!("{:.0}", value)
                    } else {
                        format!("{:.1}", value)
                    }
                }
            })),

            rel_env_power_out: FloatParam::new(
                "Release Power Out",
                2.0,
                FloatRange::Skewed {
                    min: 16_f32.recip(),
                    max: 16.0,
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let one_over_value = value.recip();
                if one_over_value.round() > 1.0 {
                    if one_over_value >= 10.0 {
                        format!("1/{:.0}", one_over_value)
                    } else {
                        format!("1/{:.1}", one_over_value)
                    }
                } else {
                    if value >= 10.0 {
                        format!("{:.0}", value)
                    } else {
                        format!("{:.1}", value)
                    }
                }
            })),

            rel_env_center: FloatParam::new(
                "rel S Center",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            rel_smooth_amt: FloatParam::new(
                "Release Smooth Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            rel_env_sm_polarity_in: FloatParam::new(
                "Release Polarity In",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            rel_env_sm_polarity_out: FloatParam::new(
                "Release Polarity Out",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            rel_env_sm_power_in: FloatParam::new(
                "Release Smooth Power In",
                2.0,
                FloatRange::Skewed {
                    min: 16_f32.recip(),
                    max: 16.0,
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let one_over_value = value.recip();
                if one_over_value.round() > 1.0 {
                    if one_over_value >= 10.0 {
                        format!("1/{:.0}", one_over_value)
                    } else {
                        format!("1/{:.1}", one_over_value)
                    }
                } else {
                    if value >= 10.0 {
                        format!("{:.0}", value)
                    } else {
                        format!("{:.1}", value)
                    }
                }
            })),

            rel_env_sm_power_out: FloatParam::new(
                "Release Smooth Power Out",
                2.0,
                FloatRange::Skewed {
                    min: 16_f32.recip(),
                    max: 16.0,
                    factor: 0.25,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let one_over_value = value.recip();
                if one_over_value.round() > 1.0 {
                    if one_over_value >= 10.0 {
                        format!("1/{:.0}", one_over_value)
                    } else {
                        format!("1/{:.1}", one_over_value)
                    }
                } else {
                    if value >= 10.0 {
                        format!("{:.0}", value)
                    } else {
                        format!("{:.1}", value)
                    }
                }
            })),

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

fn build_envelope(
    linearity: f32,
    center: f32,
    smooth_amount: f32,
    pol_i: f32,
    pol_o: f32,
    pow_i: f32,
    pow_o: f32,
    sm_pol_i: f32,
    sm_pol_o: f32,
    sm_pow_i: f32,
    sm_pow_o: f32,
) -> LinearBlend<SCurve<SCurve<Linear>>> {
    let linear_smoothing_factor = (1.0 - smooth_amount) * linearity.powi(2);
    LinearBlend::new(
        SCurve::new(
            EaseIn::new(pol_i, pow_i),
            EaseOut::new(pol_o, pow_o),
            center,
            smooth_amount + linear_smoothing_factor,
            SCurve::new(
                EaseIn::new(sm_pol_i, sm_pow_i),
                EaseOut::new(sm_pol_o, sm_pow_o),
                0.5,
                0.25 * linearity.powi(2),
                Linear,
            ),
        ),
        linearity,
    )
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

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.params.editor_state.clone())
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
        let atk_env = build_envelope(
            self.params.atk_env_linearity.value(),
            self.params.atk_env_center.value(),
            self.params.atk_smooth_amt.value(),
            self.params.atk_env_polarity_in.value(),
            self.params.atk_env_polarity_out.value(),
            self.params.atk_env_power_in.value(),
            self.params.atk_env_power_out.value(),
            self.params.atk_env_sm_polarity_in.value(),
            self.params.atk_env_sm_polarity_out.value(),
            self.params.atk_env_sm_power_in.value(),
            self.params.atk_env_sm_power_out.value(),
        );
        let rel_env = build_envelope(
            self.params.rel_env_linearity.value(),
            self.params.rel_env_center.value(),
            self.params.rel_smooth_amt.value(),
            self.params.rel_env_polarity_in.value(),
            self.params.rel_env_polarity_out.value(),
            self.params.rel_env_power_in.value(),
            self.params.rel_env_power_out.value(),
            self.params.rel_env_sm_polarity_in.value(),
            self.params.rel_env_sm_polarity_out.value(),
            self.params.rel_env_sm_power_in.value(),
            self.params.rel_env_sm_power_out.value(),
        );

        let (input, trim) = (self.params.drive.value(), self.params.trim.value());

        let (lookahead, atk_amt) = (
            self.params.lookahead.value() * 0.001 * self.sample_rate,
            self.params.attack_amt.value(),
        );

        let (hold, release_amt) = (
            self.params.hold.value() * 0.001 * self.sample_rate,
            self.params.release_amt.value(),
        );

        let release = self.params.release.value() * 0.001 * self.sample_rate;

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

                limiter.buffer.push_back(new_sample);

                // do stuff based on envelope state
                match &mut limiter.state {
                    EnvState::Hold(elapsed) => {
                        if *elapsed == 0.0 {
                            *limiter.target = *limiter.hold;
                            *limiter.envelope = *limiter.hold;
                        }
                        *elapsed += 1.0;
                        if *elapsed >= (hold + 1.0) {
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
                        let t = *elapsed / (release + 1.0);

                        // NOTE: calc_rel_reduction
                        *limiter.envelope = lerp(*limiter.target, 0.0, rel_env.process(t));

                        if *elapsed >= (release + 1.0) {
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

                // search buffer for peaks and calc atk env
                // or
                // calculate atk envelope using the last known peak
                let la_acc = self.params.lookahead_accuracy.value();
                let mut atk_reduction = 0.0;
                if self.lookahead_len >= 1.0 && sample_id as i32 % la_acc == 0 {
                    let mut db = 0.0;
                    let mut position = 0.0;
                    let mut curr_reduct = 0.0;

                    for (i, sample) in limiter
                        .buffer
                        .iter()
                        .rev()
                        .enumerate()
                        .filter(|x| x.1.peak())
                    {
                        let t = atk_env.process((i + 1) as f32 / (self.lookahead_len + 1.0));
                        let reduct = calc_atk_reduction(sample.db, t);
                        if reduct < curr_reduct {
                            curr_reduct = reduct;
                            db = sample.db;
                            position = i as f32;
                        }
                    }
                    if db > 0.0 {
                        *limiter.current_peak.db = db;
                        *limiter.current_peak.position = position;
                        *limiter.current_peak.lerp_len = self.lookahead_len;
                        atk_reduction = curr_reduct * atk_amt;
                    }
                } else if let Some(reduction) = limiter.current_peak.read(atk_env) {
                    atk_reduction = reduction * atk_amt;
                }

                if atk_reduction < *limiter.envelope {
                    *limiter.target = atk_reduction;
                    *limiter.hold = atk_reduction * release_amt.sqrt();
                    *limiter.envelope = atk_reduction;
                    if hold.round() >= 1.0 {
                        *limiter.state = EnvState::Hold(0.0);
                    } else if release.round() >= 1.0 {
                        *limiter.state = EnvState::Release(0.0);
                    } else {
                        *limiter.state = EnvState::Off;
                    }
                }

                // grab delayed sample from buffer
                let delay = limiter.buffer.pop_front().unwrap();

                // if the sample is still over 0.0 after the envelope is applied,
                // clip it.
                if delay.db + *limiter.envelope > 0.0 {
                    *limiter.target = -1.0 * delay.db;
                    *limiter.hold = *limiter.target * release_amt.sqrt();
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

fn calc_atk_reduction(db: f32, t: f32) -> f32 {
    lerp(0.0, -1.0 * db, t)
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
