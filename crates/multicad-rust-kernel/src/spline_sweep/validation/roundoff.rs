//! Outward arithmetic for the authored monotonicity certificate.
#[derive(Clone, Copy)]
struct Bounds {
    lo: f64,
    hi: f64,
}
fn enclosed(value: f64, residual: f64) -> Bounds {
    if value.abs() < 1e-280 && value != 0. {
        return Bounds {
            lo: value.next_down(),
            hi: value.next_up(),
        };
    }
    Bounds {
        lo: if residual < 0. {
            value.next_down()
        } else {
            value
        },
        hi: if residual > 0. {
            value.next_up()
        } else {
            value
        },
    }
}
fn sum(a: f64, b: f64) -> Bounds {
    let value = a + b;
    let part = value - a;
    enclosed(value, (a - (value - part)) + (b - part))
}
fn product(a: f64, b: f64) -> Bounds {
    let value = a * b;
    if a != 0. && b != 0. && value == 0. {
        return Bounds {
            lo: value.next_down(),
            hi: value.next_up(),
        };
    }
    enclosed(value, a.mul_add(b, -value))
}
fn multiply(range: Bounds, value: f64) -> Bounds {
    let [a, b] = [product(range.lo, value), product(range.hi, value)];
    Bounds {
        lo: a.lo.min(b.lo),
        hi: a.hi.max(b.hi),
    }
}
pub(super) fn nonnegative_difference(a: [f64; 2], b: [f64; 2], axis: [f64; 2]) -> bool {
    let [x, y] = std::array::from_fn::<_, 2, _>(|i| multiply(sum(a[i], -b[i]), axis[i]));
    sum(x.lo, y.lo).lo >= 0.
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_and_rounded_projection_terms_remain_outward() {
        let small = product(1e-150, 1e-150);
        assert!(small.lo < 1e-300 && small.hi > 1e-300);
        let underflow = product(1e-200, 1e-200);
        assert!(underflow.lo < 0. && underflow.hi > 0.);
        let positive = sum(1., 1e-17);
        assert_eq!(positive.lo, 1.);
        assert!(positive.hi > 1.);
        let negative = sum(1., -1e-17);
        assert!(negative.lo < 1.);
        assert_eq!(negative.hi, 1.);
        assert!(!nonnegative_difference([1., 0.], [1., 1e-300], [0., 1.]));
    }
}
