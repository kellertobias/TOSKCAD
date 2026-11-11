use super::*;
use crate::{display::Tessellation, manufacturing::certify};
use std::f64::consts::PI;
const TOL: f64 = 1e-7;
const MIN: f64 = 1e-5;
fn rectangle(min: [f64; 2], max: [f64; 2]) -> Vec<Edge> {
    let p = [min, [max[0], min[1]], max, [min[0], max[1]]];
    (0..4)
        .map(|i| Edge::Line {
            a: p[i],
            b: p[(i + 1) % 4],
        })
        .collect()
}
fn circle(center: [f64; 2], radius_mm: f64, clockwise: bool) -> Vec<Edge> {
    vec![Edge::Circle {
        center,
        radius_mm,
        clockwise,
    }]
}
fn base() -> Sweep {
    Sweep::new(
        vec![rectangle([0., 0.], [20., 16.]), circle([10., 8.], 1., true)],
        4.,
        TOL,
        MIN,
    )
    .unwrap()
}
fn motifs() -> Vec<Sweep> {
    vec![
        Sweep::new(vec![circle([3., 3.], 0.75, false)], 1.5, TOL, MIN).unwrap(),
        Sweep::new(
            vec![vec![
                Edge::Line {
                    a: [12., 2.],
                    b: [14., 2.],
                },
                Edge::Arc {
                    center: [14., 3.],
                    a: [14., 2.],
                    b: [14., 4.],
                    clockwise: false,
                },
                Edge::Line {
                    a: [14., 4.],
                    b: [12., 4.],
                },
                Edge::Line {
                    a: [12., 4.],
                    b: [12., 2.],
                },
            ]],
            0.6,
            TOL,
            MIN,
        )
        .unwrap(),
    ]
}
fn options() -> Tessellation {
    Tessellation {
        deflection: 0.001,
        angle: 0.08,
        triangle_limit: 100_000,
    }
}
#[test]
fn exact_mixed_relief_metrics_and_one_shared_closed_mesh() {
    for kind in [Kind::Raised, Kind::Recessed] {
        let (base, motifs) = (base(), motifs());
        let before = (base.clone(), motifs.clone());
        let relief = Relief::new(base, motifs, kind, TOL, MIN).unwrap();
        let added = 0.75_f64.powi(2) * PI * 1.5 + (4. + PI / 2.) * 0.6;
        let expected = (320. - PI) * 4. + if kind == Kind::Raised { added } else { -added };
        assert!((relief.volume_mm3() - expected).abs() < 1e-10);
        assert_eq!(relief.boundary().faces.len(), 14);
        assert_eq!(
            (relief.base(), relief.motifs()),
            (&before.0, before.1.as_slice())
        );
        let graph = relief.boundary();
        let mut uses = vec![vec![]; graph.edges.len()];
        for (face, node) in graph.faces.iter().enumerate() {
            for edge in node.loops.iter().flatten() {
                uses[edge.edge].push((face, edge.forward));
            }
        }
        assert!(
            uses.iter()
                .all(|edge| edge.len() == 2 && edge[0].1 != edge[1].1)
        );
        let mesh = relief.mesh(options()).unwrap();
        let proof = certify(&mesh).unwrap();
        assert!((proof.signed_volume_mm3 - relief.volume_mm3()).abs() < 0.05);
        assert_eq!(
            mesh.face_indices
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            (0..14).collect()
        );
        assert!(relief.contains([3., 3., if kind == Kind::Raised { 5. } else { 2. }], TOL));
        assert!(!relief.contains([10., 8., 2.], TOL));
        assert!(!relief.contains([3., 3., if kind == Kind::Raised { 6. } else { 3. }], TOL));
        assert_eq!(
            relief.bounds()[1][2],
            if kind == Kind::Raised { 5.5 } else { 4. }
        );
    }
}
#[test]
fn all_192_motifs_are_certified_without_iterated_solid_booleans() {
    let base = Sweep::new(vec![rectangle([0., 0.], [100., 80.])], 4., TOL, MIN).unwrap();
    let motifs = (0..192)
        .map(|i| {
            Sweep::new(
                vec![circle(
                    [3. + (i % 16) as f64 * 6., 3. + (i / 16) as f64 * 6.],
                    0.5,
                    false,
                )],
                0.5 + (i % 3) as f64 * 0.1,
                TOL,
                MIN,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let relief = Relief::new(base, motifs, Kind::Raised, TOL, MIN).unwrap();
    eprintln!("192 relief analytic motifs: {:?}", started.elapsed());
    assert_eq!(relief.motifs().len(), 192);
    assert_eq!(relief.boundary().faces.len(), 390);
    certify(&relief.mesh(options()).unwrap()).unwrap();
}

#[test]
fn full_public_density_keeps_fifty_exact_line_segments_per_motif() {
    let corners = [[-50., -40.], [50., -40.], [50., 40.], [-50., 40.]];
    let points = (0..1024)
        .map(|i| {
            let a = corners[i / 256];
            let b = corners[(i / 256 + 1) % 4];
            let t = (i % 256) as f64 / 256.;
            std::array::from_fn(|j| a[j] + (b[j] - a[j]) * t)
        })
        .collect::<Vec<_>>();
    let base = Sweep::new(
        vec![
            (0..1024)
                .map(|i| Edge::Line {
                    a: points[i],
                    b: points[(i + 1) % 1024],
                })
                .collect(),
        ],
        4.,
        TOL,
        MIN,
    )
    .unwrap();
    let motifs = (0..192)
        .map(|i| {
            let center = [-47. + (i % 16) as f64 * 6., -37. + (i / 16) as f64 * 6.];
            let points = (0..50)
                .map(|j| {
                    let (s, c) = (std::f64::consts::TAU * j as f64 / 50.).sin_cos();
                    [center[0] + 0.5 * c, center[1] + 0.5 * s]
                })
                .collect::<Vec<_>>();
            // The authored profile is an exact polygon, not a sampled circle.
            let ring = (0..50)
                .map(|j| Edge::Line {
                    a: points[j],
                    b: points[(j + 1) % 50],
                })
                .collect();
            Sweep::new(vec![ring], 0.5, TOL, MIN).unwrap()
        })
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let relief = Relief::new(base, motifs, Kind::Raised, TOL, MIN).unwrap();
    eprintln!("192x50-edge cap proof: {:?}", started.elapsed());
    assert_eq!(
        relief
            .exposed_cap()
            .rings()
            .iter()
            .map(Vec::len)
            .sum::<usize>(),
        10624
    );
    assert_eq!(
        Sweep::new(relief.exposed_cap().rings().to_vec(), 4., TOL, MIN),
        Err(crate::normal_sweep::Error::Limit)
    );
    certify(&relief.mesh(options()).unwrap()).unwrap();
}
#[test]
fn bad_contact_overlap_holes_floor_and_cancellation_leave_inputs_unchanged() {
    let base = base();
    let retained = base.clone();
    for motif in [
        Sweep::new(vec![circle([0., 3.], 1., false)], 1., TOL, MIN).unwrap(),
        Sweep::new(vec![circle([1., 3.], 1., false)], 1., TOL, MIN).unwrap(),
        Sweep::new(vec![circle([10., 8.], 0.5, false)], 1., TOL, MIN).unwrap(),
    ] {
        assert!(Relief::new(base.clone(), vec![motif], Kind::Raised, TOL, MIN).is_err());
    }
    let motif = motifs().remove(0);
    assert!(
        Relief::new(
            base.clone(),
            vec![motif.clone(), motif.clone()],
            Kind::Raised,
            TOL,
            MIN
        )
        .is_err()
    );
    let full = Sweep::new(motif.rings().to_vec(), 4., TOL, MIN).unwrap();
    assert_eq!(
        Relief::new(base.clone(), vec![full], Kind::Recessed, TOL, MIN).err(),
        Some(Error::Invalid)
    );
    assert_eq!(
        Relief::new(
            base.clone(),
            vec![motif.clone(); 193],
            Kind::Raised,
            TOL,
            MIN
        )
        .err(),
        Some(Error::Limit)
    );
    let calls = std::cell::Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 20
    };
    assert_eq!(
        Relief::new_with_interrupt(
            base.clone(),
            vec![motif.clone()],
            Kind::Raised,
            TOL,
            MIN,
            Some(&stop)
        )
        .err(),
        Some(Error::Cancelled)
    );
    assert_eq!(base, retained);
    Relief::new(base, vec![motif], Kind::Raised, TOL, MIN).unwrap();
}

#[test]
fn positive_column_moments_surface_properties_and_curve_charts_are_analytic() {
    let base = Sweep::new(vec![rectangle([0., 0.], [10., 8.])], 4., TOL, MIN).unwrap();
    let motifs = vec![
        Sweep::new(vec![rectangle([1., 2.], [3., 3.])], 1.5, TOL, MIN).unwrap(),
        Sweep::new(vec![circle([7., 5.], 0.5, false)], 0.75, TOL, MIN).unwrap(),
    ];
    for kind in [Kind::Raised, Kind::Recessed] {
        let relief = Relief::new(base.clone(), motifs.clone(), kind, TOL, MIN).unwrap();
        let sign = if kind == Kind::Raised { 1. } else { -1. };
        let pieces = [
            (3., [2., 2.5, 4. + sign * 0.75]),
            (PI * 0.25 * 0.75, [7., 5., 4. + sign * 0.375]),
        ];
        let volume = 320. + sign * pieces.iter().map(|p| p.0).sum::<f64>();
        let expected: [f64; 3] = std::array::from_fn(|i| {
            (320. * [5., 4., 2.][i] + sign * pieces.iter().map(|(v, c)| v * c[i]).sum::<f64>())
                / volume
        });
        assert!((relief.volume_mm3() - volume).abs() < 1e-12);
        assert!(
            relief
                .centroid_mm()
                .into_iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 1e-12)
        );
        assert_eq!(relief.kind(), kind);
        assert_eq!(relief.tolerance_mm(), TOL);
        assert_eq!(relief.minimum_mm(), MIN);
        assert!(relief.boundary_error_mm() >= 0. && relief.boundary_error_mm() < TOL / 32.);
        assert!(relief.contains([9., 7., 2.], TOL));
        assert_eq!(relief.exposed_cap().rings().len(), 3);
        let area = 304. + 1.5 * 6. + 0.75 * PI;
        assert!((relief.surface_area_mm2() - area).abs() < 1e-12);
        assert!(
            (relief
                .boundary()
                .faces
                .iter()
                .map(|f| f.area_mm2)
                .sum::<f64>()
                - area)
                .abs()
                < 1e-12
        );
        for edge in &relief.boundary().edges {
            assert_eq!(
                edge.curve.point(0.),
                relief.boundary().vertices[edge.vertices[0]]
            );
            assert_eq!(
                edge.curve.point(1.),
                relief.boundary().vertices[edge.vertices[1]]
            );
            assert!(edge.curve.point(0.5).into_iter().all(f64::is_finite));
        }
        assert!(!relief.contains([f64::NAN, 0., 0.], TOL));
        assert!(!relief.contains([0., 0., 0.], -1.));
        assert!(!relief.contains([-1., 0., 0.], TOL));
    }
}

#[test]
fn request_precision_floor_capacity_and_every_cancellation_boundary_fail_closed() {
    let base = base();
    let motif = motifs().remove(0);
    for (tolerance, minimum) in [(f64::NAN, MIN), (0., MIN), (TOL, TOL)] {
        assert_eq!(
            Relief::new(
                base.clone(),
                vec![motif.clone()],
                Kind::Raised,
                tolerance,
                minimum
            )
            .err(),
            Some(Error::Invalid)
        );
    }
    assert_eq!(
        Relief::new(base.clone(), vec![], Kind::Raised, TOL, MIN).err(),
        Some(Error::Invalid)
    );
    let hole_motif = Sweep::new(
        vec![rectangle([1., 1.], [5., 5.]), circle([3., 3.], 0.5, true)],
        1.,
        TOL,
        MIN,
    )
    .unwrap();
    assert_eq!(
        Relief::new(base.clone(), vec![hole_motif], Kind::Raised, TOL, MIN).err(),
        Some(Error::Invalid)
    );
    let tall = Sweep::new(motif.rings().to_vec(), 1e7, TOL, MIN).unwrap();
    assert_eq!(
        Relief::new(base.clone(), vec![tall], Kind::Raised, TOL, MIN).err(),
        Some(Error::Limit)
    );
    assert_eq!(
        Relief::new(
            base.clone(),
            vec![motif.clone()],
            Kind::Raised,
            1e-13,
            1e-12
        )
        .err(),
        Some(Error::Unresolved)
    );
    let tiny = Sweep::new(vec![circle([3., 3.], 1e-8, false)], 1., 1e-10, 1e-9).unwrap();
    assert_eq!(
        Relief::new(base.clone(), vec![tiny], Kind::Raised, 1e-10, 1e-9).err(),
        Some(Error::Unresolved)
    );
    let outside = Sweep::new(vec![rectangle([25., 2.], [26., 3.])], 1., TOL, MIN).unwrap();
    assert_eq!(
        Relief::new(base.clone(), vec![outside], Kind::Raised, TOL, MIN).err(),
        Some(Error::Invalid)
    );
    let mut arc = motifs().remove(1).rings().to_vec();
    if let Edge::Arc { b, .. } = &mut arc[0][1] {
        b[1] += 1e-9;
    }
    if let Edge::Line { a, .. } = &mut arc[0][2] {
        a[1] += 1e-9;
    }
    let arc = Sweep::new(arc, 1., TOL, MIN).unwrap();
    assert_eq!(
        Relief::new(base.clone(), vec![arc], Kind::Raised, TOL, MIN).err(),
        Some(Error::Unresolved)
    );
    let calls = std::cell::Cell::new(0);
    let count = || {
        calls.set(calls.get() + 1);
        false
    };
    let retained = Relief::new_with_interrupt(
        base.clone(),
        vec![motif.clone()],
        Kind::Raised,
        TOL,
        MIN,
        Some(&count),
    )
    .unwrap();
    for stop in 1..=calls.get() {
        let seen = std::cell::Cell::new(0);
        let cancelled = || {
            seen.set(seen.get() + 1);
            seen.get() >= stop
        };
        assert_eq!(
            Relief::new_with_interrupt(
                base.clone(),
                vec![motif.clone()],
                Kind::Raised,
                TOL,
                MIN,
                Some(&cancelled)
            )
            .err(),
            Some(Error::Cancelled),
            "checkpoint{stop}"
        );
    }
    assert_eq!(
        retained,
        Relief::new(base, vec![motif], Kind::Raised, TOL, MIN).unwrap()
    );
}

#[test]
fn large_combined_cap_and_underflowing_mass_are_explicitly_refused() {
    let ring = (0..1024)
        .map(|i| {
            let point = |j: usize| {
                let a = std::f64::consts::TAU * j as f64 / 1024.;
                [3. + 0.5 * a.cos(), 3. + 0.5 * a.sin()]
            };
            Edge::Line {
                a: point(i),
                b: point((i + 1) % 1024),
            }
        })
        .collect::<Vec<_>>();
    let motif = Sweep::new(vec![ring.clone()], 1., TOL, MIN).unwrap();
    assert_eq!(
        Relief::new(base(), vec![motif; 17], Kind::Raised, TOL, MIN).err(),
        Some(Error::Limit)
    );
    let oversized = Sweep::relief_cap(vec![ring; 17], 1., TOL, MIN, None).unwrap_err();
    assert_eq!(cap_error(oversized), Error::Limit);
    let base = Sweep::new(
        vec![rectangle([0., 0.], [1e-13, 1e-13])],
        1e-300,
        1e-311,
        1e-310,
    )
    .unwrap();
    let motif = Sweep::new(
        vec![rectangle([2e-14, 2e-14], [4e-14, 4e-14])],
        1e-300,
        1e-311,
        1e-310,
    )
    .unwrap();
    assert_eq!(
        Relief::new(base, vec![motif], Kind::Raised, 1e-311, 1e-310).err(),
        Some(Error::Unresolved)
    );
}

#[test]
fn step_precision_and_cached_boundary_failures_roll_back_the_writer() {
    use crate::step::{StepError, Writer, write_normal_relief};
    let relief = Relief::new(base(), motifs(), Kind::Raised, TOL, MIN).unwrap();
    let origin = [0.; 3];
    let u = [1., 0., 0.];
    let v = [0., 1., 0.];
    let expected = write_normal_relief(&relief, origin, u, v).unwrap();
    let mut invalid = relief.clone();
    invalid.volume += 1.;
    assert_eq!(
        write_normal_relief(&invalid, origin, u, v).err(),
        Some(StepError::Unsupported)
    );
    let mut periodic = relief.clone();
    let edge = periodic
        .boundary
        .edges
        .iter_mut()
        .find(|e| {
            matches!(
                e.curve,
                Curve::Profile {
                    edge: Edge::Circle { .. },
                    ..
                }
            )
        })
        .unwrap();
    edge.vertices[1] = 0;
    assert_eq!(
        write_normal_relief(&periodic, origin, u, v).err(),
        Some(StepError::InvalidGeometry)
    );
    let mut arc = relief.clone();
    let curve = arc
        .boundary
        .edges
        .iter_mut()
        .find(|e| {
            matches!(
                e.curve,
                Curve::Profile {
                    edge: Edge::Arc { .. },
                    ..
                }
            )
        })
        .unwrap();
    if let Curve::Profile {
        edge: Edge::Arc { center, .. },
        ..
    } = &mut curve.curve
    {
        center[1] += 0.1;
    }
    assert!(write_normal_relief(&arc, origin, u, v).is_err());
    let huge = Sweep::new(vec![rectangle([0., 0.], [1e6, 1e6])], 4., TOL, MIN).unwrap();
    let motif = Sweep::new(vec![rectangle([1., 1.], [3., 3.])], 1., TOL, MIN).unwrap();
    let huge = Relief::new(huge, vec![motif], Kind::Raised, TOL, MIN).unwrap();
    assert_eq!(
        write_normal_relief(&huge, origin, [(1. + 8e-11_f64).sqrt(), 0., 0.], v).err(),
        Some(StepError::Unsupported)
    );
    assert_eq!(
        write_normal_relief(&relief, [1e15; 3], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]).err(),
        Some(StepError::Unsupported)
    );
    let mut writer = Writer::new();
    assert!(writer.append_normal_relief(&invalid, origin, u, v).is_err());
    assert!(
        writer
            .append_normal_relief(&relief, origin, [0.; 3], v)
            .is_err()
    );
    writer.append_normal_relief(&relief, origin, u, v).unwrap();
    assert_eq!(writer.finish().unwrap(), expected);
    // Proper subnormal rotations exercise the same outward arithmetic guards
    // without losing a nonzero product merely because it underflows f64.
    write_normal_relief(&relief, origin, [1., 1e-200, 0.], [-1e-200, 1., 0.]).unwrap();
}
