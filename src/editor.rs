#![allow(dead_code)]
use nih_plug::prelude::Editor;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::vizia::vg::{Color, LineCap, LineJoin, Paint, Path};
use nih_plug_vizia::widgets::*;
use nih_plug_vizia::{assets, create_vizia_editor, ViziaState, ViziaTheming};
use std::sync::Arc;

use crate::easing::Ease;
use crate::{build_envelope, Limit2zeroParams};

const FUNC_STYLE: &str = r#"
    .function-graph {
        background-color: #2e2e2e;
        border-radius: 3px;
        border-width: 1px;
        border-color: #4e4e4e;
    }
    .scrollbar {
        display: none;
    }
"#;

#[derive(Lens, Data, Clone)]
struct GUIData {
    params: Arc<Limit2zeroParams>,
    attack: Vec<(f32, f32)>,
    release: Vec<(f32, f32)>,
}

enum GUIEvent {
    UpdateGraphs,
    Null,
}

impl Model for GUIData {
    fn event(&mut self, _cx: &mut EventContext, event: &mut Event) {
        event.map(|app_event, _| {
            if let GUIEvent::UpdateGraphs = app_event {
                self.attack = generate_attack_graph(&self.params, 100);
                self.release = generate_release_graph(&self.params, 100);
            }
        });
    }
}

enum FunctionGraph {
    Attack,
    Release,
}

impl View for FunctionGraph {
    fn element(&self) -> Option<&'static str> {
        Some("function-graph")
    }

    fn draw(&self, cx: &mut DrawContext, canvas: &mut Canvas) {
        let points = match self {
            FunctionGraph::Attack => GUIData::attack.0.get(cx),
            FunctionGraph::Release => GUIData::release.0.get(cx),
        };

        if points.len() < 2 {
            return;
        }

        let bounds = cx.bounds();
        let wh = f32::min(bounds.h, bounds.w);

        let (x_offset, y_offset) = if wh < bounds.h {
            let crest = bounds.h - wh;
            (0.0, crest / 2.0)
        } else if wh < bounds.w {
            let crest = bounds.w - wh;
            (crest / 2.0, 0.0)
        } else {
            (0.0, 0.0)
        };

        let mut path = Path::new();
        for (i, (x, y)) in points.iter().enumerate() {
            let mut px = x * wh;
            let mut py = wh - (y * wh);
            px += bounds.x + x_offset;
            py += bounds.y + y_offset;
            if i == 0 {
                path.move_to(px, py);
            } else {
                path.line_to(px, py);
            }
        }

        let mut paint = Paint::color(Color::rgb(77, 205, 102));
        paint.set_line_width(2.0);
        paint.set_line_cap(LineCap::Round);
        paint.set_line_join(LineJoin::Round);

        canvas.stroke_path(&path, &paint);
    }
}

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::new(|| (800, 800))
}

fn generate_release_graph(params: &Limit2zeroParams, resolution: usize) -> Vec<(f32, f32)> {
    let mut points = Vec::with_capacity(resolution);

    let envelope = build_envelope(
        params.rel_env_linearity.value(),
        params.rel_env_center.value(),
        params.rel_smooth_amt.value(),
        params.rel_env_polarity_in.value(),
        params.rel_env_polarity_out.value(),
        params.rel_env_power_in.value(),
        params.rel_env_power_out.value(),
        params.rel_env_sm_polarity_in.value(),
        params.rel_env_sm_polarity_out.value(),
        params.rel_env_sm_power_in.value(),
        params.rel_env_sm_power_out.value(),
    );

    for i in 0..=resolution {
        let x = i as f32 / resolution as f32;
        points.push((x, envelope.process(x)));
    }

    points
}

fn generate_attack_graph(params: &Limit2zeroParams, resolution: usize) -> Vec<(f32, f32)> {
    let mut points = Vec::with_capacity(resolution);

    let envelope = build_envelope(
        params.atk_env_linearity.value(),
        params.atk_env_center.value(),
        params.atk_smooth_amt.value(),
        params.atk_env_polarity_in.value(),
        params.atk_env_polarity_out.value(),
        params.atk_env_power_in.value(),
        params.atk_env_power_out.value(),
        params.atk_env_sm_polarity_in.value(),
        params.atk_env_sm_polarity_out.value(),
        params.atk_env_sm_power_in.value(),
        params.atk_env_sm_power_out.value(),
    );

    for i in 0..=resolution {
        let x = i as f32 / resolution as f32;
        points.push((x, 1.0 - envelope.process(x)));
    }

    points
}

pub(crate) fn create(
    params: Arc<Limit2zeroParams>,
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, ViziaTheming::Custom, move |cx, _| {
        assets::register_noto_sans_light(cx);
        assets::register_noto_sans_thin(cx);

        GUIData {
            params: params.clone(),
            attack: generate_attack_graph(&params.clone(), 100),
            release: generate_release_graph(&params.clone(), 100),
        }
        .build(cx);

        cx.add_stylesheet(FUNC_STYLE).unwrap();

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
                    ParamSlider::new(cx, GUIData::params, |params| &params.drive);
                    ParamButton::new(cx, GUIData::params, |params| &params.compensate);
                    Label::new(cx, "stereo_link");
                    ParamSlider::new(cx, GUIData::params, |params| &params.stereo_link);
                    Label::new(cx, "trim");
                    ParamSlider::new(cx, GUIData::params, |params| &params.trim);
                })
                .width(Percentage(25.0));
                VStack::new(cx, |cx| {
                    HStack::new(cx, |cx| {
                        FunctionGraph::Attack
                            .build(cx, |_| {})
                            .width(Stretch(1.0))
                            .height(Stretch(1.0));
                        FunctionGraph::Release
                            .build(cx, |_| {})
                            .width(Stretch(1.0))
                            .height(Stretch(1.0));
                    })
                    .height(Percentage(25.0));
                    HStack::new(cx, |cx| {
                        ScrollView::new(cx, 0.0, 0.0, false, true, |cx| {
                            VStack::new(cx, |cx| {
                                Label::new(cx, "lookahead");
                                ParamSlider::new(cx, GUIData::params, |params| &params.lookahead)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "lookahead_accuracy");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.lookahead_accuracy
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "attack_amt");
                                ParamSlider::new(cx, GUIData::params, |params| &params.attack_amt)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_linearity");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_linearity
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_center");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_center
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_power_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_power_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_power_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_power_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_polarity_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_polarity_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_polarity_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_polarity_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_smooth_amt");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_smooth_amt
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_sm_power_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_sm_power_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_sm_power_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_sm_power_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_sm_polarity_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_sm_polarity_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "atk_env_sm_polarity_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_sm_polarity_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                            })
                            .height(Auto);
                        });
                        ScrollView::new(cx, 0.0, 0.0, false, true, |cx| {
                            VStack::new(cx, |cx| {
                                Label::new(cx, "hold");
                                ParamSlider::new(cx, GUIData::params, |params| &params.hold)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "release");
                                ParamSlider::new(cx, GUIData::params, |params| &params.release)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "release_amt");
                                ParamSlider::new(cx, GUIData::params, |params| &params.release_amt)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_linearity");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_linearity
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_center");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_center
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_power_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_power_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_power_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_power_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_polarity_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_polarity_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_polarity_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_polarity_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_smooth_amt");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_smooth_amt
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_sm_power_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_sm_power_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_sm_power_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_sm_power_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_sm_polarity_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_sm_polarity_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateGraphs));
                                Label::new(cx, "rel_env_sm_polarity_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_sm_polarity_out
                                });
                            })
                            .height(Auto);
                        });
                    });
                });
            });
        });
        ResizeHandle::new(cx);
    })
}
