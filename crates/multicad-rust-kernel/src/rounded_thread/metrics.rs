//! Smooth phase integrals with closed axial Fourier moments; no axial-symmetry
//! assumption is made for fractional or complete turns.
use super::{Error, Thread};
type Point = [f64; 3];
#[derive(Clone, Debug, PartialEq)]
pub struct Properties {
    pub faces: Vec<(Point, f64)>,
    pub centroid_mm: Point,
    pub central_second_moment_mm5: [[f64; 3]; 3],
}
fn multiply([a, b]: [f64; 2], [c, d]: [f64; 2]) -> [f64; 2] {
    [a * c - b * d, a * d + b * c]
}
fn integral<const N: usize>(a: f64, b: f64, f: impl Fn(f64) -> [f64; N]) -> [f64; N] {
    let root = (10_f64 / 7.).sqrt();
    let nodes = [
        0.,
        ((5. - 2. * root) / 9.).sqrt(),
        ((5. + 2. * root) / 9.).sqrt(),
    ];
    let term = 13. * 70_f64.sqrt();
    let weights = [128. / 225., (322. + term) / 900., (322. - term) / 900.];
    let half = (b - a) / 16.;
    let mut result = [0.; N];
    for i in 0..8 {
        let c = a + (2. * i as f64 + 1.) * half;
        for (node, w) in nodes.into_iter().zip(weights) {
            let x = f(c + half * node);
            let y = if node == 0. {
                [0.; N]
            } else {
                f(c - half * node)
            };
            for j in 0..N {
                result[j] += half * w * (x[j] + y[j]);
            }
        }
    }
    result
}
pub(super) fn compute(t: &Thread) -> Result<Properties, Error> {
    let s = t.spec();
    let q = t.helical_rate();
    let h = s.length_mm / q;
    let half = h / 2.;
    let average = [
        half.cos() * half.sin() / half,
        half.sin() * half.sin() / half,
    ];
    let mut cap = [0.; 4];
    let mut walls = Vec::with_capacity(5);
    let breaks = t.profile_breaks();
    for (band, pair) in breaks.windows(2).enumerate() {
        let circle = t.profile_circle(band);
        let bounds = circle.map_or([pair[0], pair[1]], |c| {
            [pair[0], pair[1]].map(|p| ((p - c.center_phase_mm) / c.radius_mm).asin())
        });
        let values = integral(bounds[0], bounds[1], |parameter| {
            let (phase, r, ds, area) = if let Some(c) = circle {
                let (sin, cos) = parameter.sin_cos();
                let base = if c.radial_sign > 0. {
                    s.major_radius_mm
                } else {
                    t.root_radius_mm()
                };
                let r = base - c.radial_sign * 2. * c.radius_mm * (parameter / 2.).sin().powi(2);
                (
                    c.center_phase_mm + c.radius_mm * sin,
                    r,
                    c.radius_mm * cos,
                    c.radius_mm * r.hypot(q * sin),
                )
            } else {
                let [r, d] = t.radius_and_slope_at_phase(parameter);
                (parameter, r, 1., r.hypot(d * r.hypot(q)))
            };
            let (sin, cos) = (-phase / q).sin_cos();
            let sin2 = 2. * sin * cos;
            let cos2 = cos * cos - sin * sin;
            [
                area,
                area * r * cos,
                area * r * sin,
                r.powi(3) * cos * ds,
                r.powi(3) * sin * ds,
                r.powi(4) * cos2 * ds,
                r.powi(4) * sin2 * ds,
            ]
        });
        let [area, x, y, mx, my, xx, xy] = values;
        let center = multiply([x / area, y / area], average);
        walls.push((
            [center[0], center[1], s.length_mm / 2.],
            area * s.length_mm / q.abs(),
        ));
        for (slot, value) in cap.iter_mut().zip([mx, my, xx, xy]) {
            *slot += value;
        }
    }
    let volume = t.volume_mm3();
    let area = volume / s.length_mm;
    let first = [cap[0], cap[1]].map(|x| x / (3. * q.abs()));
    let second = [cap[2], cap[3]].map(|x| x / (4. * q.abs()));
    let mut faces = [0., s.length_mm]
        .map(|z| {
            let (si, co) = (z / q).sin_cos();
            let p = multiply(first, [co, si]).map(|v| v / area);
            ([p[0], p[1], z], area)
        })
        .to_vec();
    faces.extend(walls);
    let moment = multiply(first, average.map(|x| x * s.length_mm));
    let centroid = [moment[0] / volume, moment[1] / volume, s.length_mm / 2.];
    let w2 = [h.cos() * h.sin() / h, h.sin() * h.sin() / h].map(|x| x * s.length_mm);
    let difference = multiply(second, w2);
    let polar = t.polar_second_moment_mm5();
    let weighted = [
        q * s.length_mm * h.sin() + q * q * (h.cos() - 1.),
        -q * s.length_mm * h.cos() + q * q * h.sin(),
    ];
    let mixed = multiply(first, weighted);
    let raw = [
        [(polar + difference[0]) / 2., difference[1] / 2., mixed[0]],
        [difference[1] / 2., (polar - difference[0]) / 2., mixed[1]],
        [mixed[0], mixed[1], t.axial_second_moment_mm5()],
    ];
    let central = std::array::from_fn(|i| {
        std::array::from_fn(|j| raw[i][j] - volume * centroid[i] * centroid[j])
    });
    Properties::checked(faces, centroid, central)
}
impl Properties {
    fn checked(
        faces: Vec<(Point, f64)>,
        centroid_mm: Point,
        central_second_moment_mm5: [[f64; 3]; 3],
    ) -> Result<Self, Error> {
        if faces.len() != 7
            || faces
                .iter()
                .any(|(p, a)| p.iter().any(|x| !x.is_finite()) || !a.is_finite() || *a <= 0.)
            || centroid_mm
                .iter()
                .chain(central_second_moment_mm5.iter().flatten())
                .any(|x| !x.is_finite())
            || (0..3).any(|i| central_second_moment_mm5[i][i] < 0.)
        {
            return Err(Error::Unresolved);
        }
        Ok(Self {
            faces,
            centroid_mm,
            central_second_moment_mm5,
        })
    }
}
#[cfg(test)]
mod tests;
