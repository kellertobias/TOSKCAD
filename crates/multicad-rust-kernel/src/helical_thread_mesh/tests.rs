use super::*;
use crate::{
    headed_screw::{Screw, ScrewFace, ScrewHead, ScrewSpec},
    helical_thread::Spec,
    manufacturing,
};
use std::{collections::BTreeSet, f64::consts::PI};

fn metric(left_hand: bool, starts: u32, rounded: bool, length_mm: f64) -> Thread {
    let pitch_mm = 1.25;
    let crest_width_mm = pitch_mm / 8.;
    let root_radius_mm = rounded.then_some(0.18);
    let depth_mm = root_radius_mm.map_or(5. * 3_f64.sqrt() / 16. * pitch_mm, |root| {
        (pitch_mm - crest_width_mm) / (2. * 30_f64.to_radians().tan()) - root
    });
    Thread::new(Spec {
        major_radius_mm: 4.,
        pitch_mm,
        length_mm,
        depth_mm,
        crest_width_mm,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        left_hand,
        starts,
        root_radius_mm,
    })
    .unwrap()
}

fn options() -> Tessellation {
    Tessellation {
        deflection: 0.1,
        angle: 0.3,
        triangle_limit: 150_000,
    }
}

fn verify(thread: &Thread, options: Tessellation) -> DisplayMesh {
    let mesh = tessellate(thread, options).unwrap();
    let proof = manufacturing::certify(&mesh).unwrap();
    assert_eq!(proof.vertex_links as usize, mesh.vertices.len() / 3);
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
    assert!(
        (proof.signed_volume_mm3 - thread.volume_mm3()).abs()
            <= volume_error_bound(thread, options)
    );
    let spec = thread.spec();
    let faces: BTreeSet<_> = mesh.face_indices.iter().copied().collect();
    assert_eq!(faces, (0..2 + 4 * spec.starts).collect());
    for &[x, y, z] in mesh.vertices.as_chunks::<3>().0 {
        assert!((0. ..=spec.length_mm).contains(&z));
        if [x, y] != [0.; 2] {
            let radius = thread.radius(y.atan2(x), z);
            assert!((x.hypot(y) - radius).abs() < 1e-10);
        } else {
            assert!(z == 0. || z == spec.length_mm);
        }
    }
    // Sample the surface paired with each side triangle at its barycenter.
    // Screw coordinates have affine depth, including clipped cap edges.
    for (&face, &[a, b, c]) in mesh
        .face_indices
        .iter()
        .zip(mesh.indices.as_chunks::<3>().0)
    {
        if face < 2 {
            continue;
        }
        let vertices = mesh.vertices.as_chunks::<3>().0;
        let points = [a, b, c].map(|index| vertices[index as usize]);
        let first = points[0][1].atan2(points[0][0]);
        let angle = points
            .iter()
            .map(|p| {
                let relative = p[1].atan2(p[0]) - first;
                first + (relative + TAU / 2.).rem_euclid(TAU) - TAU / 2.
            })
            .sum::<f64>()
            / 3.;
        let p: [f64; 3] =
            std::array::from_fn(|i| points.iter().map(|point| point[i]).sum::<f64>() / 3.);
        let radius = thread.radius(angle, p[2]);
        assert!(
            (p[0] - radius * angle.cos()).hypot(p[1] - radius * angle.sin()) <= options.deflection
        );
    }
    mesh
}

#[test]
fn long_metric_threads_share_caps_and_periodic_seams() {
    for length_mm in [50., 49.8] {
        let thread = metric(false, 1, false, length_mm);
        let mesh = verify(&thread, options());
        assert!(mesh.face_indices.len() < 25_000);
        let proof = manufacturing::certify(&mesh).unwrap();
        assert!((proof.signed_volume_mm3 / thread.volume_mm3() - 1.).abs() < 0.02);
    }
}

#[test]
fn rounded_left_hand_multistart_refines_without_tearing_profile_rails() {
    let thread = metric(true, 3, true, 4.7);
    let coarse = verify(&thread, options());
    let fine = verify(
        &thread,
        Tessellation {
            deflection: 0.005,
            angle: 0.12,
            ..options()
        },
    );
    assert!(fine.face_indices.len() > coarse.face_indices.len());
    let volume = |mesh| manufacturing::certify(mesh).unwrap().signed_volume_mm3;
    assert!(
        (volume(&fine) - thread.volume_mm3()).abs() < (volume(&coarse) - thread.volume_mm3()).abs()
    );
}

#[test]
fn threaded_bore_has_inward_flanks_and_watertight_annular_caps() {
    let options = Tessellation {
        deflection: 0.02,
        angle: 0.2,
        ..options()
    };
    for (thread, radius) in [
        (metric(false, 1, false, 7.4), 6.),
        (metric(true, 2, true, 3.7), 4.001),
    ] {
        let mesh = tessellate_bore(&thread, radius, options).unwrap();
        let proof = manufacturing::certify(&mesh).unwrap();
        assert_eq!(proof.vertex_links as usize, mesh.vertices.len() / 3);
        assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
        let expected = PI * radius.powi(2) * thread.spec().length_mm - thread.volume_mm3();
        let error = (proof.signed_volume_mm3 - expected).abs();
        assert!(error <= bore_volume_error_bound(&thread, radius, options));
        assert!(error / expected < 0.015);
        let outer_face = 2 + 4 * thread.spec().starts;
        let faces: BTreeSet<_> = mesh.face_indices.iter().copied().collect();
        assert_eq!(faces, (0..=outer_face).collect());
        let vertices = mesh.vertices.as_chunks::<3>().0;
        for (&face, &[a, b, c]) in mesh
            .face_indices
            .iter()
            .zip(mesh.indices.as_chunks::<3>().0)
        {
            let [a, b, c] = [a, b, c].map(|id| vertices[id as usize]);
            let [u, v] = [b, c].map(|p| std::array::from_fn::<_, 3, _>(|i| p[i] - a[i]));
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let mean: [f64; 3] = std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.);
            let radial_normal = normal[0] * mean[0] + normal[1] * mean[1];
            match face {
                0 => assert!(normal[2] < 0. && [a[2], b[2], c[2]] == [0.; 3]),
                1 => {
                    assert!(normal[2] > 0. && [a[2], b[2], c[2]] == [thread.spec().length_mm; 3])
                }
                index if index == outer_face => {
                    assert!(radial_normal > 0.);
                    assert!((radius - mean[0].hypot(mean[1])).abs() <= options.deflection);
                }
                _ => {
                    assert!(radial_normal < 0.);
                    for [x, y, z] in [a, b, c] {
                        assert!((x.hypot(y) - thread.radius(y.atan2(x), z)).abs() < 1e-10);
                    }
                }
            }
        }
    }
}

#[test]
fn bore_rejects_tangent_outer_stock_and_checks_final_triangle_budget() {
    let thread = metric(false, 1, false, 5.);
    for radius in [f64::INFINITY, 1e8, 4., 4. - 1e-12, 4. + f64::EPSILON] {
        assert_eq!(
            tessellate_bore(&thread, radius, options()),
            Err(MeshError::InvalidOptions)
        );
    }
    let triangles = tessellate_bore(&thread, 5., options())
        .unwrap()
        .face_indices
        .len();
    assert_eq!(
        tessellate_bore(
            &thread,
            5.,
            Tessellation {
                triangle_limit: triangles - 1,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
}

#[test]
fn screw_shell_keeps_hex_faces_and_coincident_shank_crests_watertight() {
    let options = Tessellation {
        deflection: 0.02,
        angle: 0.2,
        ..options()
    };
    let socket = ScrewHead::SocketCap {
        radius_mm: 6.5,
        height_mm: 8.,
        socket_across_flats_mm: 6.,
        socket_depth_mm: 4.4,
    };
    let hex = ScrewHead::ExternalHex {
        across_flats_mm: 13.,
        height_mm: 5.3,
    };
    for (head, shank_length_mm, shank_radius_mm, thread) in [
        (socket, 3., 4., metric(false, 1, false, 9.7)),
        (socket, 0., 4., metric(true, 1, false, 10.)),
        (hex, 3.3, 4.2, metric(true, 2, true, 10.)),
        (hex, 0., 4., metric(false, 2, true, 9.7)),
    ] {
        let screw = Screw::new(
            thread,
            ScrewSpec {
                head,
                shank_length_mm,
                shank_radius_mm,
            },
        )
        .unwrap();
        let mesh = tessellate_screw(&screw, options).unwrap();
        let proof = manufacturing::certify(&mesh).unwrap();
        assert_eq!(proof.vertex_links as usize, mesh.vertices.len() / 3);
        assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
        assert_eq!(
            i64::from(proof.vertex_links) - i64::from(proof.edge_pairs)
                + mesh.face_indices.len() as i64,
            2
        );
        let error = (proof.signed_volume_mm3 - screw.volume_mm3()).abs();
        assert!(error <= screw_volume_error_bound(&screw, options));
        assert!(error / screw.volume_mm3() < 0.01);
        let faces = screw.faces();
        let present: BTreeSet<_> = mesh.face_indices.iter().copied().collect();
        assert_eq!(present, (0..faces.len() as u32).collect());
        let vertices = mesh.vertices.as_chunks::<3>().0;
        let (head_top, seat) = (screw.head_top_z(), screw.head_seat_z());
        for (&face, &[a, b, c]) in mesh
            .face_indices
            .iter()
            .zip(mesh.indices.as_chunks::<3>().0)
        {
            let [a, b, c] = [a, b, c].map(|id| vertices[id as usize]);
            let [u, v] = [b, c].map(|point| std::array::from_fn::<_, 3, _>(|i| point[i] - a[i]));
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let [x, y, z] = std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.);
            let radial = x.hypot(y);
            let dot = normal[0] * x + normal[1] * y;
            let planar = |expected: f64, sign: f64| {
                assert_eq!([a[2], b[2], c[2]], [expected; 3]);
                assert!(sign * normal[2] > 0.);
            };
            let check_hex = |side, across_flats: f64| {
                let theta = f64::from(side + 1) * TAU / 6.;
                let distance = x * theta.cos() + y * theta.sin();
                assert!((distance - across_flats / 2.).abs() < 1e-10);
            };
            match faces[face as usize] {
                ScrewFace::ThreadShoulder => planar(0., 1.),
                ScrewFace::ThreadTip => planar(thread.spec().length_mm, 1.),
                ScrewFace::ThreadWall(_) => {
                    assert!(dot > 0.);
                    for [x, y, z] in [a, b, c] {
                        assert!((x.hypot(y) - thread.radius(y.atan2(x), z)).abs() < 1e-10);
                    }
                }
                ScrewFace::ShankWall => {
                    assert!(dot > 0.);
                    assert!((radial - shank_radius_mm).abs() < options.deflection);
                }
                ScrewFace::HeadSeat => planar(seat, 1.),
                ScrewFace::HeadTop => planar(head_top, -1.),
                ScrewFace::HeadWall(side) => {
                    assert!(dot > 0.);
                    match head {
                        ScrewHead::ExternalHex {
                            across_flats_mm, ..
                        } => check_hex(side, across_flats_mm),
                        ScrewHead::SocketCap { radius_mm, .. } => {
                            assert!((radial - radius_mm).abs() < options.deflection);
                        }
                    }
                }
                ScrewFace::SocketWall(side) => {
                    let (across_flats, depth) = head.socket().unwrap();
                    assert!(dot < 0. && z > head_top && z < head_top + depth);
                    check_hex(side, across_flats);
                }
                ScrewFace::SocketFloor => planar(head_top + head.socket().unwrap().1, -1.),
            }
        }
    }
}

#[test]
fn screw_budget_is_enforced_while_adding_head_faces() {
    for shank_length_mm in [0., 2.] {
        let screw = Screw::new(
            metric(false, 1, false, 10.),
            ScrewSpec {
                head: ScrewHead::ExternalHex {
                    across_flats_mm: 13.,
                    height_mm: 5.,
                },
                shank_length_mm,
                shank_radius_mm: 4.,
            },
        )
        .unwrap();
        let mesh = tessellate_screw(&screw, options()).unwrap();
        let faces = screw.faces();
        let first = |role| {
            mesh.face_indices
                .iter()
                .position(|&index| faces[index as usize] == role)
                .unwrap()
        };
        for triangle_limit in [
            1,
            first(ScrewFace::ThreadShoulder) + 1,
            first(ScrewFace::HeadWall(0)) + 1,
            mesh.face_indices.len() - 1,
        ] {
            assert_eq!(
                tessellate_screw(
                    &screw,
                    Tessellation {
                        triangle_limit,
                        ..options()
                    }
                ),
                Err(MeshError::BudgetExceeded)
            );
        }
    }
}

#[test]
fn invalid_options_and_exhausted_mesh_budgets_refuse_explicitly() {
    let thread = metric(false, 1, false, 5.);
    for options in [
        Tessellation {
            angle: 0.,
            ..options()
        },
        Tessellation {
            deflection: f64::NAN,
            ..options()
        },
        Tessellation {
            triangle_limit: 0,
            ..options()
        },
    ] {
        assert_eq!(tessellate(&thread, options), Err(MeshError::InvalidOptions));
    }
    for triangle_limit in [1, 100] {
        assert_eq!(
            tessellate(
                &thread,
                Tessellation {
                    triangle_limit,
                    ..options()
                }
            ),
            Err(MeshError::BudgetExceeded)
        );
    }
}

#[test]
fn phase_resolution_and_winding_guards_reject_partial_surfaces() {
    let thread = metric(true, 2, true, 2.5);
    assert_eq!(
        phase_strips(
            &thread,
            0.1,
            Tessellation {
                triangle_limit: 1,
                ..options()
            }
        )
        .err(),
        Some(MeshError::BudgetExceeded)
    );
    // A fixed angular interval that already exceeds the normal allowance
    // cannot be rescued by splitting phase, even down to adjacent floats.
    assert_eq!(
        phase_strips(&thread, options().angle * 2., options()).err(),
        Some(MeshError::UnresolvedBoundary)
    );
    let strips = phase_strips(&thread, 0.1, options()).unwrap();
    let mut builder = Builder::new(&thread, 64, strips, options());
    assert_eq!(
        builder.add_cap(0, vec![(0., 0), (0., 1), (1., 2)]),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(
        builder
            .add_annular_cap(0, vec![(0., 0), (0.2, 1), (1., 2)], 5.)
            .err(),
        Some(MeshError::UnresolvedBoundary)
    );
    assert_eq!(
        builder.triangle([1, 1, 2], 2),
        Err(MeshError::UnresolvedBoundary)
    );
    assert!(builder.mesh.vertices.is_empty());
    assert!(builder.mesh.indices.is_empty());
}
