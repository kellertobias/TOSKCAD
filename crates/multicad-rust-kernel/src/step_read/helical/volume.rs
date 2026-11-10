//! Polynomial flux integration of the decoded helical STEP boundary.
use super::*;
use std::array::from_fn;

const WIDTH: usize = DEGREE + 1;

// Gauss–Legendre on [0, 1]. The flux degree is at most 3*DEGREE-1
// in each parameter, so 17 nodes integrate each span without tessellation.
const NODES: [(f64, f64); 17] = [
    (0.004712262342791318, 0.0120741514342737),
    (0.02466223911561616, 0.02772976468699385),
    (0.05988042313650704, 0.04251807415858946),
    (0.1092429980515993, 0.05594192359670212),
    (0.1711644203916546, 0.06756818423426277),
    (0.2436547314567615, 0.07702288053840511),
    (0.3243841182730618, 0.084002051078225),
    (0.4107579092520761, 0.08828135268349634),
    (0.5, 0.089_723_235_178_103_3),
    (0.5892420907479239, 0.08828135268349634),
    (0.6756158817269382, 0.084002051078225),
    (0.7563452685432385, 0.07702288053840511),
    (0.8288355796083453, 0.06756818423426277),
    (0.8907570019484007, 0.05594192359670212),
    (0.9401195768634929, 0.04251807415858946),
    (0.9753377608843838, 0.02772976468699385),
    (0.9952877376572087, 0.0120741514342737),
];

struct Sample {
    weight: f64,
    basis: [f64; WIDTH],
    derivative: [f64; WIDTH],
}

pub(super) struct Quadrature([Sample; NODES.len()]);

impl Quadrature {
    pub(super) fn new() -> Self {
        Self(NODES.map(|(parameter, weight)| {
            let lower = basis::<DEGREE>(parameter);
            let derivative = from_fn(|i| {
                let preceding = i.checked_sub(1).map_or(0., |i| lower[i]);
                DEGREE as f64 * (preceding - lower.get(i).copied().unwrap_or(0.))
            });
            Sample {
                weight,
                basis: basis(parameter),
                derivative,
            }
        }))
    }

    pub(super) fn spline_flux(&self, rows: &[Vec<Point>], origin: Point, axis: Point) -> f64 {
        let mut flux = 0.;
        for radial in rows.windows(WIDTH).step_by(DEGREE) {
            for axial in (0..radial[0].len() - DEGREE).step_by(DEGREE) {
                let patch = from_fn(|i| from_fn(|j| sub(radial[i][axial + j], origin)));
                flux += self.patch_flux(&patch, axis);
            }
        }
        flux
    }

    fn patch_flux(&self, controls: &[[Point; WIDTH]; WIDTH], axis: Point) -> f64 {
        let mut flux = 0.;
        for radial in &self.0 {
            // Restrict the tensor patch in u once per radial node. The
            // remaining curves give position and the u derivative along v.
            let mut columns = [[[0.; 3]; 2]; WIDTH];
            for ((row, basis), derivative) in
                controls.iter().zip(radial.basis).zip(radial.derivative)
            {
                for ([position, tangent], &point) in columns.iter_mut().zip(row) {
                    accumulate(position, point, basis);
                    accumulate(tangent, point, derivative);
                }
            }
            for axial in &self.0 {
                let [mut position, mut du, mut dv] = [[0.; 3]; 3];
                for (([p, tangent], basis), derivative) in
                    columns.into_iter().zip(axial.basis).zip(axial.derivative)
                {
                    accumulate(&mut position, p, basis);
                    accumulate(&mut du, tangent, basis);
                    accumulate(&mut dv, p, derivative);
                }
                // F = 1/2 * the radial projection of position has divergence
                // one and contributes zero on both transverse cap planes.
                let along_axis = dot(position, axis);
                let radial_point = from_fn(|i| position[i] - along_axis * axis[i]);
                flux += radial.weight * axial.weight * dot(radial_point, cross(du, dv)) / 2.;
            }
        }
        flux
    }
}

fn accumulate(sum: &mut Point, point: Point, weight: f64) {
    for (sum, value) in sum.iter_mut().zip(point) {
        *sum += value * weight;
    }
}

fn basis<const COUNT: usize>(t: f64) -> [f64; COUNT] {
    let degree = COUNT - 1;
    let mut binomial = 1.;
    from_fn(|i| {
        let value = binomial * t.powi(i as i32) * (1. - t).powi((degree - i) as i32);
        binomial *= (degree - i) as f64 / (i + 1) as f64;
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrature_integrates_the_flux_degree() {
        for degree in 0..=33 {
            let actual = NODES
                .into_iter()
                .map(|(node, weight)| node.powi(degree) * weight)
                .sum::<f64>();
            assert!((actual - 1. / f64::from(degree + 1)).abs() < 1e-14);
        }
        // A planar wall at x=2 with dy/du=3 and dz/dv=4 has flux 12.
        let span = |i: usize, j: usize| {
            [
                2.,
                3. * i as f64 / DEGREE as f64,
                4. * j as f64 / DEGREE as f64,
            ]
        };
        let controls = (0..WIDTH)
            .map(|i| (0..WIDTH).map(|j| span(i, j)).collect())
            .collect::<Vec<_>>();
        let actual = Quadrature::new().spline_flux(&controls, [0.; 3], [0., 0., 1.]);
        assert!((actual - 12.).abs() < 1e-12);
    }
}
