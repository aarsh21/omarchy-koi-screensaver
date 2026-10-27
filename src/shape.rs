//! Koi outline classification, ported from the portfolio's koi-pond `shape.ts`.

use crate::world::smoothstep;

pub const FLAG_BODY: u8 = 1;
pub const FLAG_FIN: u8 = 2;
pub const FLAG_EYE: u8 = 3;
pub const FLAG_DORSAL: u8 = 4;
pub const FLAG_BARBEL: u8 = 5;

/// Where a point landed on the koi; `du`/`da` give the fin-ray direction in body space.
#[derive(Clone, Copy, Default)]
pub struct ShapeHit {
    pub flag: u8,
    pub across: f64,
    pub du: f64,
    pub da: f64,
}

const BODY_END: f64 = 0.74;

/// Top-down outline as a fraction of length: blunt round head, plump shoulders, narrow tail stalk.
pub fn body_half_width(s: f64) -> f64 {
    if s < 0.12 {
        return 0.11 * (((s / 0.12) * std::f64::consts::PI) / 2.0).sin().powf(0.55);
    }
    if s < 0.3 {
        return 0.11 + 0.025 * smoothstep(0.12, 0.3, s);
    }
    0.033 + 0.102 * (1.0 - smoothstep(0.3, BODY_END, s))
}

/// Butterfly tail: fans out from the stalk, with a V notch in the trailing edge.
fn tail_half_width(s: f64) -> f64 {
    0.03 + 0.15 * smoothstep(0.7, 1.0, s).powf(0.8)
}

/// Test a point against a fin: an ellipse leaning back from a root on the body's edge.
fn in_fin(
    s: f64,
    a: f64,
    root_s: f64,
    angle: f64,
    reach: f64,
    width: f64,
    out: &mut ShapeHit,
) -> bool {
    let root_a = body_half_width(root_s) * 0.85;
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let du = s - root_s;
    let da = a - root_a;
    let along = du * cos_a + da * sin_a - reach;
    let perp = -du * sin_a + da * cos_a;
    if (along / reach).powi(2) + (perp / width).powi(2) > 1.0 {
        return false;
    }
    out.flag = FLAG_FIN;
    out.du = du;
    out.da = da;
    true
}

/// Classify a point given in body coordinates, all as fractions of length:
/// `s` along the spine (0 snout, 1 tail tip), `a` distance from it, and
/// `past` how far it sits beyond the tail tip. `flare` spreads the fins.
pub fn classify(
    s: f64,
    a: f64,
    past: f64,
    flare: f64,
    out: &mut ShapeHit,
    ahead: f64,
    lateral: f64,
) -> bool {
    out.flag = 0;
    out.across = 1.0;
    out.du = 1.0;
    out.da = 0.0;

    // Barbels: a short whisker angling out from each corner of the mouth, the tell that this is a koi.
    if ahead > 0.0 {
        if ahead < 0.05 && (lateral - (0.045 + ahead * 0.7)).abs() < 0.016 {
            out.flag = FLAG_BARBEL;
            return true;
        }
        return false;
    }

    let w = body_half_width(s);
    if s <= BODY_END && a <= w {
        let across = a / w;
        out.across = across;
        if s > 0.055 && s < 0.1 && across > 0.5 && across < 0.85 {
            out.flag = FLAG_EYE;
        } else if s > 0.28 && s < 0.6 && across < 0.14 {
            out.flag = FLAG_DORSAL;
        } else {
            out.flag = FLAG_BODY;
        }
        return true;
    }

    if s > 0.68 && past <= 0.0 {
        let tw = tail_half_width(s) * flare;
        if tw > 0.0 && a <= tw && s <= 0.86 + 0.14 * (a / tw) {
            out.flag = FLAG_FIN;
            out.da = (a / tw) * 0.8;
            return true;
        }
    }

    in_fin(
        s,
        a,
        0.19,
        0.85 + 0.5 * (flare - 1.0),
        0.066 * flare,
        0.04,
        out,
    ) || in_fin(s, a, 0.47, 0.75, 0.036, 0.018, out)
}
