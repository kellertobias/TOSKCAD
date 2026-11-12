//! Stable two-end Hermite tensors with an independent mixed-partial remainder.
use super::*;
use crate::tapered_thread::bivariate::Bi;
pub const MANUFACTURING_DEGREE: usize = 9;
const D: usize = MANUFACTURING_DEGREE;
const JETS: usize = 5;
const ORDER: usize = 10;
pub(super) const MAX_POLES: usize = 524288;
#[derive(Clone, Debug, PartialEq)]
pub struct ManufacturingTensor {
    band: usize,
    profile_spans: usize,
    axial_spans: usize,
    poles: Vec<[f64; 3]>,
    error_mm: f64,
}
impl ManufacturingTensor {
    pub fn band(&self) -> usize {
        self.band
    }
    pub fn spans(&self) -> [usize; 2] {
        [self.profile_spans, self.axial_spans]
    }
    pub fn dimensions(&self) -> [usize; 2] {
        [D * self.profile_spans + 1, D * self.axial_spans + 1]
    }
    pub fn poles(&self) -> &[[f64; 3]] {
        &self.poles
    }
    pub fn error_bound_mm(&self) -> f64 {
        self.error_mm
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ManufacturingTensors {
    walls: [ManufacturingTensor; 3],
    error_mm: f64,
    feature_margin_mm: f64,
}
impl ManufacturingTensors {
    pub fn feature_margin_mm(&self) -> f64 {
        self.feature_margin_mm
    }
    pub fn walls(&self) -> &[ManufacturingTensor; 3] {
        &self.walls
    }
    pub fn error_bound_mm(&self) -> f64 {
        self.error_mm
    }
}
#[derive(Clone, Copy, Default)]
struct Bounds {
    profile: f64,
    mixed: [f64; JETS],
}
fn binomial(n: usize, k: usize) -> f64 {
    let mut result = 1.;
    for j in 0..k {
        result = result * (n - j) as f64 / (j + 1) as f64;
    }
    result
}
fn weight(i: usize, k: usize) -> I {
    I::point(binomial(i, k)) / I::point(binomial(D, k))
}
fn power(x: I, n: usize) -> I {
    let mut result = I::point(1.);
    for _ in 0..n {
        result = result * x;
    }
    result
}
fn parameter(index: usize, count: usize, height: f64) -> I {
    if index == 0 {
        I::point(0.)
    } else if index == count {
        I::point(height)
    } else {
        I::point(height) * I::point(index as f64) / I::point(count as f64)
    }
}
fn roots_bi<const U: usize, const Z: usize>(roots: [S; 2]) -> [Bi<U, Z>; 2] {
    roots.map(|root| {
        let mut value = Bi::constant(0.);
        for j in 0..Z {
            value.c[0][j] = root.c[j];
        }
        value
    })
}
fn put_control(
    poles: &mut [[f64; 3]],
    filled: &mut [bool],
    index: usize,
    chosen: [f64; 3],
) -> Result<(), Error> {
    if filled[index] && poles[index] != chosen {
        return Err(Error::Unresolved);
    }
    poles[index] = chosen;
    filled[index] = true;
    Ok(())
}
fn shared_creases(walls: &[ManufacturingTensor; 3]) -> Result<(), Error> {
    let height = walls[0].dimensions()[1];
    for band in 0..2 {
        for j in 0..height {
            let a = walls[band].poles[(walls[band].dimensions()[0] - 1) * height + j];
            let b = walls[band + 1].poles[j];
            if a != b {
                return Err(Error::Unresolved);
            }
        }
    }
    Ok(())
}
impl Thread {
    fn bi_surface<const U: usize, const Z: usize>(
        &self,
        band: usize,
        u: Bi<U, Z>,
        z: Bi<U, Z>,
        roots: [Bi<U, Z>; 2],
    ) -> [Bi<U, Z>; 3] {
        let half = self.spec().crest_width_mm / 2.;
        let d = match band {
            0 => roots[0] + (Bi::constant(-half) - roots[0]) * u,
            1 => Bi::constant(-half) + Bi::constant(2. * half) * u,
            _ => Bi::constant(half) + (roots[1] - Bi::constant(half)) * u,
        };
        let p = law::evaluate(self, band, d, z);
        let [sin, cos] = ((z - d) / Bi::constant(self.source.helical_rate())).sin_cos();
        [p.re * cos - p.im * sin, p.re * sin + p.im * cos, z]
    }
    fn hermite_bounds(&self, work: &mut Work<'_>) -> Result<[Bounds; 3], Error> {
        let mut out = [Bounds::default(); 3];
        // Interval cells cover complete domains. They are not sampled proofs.
        for iz in 0..4 {
            let z = I::new(
                parameter(iz, 4, self.spec().length_mm).lo,
                parameter(iz + 1, 4, self.spec().length_mm).hi,
            );
            let roots = self.axial_root_series(z, work)?;
            for (band, bounds) in out.iter_mut().enumerate() {
                for iu in 0..2 {
                    work.step()?;
                    let u = I::new(iu as f64 / 2., (iu + 1) as f64 / 2.);
                    let along = self.series_surface(
                        band,
                        S::variable(u),
                        S::interval(z),
                        roots.map(|r| S::interval(r.c[0])),
                    );
                    let mixed = self.bi_surface(
                        band,
                        Bi::<5, 11>::profile(u),
                        Bi::axial(z),
                        roots_bi(roots),
                    );
                    for p in along {
                        p.checked()?;
                    }
                    for p in mixed {
                        p.checked()?;
                    }
                    let m = along
                        .into_iter()
                        .fold(I::point(0.), |s, p| s + I::point(p.c[ORDER].magnitude()));
                    bounds.profile = bounds.profile.max(m.hi);
                    for k in 0..JETS {
                        let m = mixed
                            .into_iter()
                            .fold(I::point(0.), |s, p| s + I::point(p.c[k][ORDER].magnitude()));
                        bounds.mixed[k] = bounds.mixed[k].max(m.hi);
                    }
                }
            }
        }
        Ok(out)
    }
    /// Approximate the actual transported walls; every result carries a
    /// source-space positional bound. Root walls remain analytic cones.
    pub fn manufacturing_tensors(
        &self,
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ManufacturingTensors, Error> {
        if !tolerance.is_finite() || tolerance <= 0. {
            return Err(Error::Invalid);
        }
        let tolerance = tolerance.min(1e-7);
        let mut work = Work {
            remaining: 1_000_000,
            interrupt,
        };
        work.step()?;
        let feature_margin_mm = self.root_sector_margin(&mut work)?;
        let tolerance = tolerance.min(feature_margin_mm / 64.);
        let budget = tolerance / 64.;
        let bounds = self.hermite_bounds(&mut work)?;
        let nu = bounds.map(|b| {
            (b.profile / (1024. * budget * 0.5))
                .powf(0.1)
                .ceil()
                .max(1.) as usize
        });
        let mixed = |b: Bounds, n: usize| {
            let step = I::point(1.) / I::point(n as f64);
            (0..JETS).fold(I::point(0.), |sum, k| {
                sum + weight(JETS - 1, k) * power(step, k) * I::point(b.mixed[k])
            })
        };
        let mz = bounds
            .into_iter()
            .zip(nu)
            .map(|(b, n)| mixed(b, n).hi)
            .fold(0., f64::max);
        let nz = (self.spec().length_mm * (mz / (1024. * budget * 0.5)).powf(0.1))
            .ceil()
            .max(1.) as usize;
        let (nu, nz, errors) = self.certify_partition(bounds, nu, nz, budget, &mut work)?;
        let mut walls = Vec::with_capacity(3);
        for band in 0..3 {
            walls.push(self.build_hermite_wall(
                band,
                nu[band],
                nz,
                errors[band],
                tolerance,
                &mut work,
            )?);
        }
        let walls: [ManufacturingTensor; 3] = walls.try_into().map_err(|_| Error::Unresolved)?;
        let error_mm = walls.iter().map(|w| w.error_mm).fold(0., f64::max);
        shared_creases(&walls)?;
        Ok(ManufacturingTensors {
            walls,
            error_mm,
            feature_margin_mm,
        })
    }
    fn certify_partition(
        &self,
        bounds: [Bounds; 3],
        mut nu: [usize; 3],
        mut nz: usize,
        budget: f64,
        work: &mut Work<'_>,
    ) -> Result<([usize; 3], usize, [f64; 3]), Error> {
        let mixed = |b: Bounds, n: usize| {
            let step = I::point(1.) / I::point(n as f64);
            (0..JETS).fold(I::point(0.), |sum, k| {
                sum + weight(JETS - 1, k) * power(step, k) * I::point(b.mixed[k])
            })
        };
        let errors = loop {
            work.step()?;
            let height = D
                .checked_mul(nz)
                .and_then(|n| n.checked_add(1))
                .ok_or(Error::Limit)?;
            let columns = nu
                .into_iter()
                .try_fold(0usize, |sum, n| {
                    D.checked_mul(n)
                        .and_then(|n| n.checked_add(1))
                        .and_then(|n| sum.checked_add(n))
                })
                .ok_or(Error::Limit)?;
            if columns
                .checked_mul(height)
                .and_then(|n| n.checked_mul(self.spec().starts as usize))
                .is_none_or(|n| n > MAX_POLES)
            {
                return Err(Error::Limit);
            }
            let dz = I::point(self.spec().length_mm) / I::point(nz as f64);
            let parts = std::array::from_fn::<_, 3, _>(|band| {
                let du = I::point(1.) / I::point(nu[band] as f64);
                [
                    I::point(bounds[band].profile) * power(du, ORDER) / I::point(1024.),
                    mixed(bounds[band], nu[band]) * power(dz, ORDER) / I::point(1024.),
                ]
            });
            if parts.into_iter().all(|p| (p[0] + p[1]).hi <= budget) {
                break parts.map(|p| (p[0] + p[1]).hi);
            }
            let mut refined = false;
            for band in 0..3 {
                if parts[band][0].hi > budget * 0.5 {
                    nu[band] = nu[band].checked_add(1).ok_or(Error::Limit)?;
                    refined = true;
                }
            }
            if !refined {
                nz = nz.checked_add(1).ok_or(Error::Limit)?;
            }
        };
        Ok((nu, nz, errors))
    }
    fn root_sector_margin(&self, work: &mut Work<'_>) -> Result<f64, Error> {
        let mut angle = f64::INFINITY;
        for iz in 0..16 {
            work.step()?;
            let z = I::new(
                parameter(iz, 16, self.spec().length_mm).lo,
                parameter(iz + 1, 16, self.spec().length_mm).hi,
            );
            let roots = self.roots(z)?;
            let lower = self.transport(0, roots[0].v, z).phase.v;
            let upper = self.transport(2, roots[1].v, z).phase.v;
            let gap = (I::point(self.spec().pitch_mm) - (upper - lower))
                / I::point(self.source.helical_rate().abs())
                - I::point(64. * f64::EPSILON);
            angle = angle.min(gap.positive()?.lo);
        }
        let minimum =
            self.root_radius_at(0.)?[0].min(self.root_radius_at(self.spec().length_mm)?[0]);
        (I::point(angle) * I::point(minimum))
            .positive()
            .map(|p| p.lo)
    }
    fn certify_hermite_regularity(
        &self,
        tensor: &ManufacturingTensor,
        work: &mut Work<'_>,
    ) -> Result<(), Error> {
        let points = tensor
            .poles
            .iter()
            .map(|p| p.map(I::point))
            .collect::<Vec<_>>();
        self.certify_control_chart(tensor, &points, work)
    }
    pub(crate) fn certify_manufacturing_charts(
        &self,
        tensors: &ManufacturingTensors,
        charts: &[Vec<[I; 3]>],
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(), Error> {
        if charts.len() != 3 * self.spec().starts as usize {
            return Err(Error::Invalid);
        }
        let count = charts
            .iter()
            .try_fold(0usize, |s, c| s.checked_add(c.len()))
            .ok_or(Error::Limit)?;
        if count > MAX_POLES {
            return Err(Error::Limit);
        }
        let mut work = Work {
            remaining: 1_000_000,
            interrupt,
        };
        for (index, points) in charts.iter().enumerate() {
            work.step()?;
            self.certify_control_chart(&tensors.walls[index % 3], points, &mut work)?;
        }
        Ok(())
    }
    fn certify_control_chart(
        &self,
        tensor: &ManufacturingTensor,
        points: &[[I; 3]],
        work: &mut Work<'_>,
    ) -> Result<(), Error> {
        certify_radial_control_chart(tensor.spans(), points, self.spec().left_hand, work)
    }
    fn build_hermite_wall(
        &self,
        band: usize,
        nu: usize,
        nz: usize,
        approximation: f64,
        tolerance: f64,
        work: &mut Work<'_>,
    ) -> Result<ManufacturingTensor, Error> {
        let width = D * nu + 1;
        let height = D * nz + 1;
        let mut poles = vec![[0.; 3]; width * height];
        let mut filled = vec![false; width * height];
        let mut error = 0f64;
        for iz in 0..nz {
            for iu in 0..nu {
                work.step()?;
                let us = [parameter(iu, nu, 1.), parameter(iu + 1, nu, 1.)];
                let zs = [
                    parameter(iz, nz, self.spec().length_mm),
                    parameter(iz + 1, nz, self.spec().length_mm),
                ];
                let du = I::point(1.) / I::point(nu as f64);
                let dz = I::point(self.spec().length_mm) / I::point(nz as f64);
                let mut corners = [[[Bi::<5, 5>::constant(0.); 3]; 2]; 2];
                for a in 0..2 {
                    for b in 0..2 {
                        let roots = self.axial_root_series(zs[b], work)?;
                        corners[a][b] = self.bi_surface(
                            band,
                            Bi::profile(us[a]),
                            Bi::axial(zs[b]),
                            roots_bi(roots),
                        );
                        if us[a].lo == us[a].hi && (us[a].lo == 0. || us[a].lo == 1.) {
                            let half = self.spec().crest_width_mm / 2.;
                            let (piece, d) = match (band, us[a].lo == 0.) {
                                (0, true) => (0, roots[0]),
                                (2, false) => (2, roots[1]),
                                (0, false) | (1, true) => (1, S::constant(-half)),
                                _ => (1, S::constant(half)),
                            };
                            let zz = S::variable(zs[b]);
                            let p = law::evaluate(self, piece, d, zz);
                            let [sin, cos] =
                                ((zz - d) / S::constant(self.source.helical_rate())).sin_cos();
                            let curve = [p.re * cos - p.im * sin, p.re * sin + p.im * cos, zz];
                            // Both adjacent walls use the same retained
                            // crease curve and its exact axial derivatives.
                            for axis in 0..3 {
                                for k in 0..JETS {
                                    corners[a][b][axis].c[0][k] = curve[axis].c[k];
                                }
                            }
                        }
                        for p in corners[a][b] {
                            p.checked()?;
                        }
                    }
                }
                for i in 0..=D {
                    for j in 0..=D {
                        let a = usize::from(i >= JETS);
                        let b = usize::from(j >= JETS);
                        let ui = if a == 0 { i } else { D - i };
                        let zj = if b == 0 { j } else { D - j };
                        let su = if a == 0 { du } else { -du };
                        let sz = if b == 0 { dz } else { -dz };
                        let mut value = [I::point(0.); 3];
                        for k in 0..=ui {
                            for l in 0..=zj {
                                let factor =
                                    weight(ui, k) * weight(zj, l) * power(su, k) * power(sz, l);
                                for (axis, sum) in value.iter_mut().enumerate() {
                                    *sum = *sum + factor * corners[a][b][axis].c[k][l];
                                }
                            }
                        }
                        for v in value {
                            v.checked()?;
                        }
                        let mut chosen = value.map(I::midpoint);
                        chosen[2] = parameter(iz * D + j, nz * D, self.spec().length_mm).midpoint();
                        let uncertainty = value
                            .into_iter()
                            .zip(chosen)
                            .fold(I::point(0.), |s, (v, p)| {
                                s + I::point((I::point(p) - v).magnitude())
                            });
                        error = error.max(uncertainty.hi);
                        // A control coefficient already exceeding the receipt
                        // budget cannot be repaired by constructing more poles.
                        if (I::point(error) + I::point(approximation)).hi > tolerance / 16. {
                            return Err(Error::Unresolved);
                        }
                        let index = (iu * D + i) * height + iz * D + j;
                        put_control(&mut poles, &mut filled, index, chosen)?;
                    }
                }
            }
        }
        let error = (I::point(error) + I::point(approximation)).hi;
        let tensor = ManufacturingTensor {
            band,
            profile_spans: nu,
            axial_spans: nz,
            poles,
            error_mm: error,
        };
        self.certify_hermite_regularity(&tensor, work)?;
        Ok(tensor)
    }
}
pub(super) fn certify_radial_control_chart(
    spans: [usize; 2],
    points: &[[I; 3]],
    forward: bool,
    work: &mut Work<'_>,
) -> Result<(), Error> {
    let [_, height] = super::retained::checked_grid(spans, points.len())?;
    for point in points {
        for coordinate in point {
            coordinate.checked()?;
        }
    }
    let uw = std::array::from_fn::<_, { D + 1 }, _>(|i| {
        std::array::from_fn::<_, D, _>(|a| {
            I::point(binomial(D, i) * binomial(D - 1, a)) / I::point(binomial(2 * D - 1, i + a))
        })
    });
    let zw = std::array::from_fn::<_, { D + 1 }, _>(|j| {
        std::array::from_fn::<_, { D + 1 }, _>(|b| {
            I::point(binomial(D, j) * binomial(D, b)) / I::point(binomial(2 * D, j + b))
        })
    });
    for iu in 0..spans[0] {
        for iz in 0..spans[1] {
            let mut jacobian = vec![I::point(0.); 2 * D * (2 * D + 1)];
            let point = |i: usize, j: usize| points[(iu * D + i) * height + iz * D + j];
            for (i, urow) in uw.iter().enumerate() {
                for (a, &uweight) in urow.iter().enumerate() {
                    work.step()?;
                    for (j, zrow) in zw.iter().enumerate() {
                        for (b, &zweight) in zrow.iter().enumerate() {
                            let p = point(i, j);
                            let from = point(a, b);
                            let to = point(a + 1, b);
                            let term = (p[0] * (to[1] - from[1]) - p[1] * (to[0] - from[0]))
                                * uweight
                                * zweight;
                            let index = (i + a) * (2 * D + 1) + j + b;
                            jacobian[index] = jacobian[index] + term;
                        }
                    }
                }
            }
            let mut minimum = f64::INFINITY;
            for coefficient in jacobian {
                let oriented = if forward { coefficient } else { -coefficient };
                minimum = minimum.min(oriented.positive()?.lo);
            }
            let mut zv = f64::INFINITY;
            let mut zu = 0f64;
            let mut rx = 0f64;
            let mut ry = 0f64;
            let mut vx = 0f64;
            let mut vy = 0f64;
            for i in 0..=D {
                for j in 0..=D {
                    let p = point(i, j);
                    rx = rx.max(p[0].magnitude());
                    ry = ry.max(p[1].magnitude());
                    if i < D {
                        zu = zu.max((point(i + 1, j)[2] - p[2]).magnitude());
                    }
                    if j < D {
                        let q = point(i, j + 1);
                        zv = zv.min((q[2] - p[2]).positive()?.lo);
                        vx = vx.max((q[0] - p[0]).magnitude());
                        vy = vy.max((q[1] - p[1]).magnitude());
                    }
                }
            }
            // det(theta,z) has numerator A_u*z_v - A_v*z_u.
            // The positive first term must dominate axial roundoff in u.
            let lower = I::point(minimum) * I::point(zv);
            let cross_bound = I::point(rx) * I::point(vy) + I::point(ry) * I::point(vx);
            (lower - cross_bound * I::point(zu)).positive()?;
        }
    }
    // Positive angular Jacobian and axial derivative certify a regular
    // radial graph, including very thin crests and true conical roots.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evaluate(tensor: &ManufacturingTensor, u: f64, v: f64) -> [f64; 3] {
        let [nu, nz] = tensor.spans();
        let [_, height] = tensor.dimensions();
        let i = ((u * nu as f64).floor() as usize).min(nu - 1);
        let j = ((v * nz as f64).floor() as usize).min(nz - 1);
        let tu = u * nu as f64 - i as f64;
        let tv = v * nz as f64 - j as f64;
        let mut rows = [[0.; 3]; D + 1];
        for (a, row) in rows.iter_mut().enumerate() {
            let mut p = std::array::from_fn::<_, { D + 1 }, _>(|b| {
                tensor.poles[(i * D + a) * height + j * D + b]
            });
            for n in (1..=D).rev() {
                for k in 0..n {
                    p[k] = std::array::from_fn(|axis| (1. - tv) * p[k][axis] + tv * p[k + 1][axis]);
                }
            }
            *row = p[0];
        }
        for n in (1..=D).rev() {
            for k in 0..n {
                rows[k] =
                    std::array::from_fn(|axis| (1. - tu) * rows[k][axis] + tu * rows[k + 1][axis]);
            }
        }
        rows[0]
    }
    #[test]
    fn actual_tensor_points_stay_inside_the_source_receipt() {
        let t = Thread::new(super::super::super::tests::spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
        let now = std::time::Instant::now();
        let tensors = t.manufacturing_tensors(1e-7, None).unwrap();
        eprintln!(
            "taper tensor build {:?} poles{}",
            now.elapsed(),
            tensors
                .walls()
                .iter()
                .map(|w| w.poles().len())
                .sum::<usize>()
        );
        assert!(tensors.error_bound_mm() <= 1e-7 / 16.);
        for tensor in tensors.walls() {
            for u in [0., 0.17, 0.5, 0.83, 1.] {
                for v in [0., 0.31, 0.7, 1.] {
                    let candidate = evaluate(tensor, u, v);
                    let source = t.surface_point_bounds(tensor.band(), u, 8. * v).unwrap();
                    let error = candidate
                        .into_iter()
                        .zip(source)
                        .map(|(c, [a, b])| (c - (a + b) * 0.5).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let band = tensor.band();
                    let receipt = tensor.error_bound_mm();
                    assert!(
                        error <= receipt + 1e-11,
                        "band{band} {u} {v} error{error} receipt{receipt}"
                    );
                }
            }
        }
    }
    #[test]
    fn manufacturing_proofs_bound_work_precision_and_shared_control_identity() {
        let t = Thread::new(super::super::super::tests::spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
        for tolerance in [0., -1., f64::NAN] {
            assert_eq!(
                t.manufacturing_tensors(tolerance, None),
                Err(Error::Invalid)
            );
        }
        assert_eq!(t.manufacturing_tensors(1e-90, None), Err(Error::Limit));
        assert_eq!(t.manufacturing_tensors(1e-12, None), Err(Error::Unresolved));
        assert_eq!(
            t.manufacturing_tensors(1e-7, Some(&|| true)),
            Err(Error::Cancelled)
        );
        let tensors = t.manufacturing_tensors(1e-7, None).unwrap();
        let charts = tensors
            .walls
            .iter()
            .map(|wall| {
                wall.poles
                    .iter()
                    .map(|p| p.map(I::point))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            t.certify_manufacturing_charts(&tensors, &[], None),
            Err(Error::Invalid)
        );
        let mut bad = charts.clone();
        bad[0].pop();
        assert_eq!(
            t.certify_manufacturing_charts(&tensors, &bad, None),
            Err(Error::Invalid)
        );
        let bad = vec![vec![[I::point(0.); 3]; MAX_POLES + 1], vec![], vec![]];
        assert_eq!(
            t.certify_manufacturing_charts(&tensors, &bad, None),
            Err(Error::Limit)
        );
        assert_eq!(
            t.certify_manufacturing_charts(&tensors, &charts, Some(&|| true)),
            Err(Error::Cancelled)
        );
        let mut bad = charts.clone();
        bad[0][0][0] = I::new(f64::NEG_INFINITY, f64::INFINITY);
        assert_eq!(
            t.certify_manufacturing_charts(&tensors, &bad, None),
            Err(Error::Unresolved)
        );
        let mut work = Work {
            remaining: 1_000_000,
            interrupt: None,
        };
        let b = t.hermite_bounds(&mut work).unwrap();
        let (nu, nz, errors) = t
            .certify_partition(b, [1; 3], 1, 1e-7 / 64., &mut work)
            .unwrap();
        assert!(
            nu.into_iter().any(|n| n > 1) && nz > 1 && errors.into_iter().all(|e| e <= 1e-7 / 64.)
        );
        let [nu, nz] = tensors.walls[0].spans();
        assert_eq!(
            t.build_hermite_wall(0, nu, nz, 0., 1e-30, &mut work),
            Err(Error::Unresolved)
        );
        let mut walls = tensors.walls.clone();
        walls[1].poles[0][0] = walls[1].poles[0][0].next_up();
        assert_eq!(shared_creases(&walls), Err(Error::Unresolved));
        let mut poles = vec![[0.; 3]];
        let mut filled = vec![false];
        let p = tensors.walls[0].poles[0];
        put_control(&mut poles, &mut filled, 0, p).unwrap();
        put_control(&mut poles, &mut filled, 0, p).unwrap();
        let changed = [p[0].next_up(), p[1], p[2]];
        assert_eq!(
            put_control(&mut poles, &mut filled, 0, changed),
            Err(Error::Unresolved)
        );
        assert_eq!(poles[0], p);
    }
}
