//! Open/close animation math (pure): the panel unfolds from the island's pill.
use windows::Win32::Foundation::RECT;

/// Time of a full open or close.
pub const DURATION_MS: f32 = 200.0;
/// Interval of the animation timer.
pub const FRAME_MS: u32 = 16;

/// Cubic ease-out: fast start, soft landing. 0 at 0, 1 at 1.
pub fn ease_out(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Moves linear progress `t` (0 = pill, 1 = full) toward the end by `dt_ms`.
/// Reversing mid-way just flips `opening`: the shape stays continuous.
pub fn advance(t: f32, opening: bool, dt_ms: f32) -> f32 {
    let step = dt_ms / DURATION_MS;
    (if opening { t + step } else { t - step }).clamp(0.0, 1.0)
}

fn mix(a: i32, b: i32, e: f32) -> i32 {
    (a as f32 + (b - a) as f32 * e).round() as i32
}

/// The rectangle between `from` (e = 0) and `to` (e = 1).
pub fn lerp_rect(from: RECT, to: RECT, e: f32) -> RECT {
    RECT {
        left: mix(from.left, to.left, e),
        top: mix(from.top, to.top, e),
        right: mix(from.right, to.right, e),
        bottom: mix(from.bottom, to.bottom, e),
    }
}

/// Opacity (0..=1) of the black veil over the content: opaque while the shape
/// is still a pill, gone once it is mostly open.
pub fn veil(e: f32) -> f32 {
    (1.0 - (e - 0.25) / 0.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_endpoints_and_shape() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!((ease_out(0.5) - 0.875).abs() < 1e-6);
        assert_eq!(ease_out(-1.0), 0.0);
        assert_eq!(ease_out(2.0), 1.0);
        assert!(ease_out(0.25) > 0.25); // ease-out is ahead of linear
    }

    #[test]
    fn rect_at_zero_half_one() {
        let pill = RECT {
            left: 900,
            top: 0,
            right: 1020,
            bottom: 28,
        };
        let full = RECT {
            left: 580,
            top: 0,
            right: 1340,
            bottom: 482,
        };
        assert_eq!(lerp_rect(pill, full, ease_out(0.0)), pill);
        assert_eq!(lerp_rect(pill, full, ease_out(1.0)), full);
        let mid = lerp_rect(pill, full, ease_out(0.5));
        assert_eq!(
            (mid.left, mid.top, mid.right, mid.bottom),
            (620, 0, 1300, 425)
        );
    }

    #[test]
    fn advance_runs_both_ways_and_clamps() {
        assert_eq!(advance(0.0, true, 100.0), 0.5);
        assert_eq!(advance(0.5, true, 1000.0), 1.0);
        assert_eq!(advance(0.5, false, 100.0), 0.0);
        assert_eq!(advance(0.75, false, 50.0), 0.5); // reversed mid-way
    }

    #[test]
    fn veil_hides_the_content_while_small() {
        assert_eq!(veil(0.0), 1.0);
        assert_eq!(veil(0.25), 1.0);
        assert_eq!(veil(0.75), 0.0);
        assert_eq!(veil(1.0), 0.0);
    }
}
