#![allow(dead_code)]
use atomic_float::AtomicF32;
use nih_plug::prelude::Editor;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::vizia::vg::{Color, LineCap, LineJoin, Paint, Path};
use nih_plug_vizia::widgets::*;
use nih_plug_vizia::{assets, create_vizia_editor, ViziaState, ViziaTheming};
use std::array;
use std::collections::VecDeque;
use std::sync::{atomic::Ordering, Arc};

use crate::easing::Ease;
use crate::{build_envelope, Limit2zeroParams};

const FUNC_STYLE: &str = r#"
    function-graph {
        background-color: #2e2e2e;
        border-radius: 3px;
        border-width: 1px;
        border-color: #4e4e4e;
    }
    .scrollbar {
        display: none;
    }
    .gain-reduction-todo {
        alignment: center;
    }
"#;

#[derive(Lens, Data, Clone)]
struct GUIData {
    params: Arc<Limit2zeroParams>,
    attack: Vec<(f32, f32)>,
    release: Vec<(f32, f32)>,
    gr_atomics: GRAtomics,
    gr_buffer: GRBuffer,
}

#[derive(Debug, Clone)]
struct GRAtomics {
    pre: [Arc<AtomicF32>; 2],
    post: [Arc<AtomicF32>; 2],
    env: [Arc<AtomicF32>; 2],
}

#[derive(Debug, Clone)]
struct GRBuffer {
    pre: VecDeque<[f32; 2]>,
    post: VecDeque<[f32; 2]>,
    env: VecDeque<[f32; 2]>,
}

impl Default for GRBuffer {
    fn default() -> Self {
        Self {
            pre: VecDeque::from_iter((0..300).map(|_| [-100.0; 2])),
            post: VecDeque::from_iter((0..300).map(|_| [-100.0; 2])),
            env: VecDeque::from_iter((0..300).map(|_| [0.0; 2])),
        }
    }
}

enum GUIEvent {
    UpdateEnvelopes,
    UpdateGRVizulization,
}

impl GUIData {
    pub fn update_functions(&mut self) {
        self.attack = generate_attack_graph(&self.params, 100);
        self.release = generate_release_graph(&self.params, 100);
    }

    pub fn update_buffers(&mut self) {
        let pre = array::from_fn(|i| self.gr_atomics.pre[i].swap(0.0, Ordering::Relaxed));
        let post = array::from_fn(|i| self.gr_atomics.post[i].swap(0.0, Ordering::Relaxed));
        let env = array::from_fn(|i| self.gr_atomics.env[i].swap(0.0, Ordering::Relaxed));

        self.gr_buffer.pre.pop_front();
        self.gr_buffer.post.pop_front();
        self.gr_buffer.env.pop_front();

        self.gr_buffer.pre.push_back(pre);
        self.gr_buffer.post.push_back(post);
        self.gr_buffer.env.push_back(env);
    }
}

impl Model for GUIData {
    fn event(&mut self, _cx: &mut EventContext, event: &mut Event) {
        event.map(|app_event, _| match app_event {
            GUIEvent::UpdateEnvelopes => self.update_functions(),
            GUIEvent::UpdateGRVizulization => self.update_buffers(),
        });
    }
}

impl Data for GRAtomics {
    fn same(&self, _: &Self) -> bool {
        true
    }
}

impl Data for GRBuffer {
    fn same(&self, other: &Self) -> bool {
        if self.pre.len() != other.pre.len() {
            return false;
        }
        for i in 0..self.pre.len() {
            let pre = self.pre[i] != other.pre[i];
            let post = self.post[i] != other.post[i];
            let reduction = self.env[i] != other.env[i];

            if pre || post || reduction {
                return false;
            }
        }
        true
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

struct GRVizualization;
impl GRVizualization {
    pub fn new(cx: &'_ mut Context) -> Handle<'_, Self> {
        cx.add_timer(
            Duration::from_secs_f32(1.0 / (60.0 - f32::EPSILON)),
            None,
            |cx, reason| match reason {
                TimerAction::Tick(_) => cx.emit(GUIEvent::UpdateGRVizulization),
                _ => (),
            },
        );
        GRVizualization.build(cx, |_| {})
    }
}
impl View for GRVizualization {
    fn element(&self) -> Option<&'static str> {
        Some("limit2zero-meter")
    }

    fn draw(&self, cx: &mut DrawContext, canvas: &mut Canvas) {
        let points = GUIData::gr_buffer.0.get(cx);

        let bounds = cx.bounds();
        let db_resolution = 100.0;

        let mut path_pre = [Path::new(), Path::new()];
        let mut path_post = [Path::new(), Path::new()];
        let mut path_env = [Path::new(), Path::new()];

        for i in 0..points.pre.len() {
            let x = (i as f32 / points.pre.len() as f32) * bounds.w + bounds.x;

            for (channel, y) in points.pre[i].iter().enumerate() {
                let y = y.clamp(0.0, -1.0 * db_resolution) / db_resolution;
                let y = y * bounds.h + bounds.y;
                if i == 0 {
                    path_pre[channel].move_to(x, y);
                } else {
                    path_pre[channel].line_to(x, y);
                }
            }
            for (channel, y) in points.post[i].iter().enumerate() {
                let y = y.clamp(0.0, -1.0 * db_resolution) / db_resolution;
                let y = y * bounds.h + bounds.y;
                if i == 0 {
                    path_post[channel].move_to(x, y);
                } else {
                    path_post[channel].line_to(x, y);
                }
            }
            for (channel, y) in points.env[i].iter().enumerate() {
                let y = y.clamp(0.0, -1.0 * db_resolution) / db_resolution;
                let y = y * bounds.h + bounds.y;
                if i == 0 {
                    path_env[channel].move_to(x, y);
                } else {
                    path_env[channel].line_to(x, y);
                }
            }
        }

        let mut paint_pre = Paint::color(Color::rgb(77, 205, 102));
        paint_pre.set_line_width(2.0);
        paint_pre.set_line_cap(LineCap::Round);
        paint_pre.set_line_join(LineJoin::Round);

        let mut paint_post = Paint::color(Color::rgb(102, 77, 205));
        paint_post.set_line_width(2.0);
        paint_post.set_line_cap(LineCap::Round);
        paint_post.set_line_join(LineJoin::Round);

        let mut paint_env = Paint::color(Color::rgb(205, 77, 102));
        paint_env.set_line_width(2.0);
        paint_env.set_line_cap(LineCap::Round);
        paint_env.set_line_join(LineJoin::Round);

        for i in 0..path_pre.len() {
            canvas.stroke_path(&path_pre[i], &paint_pre);
            canvas.stroke_path(&path_post[i], &paint_post);
            canvas.stroke_path(&path_env[i], &paint_env);
        }
    }
}

pub(crate) fn create(
    params: Arc<Limit2zeroParams>,
    pre: [Arc<AtomicF32>; 2],
    post: [Arc<AtomicF32>; 2],
    reduction: [Arc<AtomicF32>; 2],
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, ViziaTheming::Custom, move |cx, _| {
        assets::register_noto_sans_light(cx);
        assets::register_noto_sans_thin(cx);

        GUIData {
            params: params.clone(),
            attack: generate_attack_graph(&params.clone(), 100),
            release: generate_release_graph(&params.clone(), 100),
            gr_atomics: GRAtomics {
                pre: pre.clone(),
                post: post.clone(),
                env: reduction.clone(),
            },
            gr_buffer: GRBuffer::default(),
        }
        .build(cx);

        cx.add_stylesheet(FUNC_STYLE).unwrap();

        VStack::new(cx, |cx| {
            GRVizualization::new(cx);
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
                            .height(Stretch(1.0))
                            .border_width(Pixels(1.0));
                        Label::new(cx, "Todo: GR View")
                            .class("gain-reduction-todo")
                            .width(Stretch(1.0))
                            .height(Stretch(1.0));
                        FunctionGraph::Release
                            .build(cx, |_| {})
                            .width(Stretch(1.0))
                            .height(Stretch(1.0))
                            .border_width(Pixels(1.0));
                    })
                    .height(Percentage(25.0));
                    HStack::new(cx, |cx| {
                        ScrollView::new(cx, 0.0, 0.0, false, true, |cx| {
                            VStack::new(cx, |cx| {
                                Label::new(cx, "lookahead");
                                ParamSlider::new(cx, GUIData::params, |params| &params.lookahead)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "lookahead_accuracy");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.lookahead_accuracy
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "attack_amt");
                                ParamSlider::new(cx, GUIData::params, |params| &params.attack_amt)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_linearity");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_linearity
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_center");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_center
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_power_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_power_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_power_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_power_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_polarity_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_polarity_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_polarity_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_polarity_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_smooth_amt");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_smooth_amt
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_sm_power_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_sm_power_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_sm_power_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_sm_power_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_sm_polarity_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_sm_polarity_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "atk_env_sm_polarity_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.atk_env_sm_polarity_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                            })
                            .height(Auto);
                        });
                        ScrollView::new(cx, 0.0, 0.0, false, true, |cx| {
                            VStack::new(cx, |cx| {
                                Label::new(cx, "hold");
                                ParamSlider::new(cx, GUIData::params, |params| &params.hold)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "release");
                                ParamSlider::new(cx, GUIData::params, |params| &params.release)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "release_amt");
                                ParamSlider::new(cx, GUIData::params, |params| &params.release_amt)
                                    .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_linearity");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_linearity
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_center");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_center
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_power_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_power_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_power_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_power_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_polarity_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_polarity_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_polarity_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_polarity_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_smooth_amt");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_smooth_amt
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_sm_power_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_sm_power_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_sm_power_out");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_sm_power_out
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
                                Label::new(cx, "rel_env_sm_polarity_in");
                                ParamSlider::new(cx, GUIData::params, |params| {
                                    &params.rel_env_sm_polarity_in
                                })
                                .on_mouse_move(|cx, _, _| cx.emit(GUIEvent::UpdateEnvelopes));
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
