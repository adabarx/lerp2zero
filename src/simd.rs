use core::array;
use core::f32;
use std::simd::{
    cmp::SimdPartialOrd,
    num::{SimdFloat, SimdInt, SimdUint},
    LaneCount, Simd, StdFloat, SupportedLaneCount,
};

pub fn simd_max_reduction<const L: usize>(db_buffer: Vec<Simd<f32, L>>, len: f32) -> f32
where
    LaneCount<L>: SupportedLaneCount,
{
    let mut max_vec = Simd::splat(0.0);

    let mut base_index = 1;

    for vec in db_buffer {
        let peak = vec.simd_gt(Simd::splat(0.0));
        if peak.any() {
            let idx_vec = Simd::from_array(array::from_fn(|i| (base_index + i) as f32));
            let t = idx_vec / Simd::splat(len);
            let lerped = simd_lerp(Simd::splat(0.0), vec, t);
            let max_mask = max_vec.simd_gt(lerped);
            max_vec = max_mask.select(max_vec, lerped)
        }
        base_index += L;
    }

    -1.0 * max_vec.reduce_max() // mult by -1 to turn into reduction
}

fn simd_lerp<const L: usize>(a: Simd<f32, L>, b: Simd<f32, L>, t: Simd<f32, L>) -> Simd<f32, L>
where
    LaneCount<L>: SupportedLaneCount,
{
    a + (b - a) * t
}

fn powf_approx<const LANES: usize>(x: Simd<f32, LANES>, y: Simd<f32, LANES>) -> Simd<f32, LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    exp2_approx(y * log2_approx(x))
}

fn log2_approx<const LANES: usize>(x: Simd<f32, LANES>) -> Simd<f32, LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    let xi = x.to_bits();
    let exponent = ((xi >> 23) & Simd::splat(0xFF)).cast::<i32>() - Simd::splat(127);
    let e = exponent.cast::<f32>();

    let mantissa =
        Simd::<f32, LANES>::from_bits((xi & Simd::splat(0x007FFFFF)) | Simd::splat(127 << 23));
    let m = mantissa - Simd::splat(1.0);
    let m2 = m * m;

    let log2_m = m * Simd::splat(1.3466) - m2 * Simd::splat(0.3607);

    e + log2_m
}

fn exp2_approx<const LANES: usize>(x: Simd<f32, LANES>) -> Simd<f32, LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    let floor = x.floor();
    let fract = x - floor;

    let exp_int = ((floor.cast::<i32>() + Simd::splat(127)) << 23).cast::<u32>();
    let scale = Simd::from_bits(exp_int);

    let ln2 = Simd::splat(f32::consts::LN_2);
    let x1 = fract * ln2;
    let x2 = x1 * x1;
    let poly = Simd::splat(1.0) + x1 + x2 * Simd::splat(0.5);

    scale * poly
}
