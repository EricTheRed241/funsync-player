//! Funscript interpolation (`renderer/js/interpolation.js`): linear, PCHIP
//! (Fritsch-Carlson, monotone) and Makima, plus the speed limit.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Action {
    pub at: f64,
    pub pos: f64,
}

/// `i` with `a[i].at <= t < a[i+1].at`; None before the first action.
fn interval(a: &[Action], t: f64) -> Option<usize> {
    let i = a.partition_point(|x| x.at <= t);
    i.checked_sub(1)
}

/// Position 0–100 at `t` ms with `mode` (`linear`, `pchip`, `makima`,
/// `step`: hold each action); the first / last position outside the script.
pub fn position(mode: &str, a: &[Action], t: f64) -> Option<f64> {
    let (first, last) = (a.first()?, a.last()?);
    let Some(i) = interval(a, t) else {
        return Some(first.pos);
    };
    if i + 1 >= a.len() {
        return Some(last.pos);
    }
    let h = a[i + 1].at - a[i].at;
    if h <= 0.0 {
        return Some(a[i].pos);
    }
    let s = (t - a[i].at) / h;
    let d = match mode {
        "step" => return Some(a[i].pos),
        "pchip" if a.len() > 2 => pchip_slope,
        "makima" if a.len() > 3 => makima_slope,
        _ => return Some(a[i].pos + s * (a[i + 1].pos - a[i].pos)),
    };
    let (m0, m1) = (d(a, i) * h, d(a, i + 1) * h);
    let (s2, s3) = (s * s, s * s * s);
    let v = (2.0 * s3 - 3.0 * s2 + 1.0) * a[i].pos
        + (s3 - 2.0 * s2 + s) * m0
        + (-2.0 * s3 + 3.0 * s2) * a[i + 1].pos
        + (s3 - s2) * m1;
    Some(v.clamp(0.0, 100.0))
}

fn slope(a: &[Action], j: usize) -> f64 {
    let h = a[j + 1].at - a[j].at;
    if h > 0.0 {
        (a[j + 1].pos - a[j].pos) / h
    } else {
        0.0
    }
}

fn pchip_slope(a: &[Action], k: usize) -> f64 {
    let n = a.len();
    if k == 0 {
        return slope(a, 0);
    }
    if k == n - 1 {
        return slope(a, n - 2);
    }
    let (h0, h1) = (a[k].at - a[k - 1].at, a[k + 1].at - a[k].at);
    let (d0, d1) = (slope(a, k - 1), slope(a, k));
    if h0 <= 0.0 || h1 <= 0.0 || d0 * d1 <= 0.0 {
        return 0.0;
    }
    let (w1, w2) = (2.0 * h1 + h0, h1 + 2.0 * h0);
    (w1 + w2) / (w1 / d0 + w2 / d1)
}

fn makima_slope(a: &[Action], k: usize) -> f64 {
    if k <= 1 || k >= a.len() - 2 {
        return pchip_slope(a, k);
    }
    let d: Vec<f64> = (k - 2..k + 2).map(|j| slope(a, j)).collect();
    let w1 = (d[3] - d[2]).abs() + (d[3] + d[2]).abs() / 2.0;
    let w2 = (d[1] - d[0]).abs() + (d[1] + d[0]).abs() / 2.0;
    if w1 + w2 == 0.0 {
        (d[1] + d[2]) / 2.0
    } else {
        (w1 * d[1] + w2 * d[2]) / (w1 + w2)
    }
}

/// Signed speed of the script segment at `t`, in position units per second.
pub fn velocity(a: &[Action], t: f64) -> f64 {
    match interval(a, t) {
        Some(i) if i + 1 < a.len() => slope(a, i) * 1000.0,
        _ => 0.0,
    }
}

/// Pull `target` toward `last` so the move is at most `max_speed`
/// (position units per second, 0 = off) over `dt_ms`.
pub fn speed_limit(target: f64, last: f64, dt_ms: f64, max_speed: f64) -> f64 {
    if max_speed <= 0.0 || dt_ms <= 0.0 {
        return target;
    }
    let max = max_speed * dt_ms / 1000.0;
    last + (target - last).clamp(-max, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acts(v: &[(f64, f64)]) -> Vec<Action> {
        v.iter().map(|&(at, pos)| Action { at, pos }).collect()
    }

    #[test]
    fn linear_midpoint_and_ends() {
        let a = acts(&[(0.0, 0.0), (100.0, 100.0)]);
        assert_eq!(position("linear", &a, 50.0), Some(50.0));
        assert_eq!(position("linear", &a, -5.0), Some(0.0));
        assert_eq!(position("linear", &a, 500.0), Some(100.0));
        assert_eq!(position("linear", &[], 0.0), None);
    }

    #[test]
    fn smooth_modes_stay_monotone_without_overshoot() {
        // A rise, a plateau, a sharp drop: the classic overshoot trap.
        let a = acts(&[
            (0.0, 0.0),
            (100.0, 90.0),
            (200.0, 100.0),
            (300.0, 100.0),
            (310.0, 0.0),
            (600.0, 10.0),
        ]);
        for mode in ["linear", "pchip", "makima"] {
            for w in a.windows(2) {
                let (lo, hi) = (w[0].pos.min(w[1].pos), w[0].pos.max(w[1].pos));
                let mut prev = w[0].pos;
                for k in 0..=20 {
                    let t = w[0].at + (w[1].at - w[0].at) * k as f64 / 20.0;
                    let p = position(mode, &a, t).unwrap();
                    assert!(p >= lo - 1e-9 && p <= hi + 1e-9, "{mode} {t}: {p}");
                    let rising = w[1].pos >= w[0].pos;
                    assert!(if rising {
                        p >= prev - 1e-9
                    } else {
                        p <= prev + 1e-9
                    });
                    prev = p;
                }
            }
        }
        // Smoothing actually differs from linear.
        assert_ne!(position("pchip", &a, 50.0), position("linear", &a, 50.0));
    }

    #[test]
    fn speed_limit_caps_moves() {
        assert_eq!(speed_limit(100.0, 0.0, 100.0, 400.0), 40.0);
        assert_eq!(speed_limit(0.0, 100.0, 100.0, 400.0), 60.0);
        assert_eq!(speed_limit(30.0, 0.0, 100.0, 400.0), 30.0);
        assert_eq!(speed_limit(100.0, 0.0, 100.0, 0.0), 100.0);
        let a = acts(&[(0.0, 100.0), (500.0, 0.0)]);
        assert_eq!(velocity(&a, 10.0), -200.0);
        assert_eq!(position("step", &a, 400.0), Some(100.0));
    }
}
