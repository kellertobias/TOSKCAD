use super::*;
use crate::rounded_thread::Spec;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};
fn bore(left_hand: bool) -> ThroughBore {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    ThroughBore::new(
        Thread::new(Spec {
            major_radius_mm: 3.175,
            pitch_mm: p,
            length_mm: 3.7 * p,
            depth_mm: p / (3. * a.tan()),
            crest_radius_mm: r,
            root_radius_mm: r,
            flank_angle_deg: 27.5,
            starts: 1,
            left_hand,
        })
        .unwrap(),
        5.,
    )
    .unwrap()
}
fn options() -> Tessellation {
    Tessellation {
        deflection: 0.05,
        angle: 0.3,
        triangle_limit: 500_000,
    }
}
fn validate(mesh: &DisplayMesh, bore: &ThroughBore) {
    let point = |id: u32| {
        let i = id as usize * 3;
        [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]]
    };
    let mut edges = BTreeMap::new();
    let mut volume = 0.;
    let mut faces = BTreeSet::new();
    for (triangle, face) in mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&mesh.face_indices)
    {
        faces.insert(*face);
        for [a, b] in [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ] {
            let (key, sign) = edge_key([a, b]);
            let entry = edges.entry(key).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += sign;
        }
        let [a, b, c] = triangle.map(point);
        volume += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    assert!(edges.values().all(|&x| x == (2, 0)));
    assert_eq!(faces, (0..8).collect());
    assert_eq!(
        mesh.vertices.len() / 3 + mesh.face_indices.len(),
        edges.len()
    ); // genus one
    let volume = volume / 6.;
    assert!(volume > 0.);
    assert!(
        (volume - bore.volume_mm3()).abs() <= bore_volume_error_bound(bore, options()).unwrap()
    );
}
#[test]
fn both_hands_emit_closed_annular_material_with_all_eight_faces() {
    let mut counts = vec![];
    for left in [false, true] {
        let b = bore(left);
        let mesh = tessellate_bore(&b, options()).unwrap();
        validate(&mesh, &b);
        counts.push(mesh.indices.len());
        assert_eq!(mesh, tessellate_bore(&b, options()).unwrap());
    }
    assert_eq!(counts[0], counts[1]);
}
#[test]
fn budget_precision_and_cancellation_refuse_atomically_then_retry() {
    let b = bore(false);
    assert_eq!(
        tessellate_bore(
            &b,
            Tessellation {
                triangle_limit: 3,
                ..options()
            }
        ),
        Err(Error::Mesh(MeshError::BudgetExceeded))
    );
    let count = Cell::new(0);
    let mesh = tessellate_bore_with_interrupt(
        &b,
        options(),
        Some(&|| {
            count.set(count.get() + 1);
            false
        }),
    )
    .unwrap();
    for target in [1, count.get() / 2, count.get()] {
        let at = Cell::new(0);
        assert_eq!(
            tessellate_bore_with_interrupt(
                &b,
                options(),
                Some(&|| {
                    at.set(at.get() + 1);
                    at.get() == target
                })
            ),
            Err(Error::Cancelled)
        );
    }
    let tiny = ThroughBore::new(*b.thread(), b.thread().spec().major_radius_mm + 1e-12).unwrap();
    assert_eq!(
        tessellate_bore(&tiny, options()),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    assert_eq!(mesh, tessellate_bore(&b, options()).unwrap());
}

#[test]
fn outer_numeric_bounds_and_complete_sweep_are_independent_receipts() {
    assert!(prove_outer_polygon(5., 4., 3.2, 64).is_ok());
    assert_eq!(
        prove_outer_polygon(3.175001, 4., 3.175, 64),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    assert_eq!(
        closed_sweep([7, 8], [8, 8]),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    assert!(closed_sweep([8, 8], [8, 8]).is_ok());
    let b = bore(false);
    assert_eq!(
        bore_volume_error_bound(
            &b,
            Tessellation {
                deflection: 3e153,
                ..options()
            }
        ),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    let mesh = tessellate_bore(&b, options()).unwrap();
    assert_eq!(
        tessellate_bore(
            &b,
            Tessellation {
                triangle_limit: mesh.face_indices.len() - 1,
                ..options()
            }
        ),
        Err(Error::Mesh(MeshError::BudgetExceeded))
    );
}

#[test]
fn cap_edge_receipt_rejects_missing_wrong_and_nonmanifold_incidence() {
    let mut work = Work::new(None);
    let quad = |work: &mut Work<'_>| {
        let mut p = CapProof::default();
        p.add([0, 1, 2], work).unwrap();
        p.add([0, 2, 3], work).unwrap();
        p
    };
    assert!(
        quad(&mut work)
            .finish(vec![[0, 1], [1, 2], [2, 3], [3, 0]].into_iter(), &mut work)
            .is_ok()
    );
    assert_eq!(
        quad(&mut work).finish(vec![[0, 9]].into_iter(), &mut work),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    assert_eq!(
        quad(&mut work).finish(vec![[1, 0]].into_iter(), &mut work),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    assert_eq!(
        quad(&mut work).finish(vec![[0, 1]].into_iter(), &mut work),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    // A single concrete iterator contract exercises successful, invalid,
    // interrupted and limited graph proof before successful recovery.
    let boundary = vec![[0, 1], [1, 2], [2, 3], [3, 0]];
    let cancel = || true;
    for used in [127, 123] {
        let mut work = Work::new(Some(&cancel));
        let p = quad(&mut work);
        work.used = used;
        assert_eq!(
            p.finish(boundary.clone().into_iter(), &mut work),
            Err(Error::Cancelled)
        );
    }
    for used in [MAX_WORK, MAX_WORK - 4] {
        let mut work = Work::new(None);
        let p = quad(&mut work);
        work.used = used;
        assert_eq!(
            p.finish(boundary.clone().into_iter(), &mut work),
            Err(Error::Mesh(MeshError::BudgetExceeded))
        );
    }
    let mut recovered = Work::new(None);
    assert!(
        quad(&mut recovered)
            .finish(boundary.into_iter(), &mut recovered)
            .is_ok()
    );
    let mut p = quad(&mut work);
    assert_eq!(
        p.add([0, 1, 2], &mut work),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    let mut work = Work::new(None);
    work.used = MAX_WORK - 2;
    assert_eq!(
        CapProof::default().add([0, 1, 2], &mut work),
        Err(Error::Mesh(MeshError::BudgetExceeded))
    );
}

// A cap is a planar triangulation contract, independent of the analytic wall
// profile. Exercise actual finite triangle and graph failure propagation using
// concentric polygon fixtures; no manufactured Thread state is introduced.
fn cap_fixture(
    interrupt: Option<&dyn Fn() -> bool>,
    inner_radius: f64,
    first_angle: f64,
    inner_count: usize,
) -> Result<(), AttemptError> {
    let b = bore(false);
    let t = b.thread();
    let opt = options();
    let receipt = Receipt::new(t, opt)?;
    let mut work = Work::new(interrupt);
    let strips = phase_strips(t, 0.1, opt, receipt, &mut work)?;
    let mut builder = Builder::new(t, 64, strips, opt, receipt, &mut work);
    let mut inner = vec![];
    let mut outer = vec![];
    for (ring, radius) in [inner_radius, 2.].into_iter().enumerate() {
        let count = if ring == 0 { inner_count } else { 64 };
        for i in 0..count {
            let angle = TAU * i as f64 / count as f64;
            let (s, c) = angle.sin_cos();
            let id = builder.mesh.vertices.len() as u32 / 3;
            builder.mesh.vertices.extend([radius * c, radius * s, 0.]);
            builder.parameters.push([angle, 0.]);
            if ring == 0 {
                inner.push((if i == 0 { first_angle } else { angle }, id));
            } else {
                outer.push(id);
            }
        }
    }
    builder.bore_cap(0, &inner, &outer)
}
#[test]
fn finite_cap_and_every_cancel_checkpoint_refuse_without_leaking_partial_mesh() {
    assert!(cap_fixture(None, 1., 0., 64).is_ok());
    assert!(matches!(
        cap_fixture(None, 1., 0.1, 64),
        Err(AttemptError::Failure(Error::Mesh(
            MeshError::UnresolvedBoundary
        )))
    ));
    assert!(matches!(
        cap_fixture(None, 2., 0., 64),
        Err(AttemptError::Failure(Error::Mesh(
            MeshError::UnresolvedBoundary
        )))
    ));
    let count = Cell::new(0);
    cap_fixture(
        Some(&|| {
            count.set(count.get() + 1);
            false
        }),
        1.,
        0.,
        64,
    )
    .unwrap();
    for target in 1..=count.get() {
        let at = Cell::new(0);
        assert!(matches!(
            cap_fixture(
                Some(&|| {
                    at.set(at.get() + 1);
                    at.get() == target
                }),
                1.,
                0.,
                64
            ),
            Err(AttemptError::Failure(Error::Cancelled))
        ));
    }
    assert!(cap_fixture(None, 1., 0., 64).is_ok());
}
#[test]
fn polygon_receipt_requires_a_finite_nonempty_sampling_domain() {
    for (r, h, inner, steps) in [
        (f64::NAN, 1., 0.5, 64),
        (1., 0., 0.5, 64),
        (1., 1., -1., 64),
        (1., 1., 0.5, 0),
    ] {
        assert_eq!(
            prove_outer_polygon(r, h, inner, steps),
            Err(Error::Mesh(MeshError::UnresolvedBoundary))
        );
    }
    assert!(prove_outer_polygon(1., 1., 0.5, 64).is_ok());
}

#[test]
fn cap_sweep_handles_a_finer_outer_grid_without_dropping_inner_boundaries() {
    assert!(cap_fixture(None, 1., 0., 32).is_ok());
    assert!(cap_fixture(None, 1., 0., 64).is_ok());
}
