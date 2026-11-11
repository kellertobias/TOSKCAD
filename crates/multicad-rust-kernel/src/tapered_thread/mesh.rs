//! Shared indexed cap/crest/flank/cone mesh, certified in source coordinates.
use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation};

pub fn tessellate(thread: &Thread, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    tessellate_with_interrupt(thread, options, None).map_err(mesh_error)
}
pub fn tessellate_with_interrupt(
    thread: &Thread,
    options: Tessellation,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<DisplayMesh, Error> {
    let cancelled = std::cell::Cell::new(false);
    // Every positive Work checkpoint propagates immediately out of build.
    let check = || {
        let value = interrupt.is_some_and(|f| f());
        cancelled.set(value);
        value
    };
    let result = build(thread, options, Some(&check));
    if cancelled.get() {
        return Err(Error::Cancelled);
    }
    result.map_err(|e| match e {
        MeshError::InvalidOptions => Error::Invalid,
        MeshError::BudgetExceeded => Error::Limit,
        MeshError::UnresolvedBoundary => Error::Unresolved,
    })
}
fn build(
    thread: &Thread,
    options: Tessellation,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<DisplayMesh, MeshError> {
    if !options.deflection.is_finite()
        || options.deflection <= 0.
        || !options.angle.is_finite()
        || !(0. ..=std::f64::consts::PI).contains(&options.angle)
        || options.angle == 0.
        || options.triangle_limit == 0
    {
        return Err(MeshError::InvalidOptions);
    }
    if options.triangle_limit < 16 * thread.spec().starts as usize {
        return Err(MeshError::BudgetExceeded);
    }
    let radial_extent = I::point(thread.spec().major_radius_mm)
        + I::point(thread.taper.abs()) * I::point(thread.spec().length_mm);
    let periodic_error = (I::point(32. * f64::EPSILON) * radial_extent).hi;
    I::point(periodic_error)
        .bounded_by(options.deflection * 0.125)
        .map_err(mesh_error)?;
    let mut nz = 1usize;
    let mut work = Work {
        remaining: 12_000_000,
        interrupt,
    };
    let bounds = bounds(thread, &mut work)?;
    let nu = loop {
        work.step().map_err(mesh_error)?;
        let dz = I::point(thread.spec().length_mm) / I::point(nz as f64);
        let counts = bounds.map(|b| (b.phase_width / dz.lo).round().max(1.) as usize);
        let per = counts
            .into_iter()
            .try_fold(0usize, |s, n| s.checked_add(n))
            .ok_or(MeshError::BudgetExceeded)?;
        let triangles = 2usize
            .checked_mul(thread.spec().starts as usize)
            .and_then(|n| n.checked_mul(per))
            .and_then(|n| n.checked_mul(nz + 1))
            .ok_or(MeshError::BudgetExceeded)?;
        if triangles > options.triangle_limit || triangles > 5_000_000 {
            return Err(MeshError::BudgetExceeded);
        }
        let accepted = bounds.into_iter().zip(counts).all(|(b, n)| {
            let dt = I::point(1.) / I::point(n as f64);
            let chord = (I::point(b.hessian[0]) * dt.square()
                + I::point(2. * b.hessian[1]) * dt * dz
                + I::point(b.hessian[2]) * dz.square())
                / I::point(8.);
            let normal = I::point(b.normal[0]) * dt + I::point(b.normal[1]) * dz;
            chord.hi <= options.deflection * 0.75 && normal.hi <= options.angle * 0.5
        });
        if accepted {
            break counts;
        }
        nz = nz.checked_mul(2).ok_or(MeshError::BudgetExceeded)?;
    };
    let per: usize = nu.into_iter().sum();
    let offsets = [0, nu[0], nu[0] + nu[1], nu[0] + nu[1] + nu[2]];
    let ring = per * thread.spec().starts as usize;
    let mut mesh = DisplayMesh {
        vertices: Vec::with_capacity(3 * ((nz + 1) * ring + 2)),
        indices: Vec::new(),
        face_indices: Vec::new(),
    };
    let mut normals = Vec::with_capacity((nz + 1) * ring);
    for iz in 0..=nz {
        let z = if iz == 0 {
            I::point(0.)
        } else if iz == nz {
            I::point(thread.spec().length_mm)
        } else {
            I::point(thread.spec().length_mm) * I::point(iz as f64) / I::point(nz as f64)
        };
        let roots = thread.roots(z).map_err(mesh_error)?;
        for start in 0..thread.spec().starts {
            for (band, &n) in nu.iter().enumerate() {
                for iu in 0..n {
                    work.step().map_err(mesh_error)?;
                    let t = I::point(iu as f64 / n as f64);
                    let [r, theta] = thread
                        .polar_patch_roots(band, t, z, roots)
                        .map_err(mesh_error)?;
                    let p = [r * theta.cos(), r * theta.sin(), J::z(z)];
                    let canonical = if band == 0 && iu == 0 {
                        let phase = thread.transport_j(0, roots[0], J::z(z)).phase;
                        let r = J::constant(thread.source.root_radius_mm())
                            + J::constant(thread.taper) * J::z(z);
                        let theta = (J::z(z) - phase) / J::constant(thread.source.helical_rate());
                        [r * theta.cos(), r * theta.sin(), J::z(z)]
                    } else {
                        p
                    };
                    let rotation = I::point(-std::f64::consts::TAU) * I::point(f64::from(start))
                        / I::point(f64::from(thread.spec().starts))
                        * I::point(if thread.spec().left_hand { -1. } else { 1. });
                    let sin = rotation.sin();
                    let cos = rotation.cos();
                    let exact = [
                        cos * p[0].v - sin * p[1].v,
                        sin * p[0].v + cos * p[1].v,
                        p[2].v,
                    ];
                    let chosen = [
                        cos * canonical[0].v - sin * canonical[1].v,
                        sin * canonical[0].v + cos * canonical[1].v,
                        canonical[2].v,
                    ]
                    .map(I::midpoint);
                    let error = std::array::from_fn::<_, 3, _>(|i| I::point(chosen[i]) - exact[i])
                        .into_iter()
                        .fold(I::point(0.), |s, e| s + I::point(e.magnitude()));
                    (error + I::point(periodic_error))
                        .bounded_by(options.deflection * 0.25)
                        .map_err(mesh_error)?;
                    let raw = [
                        p[1].d * p[2].z - p[2].d * p[1].z,
                        p[2].d * p[0].z - p[0].d * p[2].z,
                        p[0].d * p[1].z - p[1].d * p[0].z,
                    ];
                    let raw = if thread.spec().left_hand {
                        raw
                    } else {
                        raw.map(|v| -v)
                    };
                    let rotated = [
                        cos * raw[0] - sin * raw[1],
                        sin * raw[0] + cos * raw[1],
                        raw[2],
                    ];
                    normals.push(unit(rotated)?);
                    mesh.vertices.extend(chosen);
                }
            }
        }
    }
    let reverse = !thread.spec().left_hand;
    for iz in 0..nz {
        for j in 0..ring {
            let next = (j + 1) % ring;
            let [a, b, c, d] = [
                iz * ring + j,
                iz * ring + next,
                (iz + 1) * ring + j,
                (iz + 1) * ring + next,
            ];
            let start = j / per;
            let column = j % per;
            let band = (0..4)
                .rfind(|&band| column >= offsets[band])
                .ok_or(MeshError::UnresolvedBoundary)?;
            let face = 2 + (start * 4 + band) as u32;
            triangle(
                &mut mesh,
                [a, b, d],
                face,
                reverse,
                normals[a],
                options.angle * 0.5,
                &mut work,
            )?;
            triangle(
                &mut mesh,
                [a, d, c],
                face,
                reverse,
                normals[a],
                options.angle * 0.5,
                &mut work,
            )?;
        }
    }
    drop(normals);
    for end in 0..2 {
        let center = mesh.vertices.len() / 3;
        mesh.vertices.extend([
            0.,
            0.,
            if end == 0 {
                0.
            } else {
                thread.spec().length_mm
            },
        ]);
        let row = if end == 0 { 0 } else { nz * ring };
        certify_fan(&mesh, row, ring, reverse, &mut work)?;
        let expected = [
            I::point(0.),
            I::point(0.),
            I::point(if end == 0 { -1. } else { 1. }),
        ];
        for j in 0..ring {
            triangle(
                &mut mesh,
                [center, row + j, row + (j + 1) % ring],
                end as u32,
                reverse ^ (end == 0),
                expected,
                options.angle,
                &mut work,
            )?;
        }
    }
    Ok(mesh)
}
fn unit(normal: [I; 3]) -> Result<[I; 3], MeshError> {
    let length = normal
        .into_iter()
        .fold(I::point(0.), |s, n| s + n.square())
        .sqrt()
        .positive()
        .map_err(mesh_error)?;
    let result = normal.map(|n| n / length);
    for n in result {
        n.checked().map_err(mesh_error)?;
    }
    Ok(result)
}
fn triangle(
    mesh: &mut DisplayMesh,
    mut t: [usize; 3],
    face: u32,
    reverse: bool,
    source: [I; 3],
    angle: f64,
    work: &mut Work<'_>,
) -> Result<(), MeshError> {
    work.step().map_err(mesh_error)?;
    if reverse {
        t.swap(1, 2)
    }
    let p = t.map(|i| std::array::from_fn::<_, 3, _>(|axis| I::point(mesh.vertices[3 * i + axis])));
    let a = std::array::from_fn::<_, 3, _>(|i| p[1][i] - p[0][i]);
    let b = std::array::from_fn::<_, 3, _>(|i| p[2][i] - p[0][i]);
    let normal = unit([
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ])?;
    let dot = normal
        .into_iter()
        .zip(source)
        .fold(I::point(0.), |s, (n, p)| s + n * p);
    if dot.lo < I::point(angle).cos().hi {
        return Err(MeshError::UnresolvedBoundary);
    }
    mesh.triangle(t, face);
    Ok(())
}
fn certify_fan(
    mesh: &DisplayMesh,
    row: usize,
    count: usize,
    reverse: bool,
    work: &mut Work<'_>,
) -> Result<(), MeshError> {
    let mut wraps = 0;
    let at = |i: usize| {
        let index = if reverse {
            (count - i) % count
        } else {
            i % count
        };
        [
            mesh.vertices[3 * (row + index)],
            mesh.vertices[3 * (row + index) + 1],
        ]
    };
    let lower = |p: [f64; 2]| p[1] < 0. || (p[1] == 0. && p[0] < 0.);
    for i in 0..count {
        work.step().map_err(mesh_error)?;
        let a = at(i);
        let b = at(i + 1);
        (I::point(a[0]) * I::point(b[1]) - I::point(a[1]) * I::point(b[0]))
            .positive()
            .map_err(mesh_error)?;
        wraps += usize::from(lower(a) && !lower(b));
    }
    // Positive spoke crossings give strictly increasing angles <pi. One
    // ray wrap proves a single star-shaped boundary, without convexity.
    if wraps != 1 {
        return Err(MeshError::UnresolvedBoundary);
    }
    Ok(())
}
fn mesh_error(e: Error) -> MeshError {
    match e {
        Error::Invalid => MeshError::InvalidOptions,
        Error::Limit | Error::Cancelled => MeshError::BudgetExceeded,
        _ => MeshError::UnresolvedBoundary,
    }
}
#[derive(Clone, Copy, Default)]
struct Bounds {
    phase_width: f64,
    hessian: [f64; 3],
    normal: [f64; 2],
}
fn bounds(thread: &Thread, work: &mut Work<'_>) -> Result<[Bounds; 4], MeshError> {
    let mut bounds = [Bounds::default(); 4];
    // These are proof cells, independent of the eventual mesh density. Their
    // derivative enclosures cover the full parameter domain, not samples.
    for iz in 0..64 {
        let z = I::new(
            thread.spec().length_mm * f64::from(iz) / 64.,
            thread.spec().length_mm * f64::from(iz + 1) / 64.,
        );
        let roots = thread.roots(z).map_err(mesh_error)?;
        for (band, bounds) in bounds.iter_mut().enumerate() {
            for iu in 0..8 {
                work.step().map_err(mesh_error)?;
                let t = I::new(f64::from(iu) / 8., f64::from(iu + 1) / 8.);
                let [radius, theta] = thread
                    .polar_patch_roots(band, t, z, roots)
                    .map_err(mesh_error)?;
                bounds.phase_width = bounds.phase_width.max(
                    (I::point(theta.d.magnitude()) * I::point(thread.source.helical_rate().abs()))
                        .hi,
                );
                let p = [radius * theta.cos(), radius * theta.sin(), J::z(z)];
                for (axis, get) in [|j: J| j.dd, |j: J| j.dz, |j: J| j.zz]
                    .into_iter()
                    .enumerate()
                {
                    let bound = p
                        .into_iter()
                        .fold(I::point(0.), |s, j| s + I::point(get(j).magnitude()));
                    bound.checked().map_err(mesh_error)?;
                    bounds.hessian[axis] = bounds.hessian[axis].max(bound.hi);
                }
                let den = radius.v * theta.d;
                let den_t = radius.d * theta.d + radius.v * theta.dd;
                let den_z = radius.z * theta.d + radius.v * theta.dz;
                let at = -radius.dd / den + radius.d * den_t / den.square();
                let az = -radius.dz / den + radius.d * den_z / den.square();
                let bt = radius.dd * theta.z / theta.d + radius.d * theta.dz / theta.d
                    - radius.d * theta.z * theta.dd / theta.d.square()
                    - radius.dz;
                let bz = radius.dz * theta.z / theta.d + radius.d * theta.zz / theta.d
                    - radius.d * theta.z * theta.dz / theta.d.square()
                    - radius.zz;
                // In the rotating radial frame the normal is parallel to
                // (1,A,B). Its norm is ≥1 throughout each convex coefficient box,
                // so normalization is 1-Lipschitz. Add radial-basis rotation.
                for (axis, values) in [[theta.d, at, bt], [theta.z, az, bz]]
                    .into_iter()
                    .enumerate()
                {
                    let bound = values
                        .into_iter()
                        .fold(I::point(0.), |s, v| s + I::point(v.magnitude()));
                    bound.checked().map_err(mesh_error)?;
                    bounds.normal[axis] = bounds.normal[axis].max(bound.hi);
                }
            }
        }
    }
    Ok(bounds)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn work() -> Work<'static> {
        Work {
            remaining: 100,
            interrupt: None,
        }
    }
    #[test]
    fn facet_and_single_wrap_fan_proofs_reject_corruption_without_append() {
        let mut mesh = DisplayMesh {
            vertices: vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            indices: vec![],
            face_indices: vec![],
        };
        let expected = [I::point(0.), I::point(0.), I::point(1.)];
        triangle(&mut mesh, [0, 1, 2], 0, false, expected, 0.1, &mut work()).unwrap();
        let before = mesh.clone();
        assert_eq!(
            triangle(
                &mut mesh,
                [0, 1, 2],
                0,
                false,
                expected.map(|x| -x),
                0.1,
                &mut work()
            ),
            Err(MeshError::UnresolvedBoundary)
        );
        assert_eq!(mesh, before);
        assert_eq!(
            unit([I::point(0.); 3]).unwrap_err(),
            MeshError::UnresolvedBoundary
        );
        let ring = DisplayMesh {
            vertices: vec![1., 0., 0., 0., 1., 0., -1., 0., 0., 0., -1., 0.],
            indices: vec![],
            face_indices: vec![],
        };
        certify_fan(&ring, 0, 4, false, &mut work()).unwrap();
        assert_eq!(
            certify_fan(&ring, 0, 4, true, &mut work()),
            Err(MeshError::UnresolvedBoundary)
        );
        let mut twice = ring.clone();
        twice.vertices.extend(ring.vertices);
        assert_eq!(
            certify_fan(&twice, 0, 8, false, &mut work()),
            Err(MeshError::UnresolvedBoundary)
        );
        let mut exhausted = Work {
            remaining: 0,
            interrupt: None,
        };
        assert_eq!(
            certify_fan(&twice, 0, 8, false, &mut exhausted),
            Err(MeshError::BudgetExceeded)
        );
    }
    #[test]
    fn cancellation_at_both_wall_triangles_and_cap_publish_stays_atomic() {
        use std::cell::Cell;
        let t = Thread::new(super::super::tests::spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
        let options = Tessellation {
            deflection: 0.15,
            angle: 0.8,
            triangle_limit: 5_000_000,
        };
        let mesh = tessellate(&t, options).unwrap();
        let ring = mesh
            .vertices
            .as_chunks::<3>()
            .0
            .iter()
            .take_while(|p| p[2] == 0.)
            .count();
        let nz = (mesh.vertices.len() / 3 - 2) / ring - 1;
        let prefix = 2048 + nz.ilog2() as usize + 1 + (nz + 1) * ring;
        for cut in [prefix + 1, prefix + 2, prefix + 2 * nz * ring + ring + 1] {
            let calls = Cell::new(0);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() == cut
            };
            assert_eq!(
                tessellate_with_interrupt(&t, options, Some(&cancel)),
                Err(Error::Cancelled)
            );
            assert_eq!(calls.get(), cut);
        }
        assert_eq!(tessellate(&t, options).unwrap(), mesh);
    }
}
