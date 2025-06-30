fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub trait Ease {
    fn process(&self, x: f32) -> f32;
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Linear;

impl Ease for Linear {
    fn process(&self, x: f32) -> f32 {
        x
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct LinearBlend<T: Ease> {
    curve: T,
    linearity: f32,
}

impl<T: Ease> LinearBlend<T> {
    pub fn new(curve: T, linearity: f32) -> Self {
        Self { curve, linearity }
    }
}

impl<T: Ease> Ease for LinearBlend<T> {
    fn process(&self, x: f32) -> f32 {
        lerp(self.curve.process(x), x, self.linearity)
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct SCurve<T: Ease> {
    ease_in: EaseIn,
    ease_out: EaseOut,
    center: f32,
    smoothing: f32, // 0.0 - 1.0
    sm_ease: T,
}

impl<T: Ease> SCurve<T> {
    pub fn new(
        ease_in: EaseIn,
        ease_out: EaseOut,
        center: f32,
        smoothing: f32,
        sm_ease: T,
    ) -> Self {
        Self {
            ease_in,
            ease_out,
            center,
            smoothing,
            sm_ease,
        }
    }
}

impl<T: Ease> Ease for SCurve<T> {
    fn process(&self, x: f32) -> f32 {
        // ease in  [0.0  -->  smoothing_end]
        // ease out [smoothing_start --> 1.0]

        let (len_start, len_end) = (self.center, 1.0 - self.center);

        let smoothing_start = self.center - (len_start * self.smoothing).max(f32::EPSILON);
        let smoothing_end = self.center + (len_end * self.smoothing).max(f32::EPSILON);

        let in_len = smoothing_end;
        let in_prog = x / in_len;

        let out_len = 1.0 - smoothing_start;
        let out_prog = (x - smoothing_start) / out_len;

        let mut values = [0.0, 0.0];
        if in_prog < 1.0 {
            values[0] = self.ease_in.process(in_prog) * in_len;
        }
        if out_prog > 0.0 {
            values[1] = self.ease_out.process(out_prog) * out_len;
            values[1] += smoothing_start;
        }

        if values[0] != 0.0 && values[1] != 0.0 {
            let sm_progress = (x - smoothing_start) / (smoothing_end - smoothing_start);
            lerp(values[0], values[1], self.sm_ease.process(sm_progress))
        } else {
            values.iter().sum()
        }
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct EaseOut {
    polarity: f32,
    power: f32,
}

impl EaseOut {
    pub fn new(polarity: f32, power: f32) -> Self {
        Self { polarity, power }
    }
}

impl Ease for EaseOut {
    fn process(&self, x: f32) -> f32 {
        let p = self.polarity.clamp(0.0, 1.0);
        if p == 1.0 {
            1.0 - (1.0 - x).powf(self.power)
        } else if p == 0.0 {
            x.powf(self.power.recip())
        } else {
            lerp(
                x.powf(self.power.recip()),
                1.0 - (1.0 - x).powf(self.power),
                x,
            )
        }
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct EaseIn {
    polarity: f32,
    power: f32,
}

impl EaseIn {
    pub fn new(polarity: f32, power: f32) -> Self {
        Self { polarity, power }
    }
}

impl Ease for EaseIn {
    fn process(&self, x: f32) -> f32 {
        let p = self.polarity.clamp(0.0, 1.0);
        if p == 1.0 {
            x.powf(self.power)
        } else if p == 0.0 {
            1.0 - (1.0 - x).powf(self.power.recip())
        } else {
            lerp(
                1.0 - (1.0 - x).powf(self.power.recip()),
                x.powf(self.power),
                x,
            )
        }
    }
}
