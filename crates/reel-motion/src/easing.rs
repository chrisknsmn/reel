//! The easing curve zooms use.

#[must_use]
pub fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn sample(a1: f64, a2: f64, t: f64) -> f64 {
    let inverse = 1.0 - t;
    3.0 * a1 * inverse * inverse * t + 3.0 * a2 * inverse * t * t + t * t * t
}

fn derivative(a1: f64, a2: f64, t: f64) -> f64 {
    let inverse = 1.0 - t;
    3.0 * a1 * inverse * inverse + 6.0 * (a2 - a1) * inverse * t + 3.0 * (1.0 - a2) * t * t
}

/// A CSS-style cubic Bézier, solved with Newton steps and then bisection.
#[must_use]
pub fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    let target = clamp01(t);
    let mut solved = target;
    for _ in 0..8 {
        let error = sample(x1, x2, solved) - target;
        let slope = derivative(x1, x2, solved);
        if error.abs() < 1e-6 || slope.abs() < 1e-6 {
            break;
        }
        solved -= error / slope;
    }
    let (mut lower, mut upper) = (0.0, 1.0);
    solved = clamp01(solved);
    for _ in 0..10 {
        let current = sample(x1, x2, solved);
        if (current - target).abs() < 1e-6 {
            break;
        }
        if current < target {
            lower = solved;
        } else {
            upper = solved;
        }
        solved = f64::midpoint(lower, upper);
    }
    sample(y1, y2, solved)
}

/// The curve zoom strength follows on the way in.
#[must_use]
pub fn ease_out_zoom(t: f64) -> f64 {
    cubic_bezier(0.16, 1.0, 0.3, 1.0, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_starts_and_ends_where_it_should() {
        assert!(ease_out_zoom(0.0).abs() < 1e-6);
        assert!((ease_out_zoom(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn it_is_steep_early_and_gentle_late() {
        assert!(ease_out_zoom(0.25) > 0.7);
        assert!(ease_out_zoom(0.75) > 0.97);
    }

    #[test]
    fn it_never_goes_backwards() {
        let mut previous = 0.0;
        for step in 0..=1000 {
            let value = ease_out_zoom(f64::from(step) / 1000.0);
            assert!(value >= previous - 1e-5, "went backwards at step {step}");
            previous = value;
        }
    }

    #[test]
    fn a_straight_line_bezier_is_linear() {
        for step in 0..=20 {
            let t = f64::from(step) / 20.0;
            assert!((cubic_bezier(0.25, 0.25, 0.75, 0.75, t) - t).abs() < 1e-3);
        }
    }

    #[test]
    fn values_outside_the_range_are_clamped() {
        assert!(ease_out_zoom(-1.0).abs() < 1e-6);
        assert!((ease_out_zoom(2.0) - 1.0).abs() < 1e-6);
    }
}
