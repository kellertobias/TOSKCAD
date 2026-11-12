//! Source derivatives for bounded manufacturing tensors; no mesh is used.
mod hermite;
mod retained;
use super::{
    law::Scalar,
    series::{Dual, Series},
    *,
};
pub use hermite::{MANUFACTURING_DEGREE, ManufacturingTensor, ManufacturingTensors};
pub(crate) use retained::{RadialChart, certify_radial_charts};
pub use retained::{RetainedTensor, TensorCell, TensorDerivatives};
type S = Series<13>;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaylorEnclosure {
    /// Entry n bounds |∂u^n P|/n! in normalized profile coordinates.
    pub profile: [[f64; 3]; 13],
    /// Entry n bounds |∂z^n P|/n! in physical millimetres.
    pub axial: [[f64; 3]; 13],
}
impl Thread {
    pub fn manufacturing_taylor_bounds(
        &self,
        band: usize,
        profile: [f64; 2],
        axial: [f64; 2],
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<TaylorEnclosure, Error> {
        if band >= 3 {
            return Err(Error::Unsupported);
        }
        let [u0, u1] = profile;
        let [z0, z1] = axial;
        if ![u0, u1, z0, z1].into_iter().all(f64::is_finite)
            || u0 < 0.
            || u0 > u1
            || u1 > 1.
            || z0 < 0.
            || z0 > z1
            || z1 > self.spec().length_mm
        {
            return Err(Error::Invalid);
        }
        let mut work = Work {
            remaining: 1000,
            interrupt,
        };
        work.step()?;
        let z = I::new(z0, z1);
        let u = I::new(u0, u1);
        let roots = self.axial_root_series(z, &mut work)?;
        let along_u = self.series_surface(
            band,
            S::variable(u),
            S::interval(z),
            roots.map(|root| S::interval(root.c[0])),
        );
        let along_z = self.series_surface(band, S::interval(u), S::variable(z), roots);
        for p in along_u.into_iter().chain(along_z) {
            p.checked()?;
        }
        Ok(TaylorEnclosure {
            profile: std::array::from_fn(|n| along_u.map(|p| p.c[n].magnitude())),
            axial: std::array::from_fn(|n| along_z.map(|p| p.c[n].magnitude())),
        })
    }
    fn axial_root_series(&self, z: I, work: &mut Work<'_>) -> Result<[S; 2], Error> {
        let mut result = [S::constant(0.); 2];
        for (index, piece) in [0, 2].into_iter().enumerate() {
            work.step()?;
            let root = self.root_station(piece, z)?.v;
            let mut d = S::interval(root);
            let zz = S::variable(z);
            // Formal Newton doubles the number of correct Taylor terms when
            // its constant term is the true implicit root. Its unknown value
            // is enclosed by root; four steps determine all thirteen terms.
            for _ in 0..4 {
                work.step()?;
                let f = law::evaluate(
                    self,
                    piece,
                    Dual {
                        v: d,
                        d: S::constant(1.),
                    },
                    Dual {
                        v: zz,
                        d: S::constant(0.),
                    },
                )
                .excess;
                // The reciprocal refuses zero-containing or nonfinite
                // derivatives. Check every correction coefficient before
                // restoring the independently enclosed constant root.
                d = d - (f.v / f.d).checked()?;
                d.c[0] = root;
            }
            result[index] = d.checked()?;
        }
        Ok(result)
    }
    fn series_surface(&self, band: usize, u: S, z: S, roots: [S; 2]) -> [S; 3] {
        let half = self.spec().crest_width_mm / 2.;
        let d = match band {
            0 => roots[0] + (S::constant(-half) - roots[0]) * u,
            1 => S::constant(-half) + S::constant(2. * half) * u,
            _ => S::constant(half) + (roots[1] - S::constant(half)) * u,
        };
        let p = law::evaluate(self, band, d, z);
        let [sin, cos] = ((z - d) / S::constant(self.source.helical_rate())).sin_cos();
        [p.re * cos - p.im * sin, p.re * sin + p.im * cos, z]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formal_root_derivatives_match_independent_implicit_first_and_second_partials() {
        for (k, left) in [(0.1, false), (-0.1, true), (0., false)] {
            let t = Thread::new(super::super::tests::spec(3, left), k, 1e-7, 1e-6).unwrap();
            let mut work = Work {
                remaining: 1000,
                interrupt: None,
            };
            let z = I::point(3.);
            let roots = t.axial_root_series(z, &mut work).unwrap();
            for (index, piece) in [0, 2].into_iter().enumerate() {
                let reference = t.root_station(piece, z).unwrap();
                let root = roots[index];
                assert!(root.c[1].lo <= reference.z.hi && root.c[1].hi >= reference.z.lo);
                let second = root.c[2] * I::point(2.);
                assert!(second.lo <= reference.zz.hi && second.hi >= reference.zz.lo);
                let residual = law::evaluate(&t, piece, root, S::variable(z)).excess;
                for coefficient in residual.c {
                    assert!(
                        coefficient.lo <= 0. && coefficient.hi >= 0.,
                        "piece{piece} {coefficient:?}"
                    );
                }
            }
            for band in 0..3 {
                let bound = t
                    .manufacturing_taylor_bounds(band, [0.2, 0.3], [3., 3.01], None)
                    .unwrap();
                assert!(
                    bound
                        .profile
                        .into_iter()
                        .chain(bound.axial)
                        .flatten()
                        .all(|x| x.is_finite() && x >= 0.)
                );
                let p = t
                    .series_surface(band, S::constant(0.3), S::constant(3.), roots)
                    .map(|p| p.c[0]);
                let reference = t.surface_point_bounds(band, 0.3, 3.).unwrap();
                for (actual, reference) in p.into_iter().zip(reference) {
                    assert!(actual.lo <= reference[1] && actual.hi >= reference[0]);
                }
            }
        }
    }
    #[test]
    fn derivative_proofs_refuse_invalid_domains_and_cancel_without_poisoning_retry() {
        let t = Thread::new(super::super::tests::spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
        assert_eq!(
            t.manufacturing_taylor_bounds(3, [0., 1.], [0., 1.], None),
            Err(Error::Unsupported)
        );
        assert_eq!(
            t.manufacturing_taylor_bounds(0, [-0.1, 1.], [0., 1.], None),
            Err(Error::Invalid)
        );
        assert_eq!(
            t.manufacturing_taylor_bounds(0, [0., 1.], [0., 1.], Some(&|| true)),
            Err(Error::Cancelled)
        );
        assert!(
            t.manufacturing_taylor_bounds(0, [0., 1.], [0., 1.], None)
                .is_ok()
        );
    }
}
