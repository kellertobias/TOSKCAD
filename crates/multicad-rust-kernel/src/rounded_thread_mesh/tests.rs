use super::*;
use crate::rounded_thread::Spec;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    f64::consts::PI,
};
fn thread(left_hand: bool) -> Thread {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
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
    .unwrap()
}
fn rd(internal: bool, left_hand: bool) -> Thread {
    let p = 3.175;
    let a = 15_f64.to_radians();
    let r = p * (a.cos() - a.sin()) / (4. * (1. - a.sin()));
    let d = if internal {
        0.05 * p * a.sin() / (1. - a.sin())
    } else {
        0.
    };
    Thread::new(Spec {
        major_radius_mm: 10. + if internal { 0.05 * p } else { 0. },
        pitch_mm: p,
        length_mm: 2.37 * p,
        depth_mm: p / 2.,
        crest_radius_mm: r - d,
        root_radius_mm: r + d,
        flank_angle_deg: 15.,
        starts: 1,
        left_hand,
    })
    .unwrap()
}
fn options() -> Tessellation {
    Tessellation {
        deflection: 0.05,
        angle: 0.3,
        triangle_limit: 500_000,
    }
}
fn verify(t: &Thread, o: Tessellation) -> DisplayMesh {
    let mesh = tessellate(t, o).unwrap();
    let proof = crate::manufacturing::certify(&mesh).unwrap();
    assert!((proof.signed_volume_mm3 - t.volume_mm3()).abs() <= volume_error_bound(t, o).unwrap());
    assert_eq!(proof.vertex_links as usize, mesh.vertices.len() / 3);
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
    assert_eq!(
        mesh.vertices.len() / 3 + mesh.indices.len() / 3,
        proof.edge_pairs as usize + 2
    );
    assert!(mesh.face_indices.len() <= o.triangle_limit);
    assert_eq!(
        mesh.face_indices.iter().copied().collect::<BTreeSet<_>>(),
        (0..7).collect()
    );
    let receipt = Receipt::new(t, o).unwrap();
    let vertices = mesh.vertices.as_chunks::<3>().0;
    let mut parent: Vec<usize> = (0..vertices.len()).collect();
    fn root(p: &mut [usize], mut v: usize) -> usize {
        while p[v] != v {
            p[v] = p[p[v]];
            v = p[v];
        }
        v
    }
    for p in vertices {
        assert!(p.iter().all(|x| x.is_finite()));
        assert!((0. ..=t.spec().length_mm).contains(&p[2]));
        if p[0] != 0. || p[1] != 0. {
            assert!(
                (p[0].hypot(p[1]) - t.radius(p[1].atan2(p[0]), p[2])).abs()
                    <= receipt.point_error * 2.
            );
        }
    }
    for (&face, &ids) in mesh
        .face_indices
        .iter()
        .zip(mesh.indices.as_chunks::<3>().0)
    {
        let points = ids.map(|i| vertices[i as usize]);
        for i in [1, 2] {
            let a = root(&mut parent, ids[0] as usize);
            let b = root(&mut parent, ids[i] as usize);
            parent[b] = a;
        }
        let u: [f64; 3] = std::array::from_fn(|i| points[1][i] - points[0][i]);
        let v: [f64; 3] = std::array::from_fn(|i| points[2][i] - points[0][i]);
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length = n[0].hypot(n[1]).hypot(n[2]);
        let n = n.map(|x| x / length);
        if face < 2 {
            assert!(
                points
                    .into_iter()
                    .all(|p| p[2] == if face == 0 { 0. } else { t.spec().length_mm })
            );
            assert_eq!(n, [0., 0., if face == 0 { -1. } else { 1. }]);
            continue;
        }
        let first = points[0][1].atan2(points[0][0]);
        let angles = points.map(|p| first + (p[1].atan2(p[0]) - first + PI).rem_euclid(TAU) - PI);
        for weights in [
            [1. / 3.; 3],
            [0.6, 0.2, 0.2],
            [0.2, 0.6, 0.2],
            [0.2, 0.2, 0.6],
        ] {
            let a = (0..3).map(|i| weights[i] * angles[i]).sum::<f64>();
            let p: [f64; 3] =
                std::array::from_fn(|j| (0..3).map(|i| weights[i] * points[i][j]).sum());
            let r = t.radius(a, p[2]);
            assert!((p[0] - r * a.cos()).hypot(p[1] - r * a.sin()) <= o.deflection);
            let expected = t.normal(a, p[2]);
            let dot = n
                .into_iter()
                .zip(expected)
                .map(|(a, b)| a * b)
                .sum::<f64>()
                .clamp(-1., 1.);
            assert!(
                dot.acos() <= o.angle + 1e-10,
                "facet angle {} > {}",
                dot.acos(),
                o.angle
            );
        }
    }
    let roots: BTreeSet<_> = (0..parent.len()).map(|i| root(&mut parent, i)).collect();
    assert_eq!(roots.len(), 1);
    mesh
}
fn mirror_stations(mesh: &DisplayMesh, left: bool, length: f64) -> Vec<Vec<[f64; 3]>> {
    let mut groups = vec![];
    for cap in [0., length] {
        let mut rim: Vec<_> = mesh
            .vertices
            .as_chunks::<3>()
            .0
            .iter()
            .copied()
            .filter(|p| p[2] == cap)
            .map(|mut p| {
                if left {
                    p[1] = -p[1];
                }
                p
            })
            .collect();
        rim.sort_by(|a, b| {
            a[1].atan2(a[0])
                .rem_euclid(TAU)
                .total_cmp(&b[1].atan2(b[0]).rem_euclid(TAU))
        });
        groups.push(rim);
    }
    let mut inside: Vec<_> = mesh
        .vertices
        .as_chunks::<3>()
        .0
        .iter()
        .copied()
        .filter(|p| p[2] > 0. && p[2] < length)
        .map(|mut p| {
            if left {
                p[1] = -p[1];
            }
            p
        })
        .collect();
    inside.sort_by(|a, b| {
        a[1].atan2(a[0])
            .rem_euclid(TAU)
            .total_cmp(&b[1].atan2(b[0]).rem_euclid(TAU))
    });
    let mut last = f64::NAN;
    for p in inside {
        let angle = p[1].atan2(p[0]).rem_euclid(TAU);
        if !last.is_finite() || (angle - last).abs() > 1e-11 {
            groups.push(vec![]);
            last = angle;
        }
        groups.last_mut().unwrap().push(p);
    }
    for g in groups.iter_mut().skip(2) {
        g.sort_by(|a, b| a[2].total_cmp(&b[2]));
    }
    groups
}
#[test]
fn rounded_wall_and_caps_share_one_closed_graph_with_mirror_stations() {
    for (a, b) in [
        (thread(false), thread(true)),
        (rd(false, false), rd(false, true)),
        (rd(true, false), rd(true, true)),
    ] {
        let o = options();
        let right = verify(&a, o);
        let left = verify(&b, o);
        assert_eq!(right.vertices.len(), left.vertices.len());
        assert_eq!(right.indices.len(), left.indices.len());
        let hist = |m: &DisplayMesh| {
            let mut h = BTreeMap::new();
            for i in &m.face_indices {
                *h.entry(*i).or_insert(0) += 1;
            }
            h
        };
        assert_eq!(hist(&right), hist(&left));
        let ar = mirror_stations(&right, false, a.spec().length_mm);
        let al = mirror_stations(&left, true, b.spec().length_mm);
        assert_eq!(ar.len(), al.len());
        let error = Receipt::new(&a, o).unwrap().point_error * 4.;
        for (a, b) in ar.iter().zip(al) {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                for i in 0..3 {
                    assert!(
                        (a[i] - b[i]).abs() <= error,
                        "mirror station {a:?} != {b:?}"
                    );
                }
            }
        }
        println!(
            "R={} p={} triangles={} per hand",
            a.spec().major_radius_mm,
            a.spec().pitch_mm,
            right.face_indices.len()
        );
    }
}
#[test]
fn preview_and_manufacturing_keep_their_requested_quality_and_converge() {
    let t = thread(false);
    let coarse = Tessellation {
        deflection: 0.2,
        angle: 0.6,
        triangle_limit: 150_000,
    };
    let fine = Tessellation {
        deflection: 0.01,
        angle: 0.2,
        triangle_limit: 1_000_000,
    };
    let a = verify(&t, coarse);
    let b = verify(&t, fine);
    assert!(a.indices.len() < b.indices.len());
    let error = |m: &DisplayMesh| {
        (crate::manufacturing::certify(m).unwrap().signed_volume_mm3 - t.volume_mm3()).abs()
    };
    assert!(error(&b) < error(&a));
    println!(
        "preview {} triangles, manufacturing {} triangles",
        a.face_indices.len(),
        b.face_indices.len()
    );
}
#[test]
fn invalid_precision_and_budget_refusals_leave_source_and_retry_intact() {
    let t = thread(false);
    let before = t;
    for o in [
        Tessellation {
            deflection: 0.,
            ..options()
        },
        Tessellation {
            deflection: f64::NAN,
            ..options()
        },
        Tessellation {
            angle: 0.001,
            ..options()
        },
        Tessellation {
            angle: 1.1,
            ..options()
        },
        Tessellation {
            triangle_limit: 0,
            ..options()
        },
        Tessellation {
            triangle_limit: 5_000_001,
            ..options()
        },
    ] {
        assert_eq!(
            tessellate(&t, o),
            Err(Error::Mesh(MeshError::InvalidOptions))
        );
    }
    assert_eq!(
        tessellate(
            &t,
            Tessellation {
                deflection: 1e-30,
                ..options()
            }
        ),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    );
    for limit in [1, 100, 1000] {
        assert_eq!(
            tessellate(
                &t,
                Tessellation {
                    triangle_limit: limit,
                    ..options()
                }
            ),
            Err(Error::Mesh(MeshError::BudgetExceeded))
        );
    }
    assert!(tessellate(&t, options()).is_ok());
    assert_eq!(t, before);
    let mut work = Work::new(None);
    work.charge(MAX_WORK).unwrap();
    assert_eq!(work.charge(1), Err(Error::Mesh(MeshError::BudgetExceeded)));
    assert_eq!(
        work.charge(usize::MAX),
        Err(Error::Mesh(MeshError::BudgetExceeded))
    );
}
#[test]
fn every_one_shot_cancellation_checkpoint_aborts_and_retries_identically() {
    let t = thread(false);
    let o = Tessellation {
        deflection: 0.2,
        angle: 1.,
        triangle_limit: 100_000,
    };
    let count = Cell::new(0);
    let checkpoint = || {
        count.set(count.get() + 1);
        false
    };
    let expected = tessellate_with_interrupt(&t, o, Some(&checkpoint)).unwrap();
    let total = count.get();
    assert!(total > 10);
    for cutoff in 1..=total {
        let seen = Cell::new(0);
        let cancel = || {
            seen.set(seen.get() + 1);
            seen.get() == cutoff
        };
        assert_eq!(
            tessellate_with_interrupt(&t, o, Some(&cancel)),
            Err(Error::Cancelled)
        );
    }
    assert_eq!(tessellate(&t, o).unwrap(), expected);
    println!("{total} cancellation checkpoints");
}

#[test]
fn conforming_fan_choices_preserve_every_boundary_station_and_position() {
    let t = thread(false);
    let o = options();
    let mut work = Work::new(None);
    let receipt = Receipt::new(&t, o).unwrap();
    let steps = 500;
    let strips = phase_strips(&t, TAU / steps as f64, o, receipt, &mut work).unwrap();
    let n = strips.len() as i64;
    let r = n + strips.iter().position(|s| s.band == 1).unwrap() as i64;
    let mut b = Builder::new(&t, steps, strips, o, receipt, &mut work);
    let p = [
        b.vertex(VertexKey::Grid(0, r)).unwrap(),
        b.vertex(VertexKey::Grid(1, r)).unwrap(),
        b.vertex(VertexKey::Grid(1, r + 1)).unwrap(),
        b.vertex(VertexKey::Grid(0, r + 1)).unwrap(),
    ];
    let vertices = b.mesh.vertices.clone();
    let stations = b.vertex_ids.clone();
    for triangles in [
        [[p[0], p[1], p[2]], [p[0], p[2], p[3]]],
        [[p[1], p[2], p[3]], [p[1], p[3], p[0]]],
    ] {
        for triangle in triangles {
            assert!(b.check_triangle(triangle, 3).is_ok());
        }
    }
    b.triangulate_polygon(&p, 3).unwrap();
    assert_eq!(b.mesh.vertices, vertices);
    assert!(b.vertex_ids == stations);
    assert_eq!(b.mesh.face_indices.len(), 2);
    assert_eq!(
        b.triangulate_polygon(&[p[0]; 7], 3).err(),
        Some(AttemptError::Failure(Error::Mesh(
            MeshError::BudgetExceeded
        )))
    );
    assert!(b.triangulate_polygon(&p[..2], 3).is_ok());
}

#[test]
fn finite_chart_failure_and_capacity_contracts_recover_without_moving_stations() {
    let t = thread(false);
    let before = t;
    let o = options();
    let mut work = Work::new(None);
    let receipt = Receipt::new(&t, o).unwrap();
    let steps = 500;
    let strips = phase_strips(&t, TAU / steps as f64, o, receipt, &mut work).unwrap();
    let mut tiny = o;
    tiny.triangle_limit = 1;
    assert!(matches!(
        phase_strips(&t, TAU / steps as f64, tiny, receipt, &mut work),
        Err(Error::Mesh(MeshError::BudgetExceeded))
    ));
    let n = strips.len() as i64;
    let flank = n + strips.iter().position(|s| s.band == 1).unwrap() as i64;
    let mut b = Builder::new(&t, steps, strips, tiny, receipt, &mut work);
    // A deliberately coarse, otherwise finite wall cell must request angular
    // refinement before spending output capacity or changing its stations.
    let coarse = [
        b.vertex(VertexKey::Grid(0, flank)).unwrap(),
        b.vertex(VertexKey::Grid(50, flank)).unwrap(),
        b.vertex(VertexKey::Grid(50, flank + 1)).unwrap(),
    ];
    assert!(matches!(b.triangle(coarse, 3), Err(AttemptError::Refine)));
    assert!(b.mesh.indices.is_empty());
    let top = b.corner(0, 0, b.length.next_down());
    assert_eq!(top.z, b.length);
    assert!(matches!(
        b.corner_key(top),
        Some(VertexKey::CapColumn(0, 1))
    ));
    for mut ring in [vec![], vec![(0., 0), (0., 1), (1., 2)]] {
        assert!(Builder::sort_ring(&mut ring).is_err());
    }
    let ids = [0, 1, 2].map(|i| b.vertex(VertexKey::CapColumn(i, 0)).unwrap());
    let positions = b.mesh.vertices.clone();
    assert!(matches!(
        b.triangulate_polygon(&[ids[0]; 3], 0),
        Err(AttemptError::Failure(Error::Mesh(
            MeshError::UnresolvedBoundary
        )))
    ));
    // Wrong winding is a precision/geometry refusal, not a request to refine.
    assert!(matches!(
        b.triangle(ids, 0),
        Err(AttemptError::Failure(Error::Mesh(
            MeshError::UnresolvedBoundary
        )))
    ));
    b.triangle([ids[0], ids[2], ids[1]], 0).unwrap();
    assert!(matches!(
        b.emit_triangle(ids, 0),
        Err(AttemptError::Failure(Error::Mesh(
            MeshError::BudgetExceeded
        )))
    ));
    assert_eq!(b.mesh.vertices, positions);
    assert_eq!(b.mesh.indices.len(), 3);
    assert_eq!(t, before);
    assert!(tessellate(&t, o).is_ok());
}

#[test]
fn radial_volume_envelope_refuses_missing_inner_domain_and_numeric_overflow() {
    let t = thread(false);
    assert!(volume_error_bound(&t, options()).unwrap() > 0.);
    assert!(matches!(
        volume_error_bound(
            &t,
            Tessellation {
                deflection: 0.,
                ..options()
            }
        ),
        Err(Error::Mesh(MeshError::InvalidOptions))
    ));
    assert!(matches!(
        radial_graph_volume_bound(1., 1., 0., 0.1, 1.),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    ));
    assert!(matches!(
        radial_graph_volume_bound(f64::MAX, 1., 1., 0.1, 1.),
        Err(Error::Mesh(MeshError::UnresolvedBoundary))
    ));
    assert!(
        radial_graph_volume_bound(1., 1., 0.5, 0.1, 1.)
            .unwrap()
            .is_finite()
    );
}
