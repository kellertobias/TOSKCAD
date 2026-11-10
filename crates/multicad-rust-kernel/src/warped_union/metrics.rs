//! Integrate physical volume and areas without discretizing the shell.
use super::*;

#[derive(Default)]
pub(super) struct Sum {
    value: f64,
    correction: f64,
}

impl Sum {
    pub(super) fn add(&mut self, term: f64) {
        let total = self.value + term;
        self.correction += if self.value.abs() >= term.abs() {
            (self.value - total) + term
        } else {
            (term - total) + self.value
        };
        self.value = total;
    }

    pub(super) fn value(self) -> f64 {
        self.value + self.correction
    }
}

type XY = [f64; 2];

#[derive(Clone, Copy)]
struct Rail {
    start: XY,
    slope: XY,
}

fn rails(ring: &[Point], lo: f64, hi: f64) -> Result<[Rail; 2], Error> {
    let mut rails = [None, None];
    for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let dw = b[2] - a[2];
        if dw == 0. || a[2].min(b[2]) > lo || a[2].max(b[2]) < hi {
            continue;
        }
        let slope: XY = std::array::from_fn(|i| (b[i] - a[i]) / dw);
        let near = if (lo - a[2]).abs() <= (lo - b[2]).abs() {
            a
        } else {
            b
        };
        let start = std::array::from_fn(|i| slope[i].mul_add(lo - near[2], near[i]));
        let direction = usize::from(dw > 0.);
        if rails[direction].replace(Rail { start, slope }).is_some() {
            return Err(Error::Unsupported);
        }
    }
    let [Some(a), Some(b)] = rails else {
        return Err(Error::Unsupported);
    };
    Ok([a, b])
}

struct Moments {
    /// Integrals of (w − lo)ⁿ dz, n = 0, 1, 2.
    z: [f64; 3],
    /// Integrals of (w − lo)ⁿ r dθ, n = 0, 1.
    cylinder: [f64; 2],
}

fn moments(band: Band, region: Region, lo: f64, hi: f64) -> Moments {
    let dw = hi - lo;
    if region != Region::Round {
        return Moments {
            z: [dw, dw * dw * 0.5, dw * dw * dw / 3.],
            cylinder: [0.; 2],
        };
    }
    let r = band.radius_mm;
    let z0 = band.radial_drop(lo);
    let z1 = band.radial_drop(hi);
    let dz = dw * (hi + lo) / (z0 + z1);
    let [t0, t1] = [lo, hi].map(|w| band.theta(w));
    let h = 0.5 * (t1 - t0);
    let m = t0.midpoint(t1);
    let (sh, ch) = h.sin_cos();
    let h_minus_sh_ch = if h < 1e-3 {
        let h2 = h * h;
        h * h2 * (2. / 3. + h2 * (-2. / 15. + h2 * (4. / 315.)))
    } else {
        h - sh * ch
    };
    // ∫w dz = r²∫cos²θ dθ. Written to avoid subtracting nearly
    // equal terms at the lower limit of the quarter circle.
    let first = r * r * (h_minus_sh_ch + 2. * m.cos().powi(2) * sh * ch);
    // ∫w² dz follows from the antiderivative of cos³θ.
    let second = dz * (0.5 * (lo * lo + hi * hi) + dz * dz / 6.);
    let arc = 2. * r * h;
    Moments {
        z: [dz, first - lo * dz, second - 2. * lo * first + lo * lo * dz],
        cylinder: [arc, r * dz - lo * arc],
    }
}

pub(super) fn polygon_area(ring: &[Point], normal: Point) -> f64 {
    let origin = ring[0];
    let mut area = Sum::default();
    for edge in ring[1..].windows(2) {
        area.add(dot(
            cross(sub(edge[0], origin), sub(edge[1], origin)),
            normal,
        ));
    }
    area.value() * 0.5
}

#[derive(Debug, PartialEq)]
pub(super) struct FaceMetrics {
    pub area_mm2: f64,
    pub volume_mm3: f64,
}

pub(super) fn face_metrics(
    band: Band,
    region: Region,
    support: Support,
    normal: Point,
    ring: &[Point],
    anchor: XY,
) -> Result<FaceMetrics, Error> {
    let mut levels: Vec<_> = ring.iter().map(|point| point[2]).collect();
    levels.sort_by(f64::total_cmp);
    levels.dedup();
    let measure_curve = region == Region::Round
        && matches!(
            support,
            Support::Cylinder { .. } | Support::Plane { normal: [_, _, 0.] }
        );
    let mut area = Sum::default();
    let mut volume = Sum::default();
    for pair in levels.windows(2) {
        let lo = pair[0];
        let hi = pair[1];
        let [a, b] = rails(ring, lo, hi)?;
        let m = moments(band, region, lo, hi);
        let a0 = std::array::from_fn(|i| a.start[i] - anchor[i]);
        let b0 = std::array::from_fn(|i| b.start[i] - anchor[i]);
        let det = |[x, y]: XY, [u, v]: XY| x * v - y * u;
        let coefficients = [
            det(a0, b0),
            det(a0, b.slope) + det(a.slope, b0),
            det(a.slope, b.slope),
        ];
        volume.add(0.5 * dot(coefficients, m.z));
        if measure_curve {
            let scale = normal[0].hypot(normal[1]);
            let tangent = [-normal[1] / scale, normal[0] / scale];
            let width = |a: XY, b: XY| (b[0] - a[0]) * tangent[0] + (b[1] - a[1]) * tangent[1];
            let [m0, m1] = if matches!(support, Support::Cylinder { .. }) {
                m.cylinder
            } else {
                [m.z[0], m.z[1]]
            };
            area.add(width(a.start, b.start) * m0 + width(a.slope, b.slope) * m1);
        }
    }
    let area = if measure_curve {
        area.value()
    } else {
        polygon_area(ring, normal)
    };
    if area.is_finite() && area > 0. {
        Ok(FaceMetrics {
            area_mm2: area,
            volume_mm3: volume.value(),
        })
    } else {
        Err(Error::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_rails_require_one_entry_and_one_exit() {
        let split_boundary = [[0., 0., 0.], [1., 0., 2.], [2., 0., 0.], [3., 0., 2.]];
        assert!(matches!(
            rails(&split_boundary, 0.5, 1.5),
            Err(Error::Unsupported)
        ));
        assert!(matches!(
            rails(&split_boundary, 3., 4.),
            Err(Error::Unsupported)
        ));
    }

    #[test]
    fn a_zero_area_face_cannot_supply_metrics() {
        let line = [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]];
        let normal = [0., 0., 1.];
        assert_eq!(
            face_metrics(
                Band {
                    center_z_mm: 1.,
                    radius_mm: 1.
                },
                Region::Below,
                Support::Plane { normal },
                normal,
                &line,
                [0.; 2],
            ),
            Err(Error::Unsupported)
        );
    }
}
