use core::f32;
use nih_plug::prelude::*;
use std::sync::Arc;

// This is a shortened version of the gain example with most comments removed, check out
// https://github.com/robbert-vdh/nih-plug/blob/master/plugins/examples/gain/src/lib.rs to get
// started

struct Limit2zero {
    params: Arc<Limit2zeroParams>,
    sample_rate: f32,
    release_len: f32,
    hold_len: f32,
    reduction: f32,
    envelope: f32,
    release_elapsed: f32,
    hold_elapsed: f32,
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
            reduction: 0.0, // db
            envelope: 0.0,
            release_len: 0.0,
            hold_len: 0.0,
            release_elapsed: f32::MAX,
            hold_elapsed: f32::MAX,
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

            lookahead: FloatParam::new("Lookahead", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
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

    // The first audio IO layout is used as the default. The other layouts may be selected either
    // explicitly or automatically by the host or the user depending on the plugin API/backend.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),

        aux_input_ports: &[],
        aux_output_ports: &[],

        // Individual ports and the layout as a whole can be named here. By default these names
        // are generated as needed. This layout will be called 'Stereo', while a layout with
        // only one input and output channel would be called 'Mono'.
        names: PortNames::const_default(),
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    // If the plugin can send or receive SysEx messages, it can define a type to wrap around those
    // messages here. The type implements the `SysExMessage` trait, which allows conversion to and
    // from plain byte buffers.
    type SysExMessage = ();
    // More advanced plugins can use this to run expensive background tasks. See the field's
    // documentation for more information. `()` means that the plugin does not have any background
    // tasks.
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
        // Reset buffers and envelopes here. This can be called from the audio thread and may not
        // allocate. You can remove this function if you do not need it.
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_samples() {
            let input = self.params.input.smoothed.next();
            let limit2 = self.params.limit2.smoothed.next();

            let release_sec = self.params.release.smoothed.next() * 0.001;
            let hold_sec = self.params.hold.smoothed.next() * 0.001;
            self.release_len = release_sec * self.sample_rate;
            self.hold_len = hold_sec * self.sample_rate;

            for sample in channel_samples {
                *sample *= input;
                let sample_db = util::gain_to_db_fast(sample.abs());

                if sample_db + self.envelope > 0.0 {
                    self.reduction = -1.0 * sample_db;
                    self.envelope = self.reduction;
                    self.hold_elapsed = 0.0;
                    *sample *= util::db_to_gain_fast(self.envelope + limit2);
                    continue;
                }

                if self.hold_len > 1.0 && self.hold_elapsed < self.hold_len {
                    self.hold_elapsed += 1.0;

                    *sample *= util::db_to_gain_fast(self.envelope + limit2);
                    continue;
                }

                if self.release_len > 1.0 && self.release_elapsed < self.release_len {
                    let t = self.release_elapsed / self.release_len;
                    self.envelope = lerp(self.reduction, 0.0, t);
                    self.release_elapsed += 1.0;

                    *sample *= util::db_to_gain_fast(self.envelope + limit2);
                    continue;
                }

                self.envelope = 0.0;

                *sample *= util::db_to_gain_fast(self.envelope + limit2);
            }
        }

        ProcessStatus::Normal
    }
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
