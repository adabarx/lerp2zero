use nih_plug::prelude::Editor;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::*;
use nih_plug_vizia::{assets, create_vizia_editor, ViziaState, ViziaTheming};
use std::sync::Arc;

use crate::Limit2zeroParams;

#[derive(Lens)]
struct Data {
    params: Arc<Limit2zeroParams>,
}

impl Model for Data {}

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::new(|| (800, 800))
}

pub(crate) fn create(
    params: Arc<Limit2zeroParams>,
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, ViziaTheming::Custom, move |cx, _| {
        assets::register_noto_sans_light(cx);
        assets::register_noto_sans_thin(cx);

        Data {
            params: params.clone(),
        }
        .build(cx);

        VStack::new(cx, |cx| {
            Label::new(cx, "Clip2Zero")
                .font_family(vec![FamilyOwned::Name(String::from(assets::NOTO_SANS))])
                .font_weight(FontWeightKeyword::Thin)
                .font_size(30.0)
                .height(Pixels(50.0))
                .child_top(Stretch(1.0))
                .child_bottom(Pixels(0.0));

            HStack::new(cx, |cx| {
                VStack::new(cx, |cx| {
                    Label::new(cx, "Drive");
                    ParamSlider::new(cx, Data::params, |params| &params.drive);
                });

                VStack::new(cx, |cx| {
                    Label::new(cx, "lookahead");
                    ParamSlider::new(cx, Data::params, |params| &params.lookahead);
                    Label::new(cx, "lookahead_accuracy");
                    ParamSlider::new(cx, Data::params, |params| &params.lookahead_accuracy);
                    Label::new(cx, "attack_amt");
                    ParamSlider::new(cx, Data::params, |params| &params.attack_amt);
                    Label::new(cx, "atk_env_linearity");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_linearity);
                    Label::new(cx, "atk_env_center");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_center);
                    Label::new(cx, "atk_env_power_in");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_power_in);
                    Label::new(cx, "atk_env_power_out");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_power_out);
                    Label::new(cx, "atk_env_polarity_in");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_polarity_in);
                    Label::new(cx, "atk_env_polarity_out");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_polarity_out);
                    Label::new(cx, "atk_smooth_amt");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_smooth_amt);
                    Label::new(cx, "atk_env_sm_power_in");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_sm_power_in);
                    Label::new(cx, "atk_env_sm_power_out");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_sm_power_out);
                    Label::new(cx, "atk_env_sm_polarity_in");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_sm_polarity_in);
                    Label::new(cx, "atk_env_sm_polarity_out");
                    ParamSlider::new(cx, Data::params, |params| &params.atk_env_sm_polarity_out);
                });
                VStack::new(cx, |cx| {
                    Label::new(cx, "hold");
                    ParamSlider::new(cx, Data::params, |params| &params.hold);
                    Label::new(cx, "release");
                    ParamSlider::new(cx, Data::params, |params| &params.release);
                    Label::new(cx, "release_amt");
                    ParamSlider::new(cx, Data::params, |params| &params.release_amt);
                    Label::new(cx, "rel_env_linearity");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_linearity);
                    Label::new(cx, "rel_env_center");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_center);
                    Label::new(cx, "rel_env_power_in");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_power_in);
                    Label::new(cx, "rel_env_power_out");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_power_out);
                    Label::new(cx, "rel_env_polarity_in");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_polarity_in);
                    Label::new(cx, "rel_env_polarity_out");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_polarity_out);
                    Label::new(cx, "rel_smooth_amt");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_smooth_amt);
                    Label::new(cx, "rel_env_sm_power_in");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_sm_power_in);
                    Label::new(cx, "rel_env_sm_power_out");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_sm_power_out);
                    Label::new(cx, "rel_env_sm_polarity_in");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_sm_polarity_in);
                    Label::new(cx, "rel_env_sm_polarity_out");
                    ParamSlider::new(cx, Data::params, |params| &params.rel_env_sm_polarity_out);
                });
                VStack::new(cx, |cx| {
                    Label::new(cx, "stereo_link");
                    ParamSlider::new(cx, Data::params, |params| &params.stereo_link);
                    Label::new(cx, "trim");
                    ParamSlider::new(cx, Data::params, |params| &params.trim);
                    ParamButton::new(cx, Data::params, |params| &params.compensate);
                });
            });
        });

        ResizeHandle::new(cx);
    })
}
