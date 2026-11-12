//! A retained finite solid without any authored thread recipe.
use multicad_rust_kernel::{
    radial_tensor::{
        Axis, BoundaryInput, BoundaryOwnership, Curve, Edge, Face, Location, RadialBoundary,
        Surface,
    },
    tapered_thread::RetainedTensor,
};
type Point = [f64; 3];
fn line(a: Point, b: Point) -> Curve {
    Curve::Spline {
        spans: 1,
        poles: (0..=9)
            .map(|i| std::array::from_fn(|k| a[k] + (b[k] - a[k]) * i as f64 / 9.))
            .collect(),
    }
}
fn wedge(longitudinal: &[usize]) -> BoundaryInput {
    let n = longitudinal.len();
    assert!((1..=2).contains(&n));
    let mut xy = Vec::new();
    for g in 0..n {
        let sign = if g == 0 { 1. } else { -1. };
        let gap = if n == 1 { [-5., 0.] } else { [0., 5. * sign] };
        xy.extend([
            [3. * sign, -4. * sign],
            [6. * sign, -2. * sign],
            [6. * sign, 2. * sign],
            [3. * sign, 4. * sign],
            gap,
        ]);
    }
    let vertices = (0..2)
        .flat_map(|end| xy.iter().map(move |p| [p[0], p[1], 4. * end as f64]))
        .collect::<Vec<_>>();
    let axis = |z| Axis {
        origin: [0., 0., z],
        normal: [0., 0., 1.],
        reference: [1., 0., 0.],
    };
    let mut curves = Vec::new();
    let mut edges = Vec::new();
    for end in 0..2 {
        for g in 0..n {
            for i in 0..5 {
                let a = 5 * n * end + 5 * g + i;
                let b = if i == 4 {
                    5 * n * end + 5 * ((g + 1) % n)
                } else {
                    a + 1
                };
                let curve = curves.len();
                curves.push(if i < 3 {
                    line(vertices[a], vertices[b])
                } else {
                    Curve::Circle {
                        axis: axis(4. * end as f64),
                        radius_mm: 5.,
                    }
                });
                edges.push(Edge {
                    vertices: [a, b],
                    curve,
                    forward: true,
                });
            }
        }
    }
    for (g, &spans) in longitudinal.iter().enumerate() {
        for i in 0..4 {
            let a = 5 * g + i;
            let b = 5 * n + a;
            let curve = curves.len();
            curves.push(Curve::Spline {
                spans,
                poles: (0..=9 * spans)
                    .map(|j| {
                        [
                            vertices[a][0],
                            vertices[a][1],
                            4. * j as f64 / (9 * spans) as f64,
                        ]
                    })
                    .collect(),
            });
            edges.push(Edge {
                vertices: [a, b],
                curve,
                forward: true,
            });
        }
    }
    let mut surfaces = vec![
        Surface::Plane(Axis {
            normal: [0., 0., -1.],
            ..axis(0.)
        }),
        Surface::Plane(axis(4.)),
    ];
    let mut faces = vec![
        Face {
            surface: 0,
            outer: (0..5 * n).rev().map(|i| (i, false)).collect(),
            forward: true,
        },
        Face {
            surface: 1,
            outer: (5 * n..10 * n).map(|i| (i, true)).collect(),
            forward: true,
        },
    ];
    for (g, &spans) in longitudinal.iter().enumerate() {
        for i in 0..3 {
            let Curve::Spline { poles: base, .. } = &curves[5 * g + i] else {
                unreachable!()
            };
            let poles = base
                .iter()
                .flat_map(|p| {
                    (0..=9 * spans).map(move |j| [p[0], p[1], 4. * j as f64 / (9 * spans) as f64])
                })
                .collect();
            let surface = surfaces.len();
            surfaces.push(Surface::Tensor(
                RetainedTensor::new([1, spans], poles).unwrap(),
            ));
            faces.push(Face {
                surface,
                outer: vec![
                    (5 * g + i, true),
                    (10 * n + 4 * g + i + 1, true),
                    (5 * n + 5 * g + i, false),
                    (10 * n + 4 * g + i, false),
                ],
                forward: true,
            });
        }
    }
    let surface = surfaces.len();
    surfaces.push(Surface::Cone {
        axis: axis(0.),
        radius_mm: 5.,
        semi_angle_rad: 0.,
    });
    for g in 0..n {
        faces.push(Face {
            surface,
            outer: vec![
                (5 * g + 3, true),
                (5 * g + 4, true),
                (10 * n + 4 * ((g + 1) % n), true),
                (5 * n + 5 * g + 4, false),
                (5 * n + 5 * g + 3, false),
                (10 * n + 4 * g + 3, false),
            ],
            forward: true,
        });
    }
    BoundaryInput {
        vertices,
        curves,
        edges,
        surfaces,
        faces,
    }
}
fn pose(mut input: BoundaryInput, origin: Point, axes: [Point; 3]) -> BoundaryInput {
    let vector = |p: Point| std::array::from_fn(|i| (0..3).map(|j| axes[j][i] * p[j]).sum::<f64>());
    let position = |p: Point| {
        let d: Point = vector(p);
        std::array::from_fn(|i| origin[i] + d[i])
    };
    let axis = |a: &mut Axis| {
        a.origin = position(a.origin);
        a.normal = vector(a.normal);
        a.reference = vector(a.reference);
    };
    for p in &mut input.vertices {
        *p = position(*p);
    }
    for curve in &mut input.curves {
        match curve {
            Curve::Spline { poles, .. } => {
                for p in poles {
                    *p = position(*p);
                }
            }
            Curve::Circle { axis: a, .. } => axis(a),
        }
    }
    for s in &mut input.surfaces {
        match s {
            Surface::Plane(a) | Surface::Cone { axis: a, .. } => axis(a),
            Surface::Tensor(t) => {
                *t = RetainedTensor::new(
                    t.spans(),
                    t.poles().iter().map(|&p| position(p)).collect(),
                )
                .unwrap();
            }
        }
    }
    input
}
fn certify(input: BoundaryInput) -> RadialBoundary {
    BoundaryOwnership::new(input, 1e-7, None)
        .unwrap()
        .bind_surfaces(1e-7, None)
        .unwrap()
        .certify_axial(1e-7, 1e-6, None)
        .unwrap()
        .certify_caps(None)
        .unwrap()
        .certify_ordering(None)
        .unwrap()
        .certify_material(1e-7, None)
        .unwrap()
}
#[test]
fn actual_nonhelical_nets_survive_blind_rust_readback_queries_and_exact_reexport() {
    let origin = [10., 20., 30.];
    let axes = [[0.8, 0.6, 0.], [-0.36, 0.48, 0.8], [0.48, -0.64, 0.6]];
    let vector = |p: Point| std::array::from_fn(|i| (0..3).map(|j| axes[j][i] * p[j]).sum::<f64>());
    let position = |p: Point| {
        let d: Point = vector(p);
        std::array::from_fn(|i| origin[i] + d[i])
    };
    let input = pose(wedge(&[1]), origin, axes);
    let body = certify(input.clone());
    for (p, expected) in [
        ([0., 0., 2.], Location::Inside),
        ([5.8, 0., 2.], Location::Inside),
        ([6.1, 0., 2.], Location::Outside),
        ([6., 0., 2.], Location::Unresolved),
        ([0., 0., -0.1], Location::Outside),
        ([0., 0., 0.], Location::Unresolved),
    ] {
        assert_eq!(body.classify(position(p), None).unwrap(), expected);
    }
    let support = body.support_bounds(axes[0], 1e-6, None).unwrap();
    assert!(support[0] <= 26. && support[1] >= 26. && support[1] - support[0] <= 1e-6);
    let text = multicad_rust_kernel::step::write_radial_boundary(&body, 1e-7).unwrap();
    let decoded = multicad_rust_kernel::step_read::read_radial_boundary(&text, 1e-7).unwrap();
    assert_eq!(decoded.input().vertices.len(), input.vertices.len());
    assert!(
        decoded
            .input()
            .vertices
            .iter()
            .all(|p| input.vertices.contains(p))
    );
    assert_eq!(decoded.input().surfaces.len(), input.surfaces.len());
    assert!(
        decoded
            .input()
            .surfaces
            .iter()
            .all(|s| input.surfaces.contains(s))
    );
    let restored = certify(decoded.input().clone());
    assert_eq!(
        restored.classify(position([5.8, 0., 2.]), None).unwrap(),
        Location::Inside
    );
    assert_eq!(
        multicad_rust_kernel::step::write_radial_boundary(&restored, 1e-7).unwrap(),
        text
    );
}

#[test]
fn independent_ridges_retain_distinct_longitudinal_knot_grids() {
    let input = wedge(&[1, 2]);
    let grids = input
        .surfaces
        .iter()
        .filter_map(|s| match s {
            Surface::Tensor(t) => Some(t.spans()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(grids, vec![[1, 1], [1, 1], [1, 1], [1, 2], [1, 2], [1, 2]]);
    let body = certify(input);
    for p in [[5.8, 0., 2.], [-5.8, 0., 2.], [0., 4.9, 2.]] {
        assert_eq!(body.classify(p, None).unwrap(), Location::Inside);
    }
    let text = multicad_rust_kernel::step::write_radial_boundary(&body, 1e-7).unwrap();
    let restored = certify(
        multicad_rust_kernel::step_read::read_radial_boundary(&text, 1e-7)
            .unwrap()
            .input()
            .clone(),
    );
    assert_eq!(
        multicad_rust_kernel::step::write_radial_boundary(&restored, 1e-7).unwrap(),
        text
    );
}

#[test]
fn retained_source_mass_and_surface_properties_match_independent_extruded_cam_integrals() {
    let alpha = (4_f64 / 3.).atan();
    let pi = std::f64::consts::PI;
    for grids in [&[1][..], &[1, 2][..]] {
        let count = grids.len() as f64;
        let area = 25. * pi + count * (30. - 25. * alpha);
        let first = if grids.len() == 1 { 106. / 3. } else { 0. };
        let xx = 156.25 * pi + count * (330. - 156.25 * alpha);
        let yy = 156.25 * pi + count * (167. - 156.25 * alpha);
        let lateral =
            4. * (count * (2. * 13_f64.sqrt() + 4.) + 5. * (2. * pi - 2. * count * alpha));
        let body = certify(wedge(grids));
        let before = body.input().clone();
        let result = body.properties_with_interrupt(1e-4, None).unwrap();
        let contains = |range: [f64; 2], expected: f64| {
            assert!(
                range[0] - 1e-10 <= expected && range[1] + 1e-10 >= expected,
                "{range:?} != {expected}"
            )
        };
        contains(result.volume_mm3, 4. * area);
        contains(result.surface_area_mm2, 2. * area + lateral);
        for (range, expected) in result.centroid_mm.into_iter().zip([first / area, 0., 2.]) {
            contains(range, expected);
        }
        let cxx = 4. * (xx - first * first / area);
        let cyy = 4. * yy;
        let czz = area * 64. / 12.;
        for (range, expected) in result.inertia_about_centroid_mm5.into_iter().zip([
            cyy + czz,
            cxx + czz,
            cxx + cyy,
            0.,
            0.,
            0.,
        ]) {
            contains(range, expected);
        }
        assert_eq!(result.faces.len(), body.input().faces.len());
        for (index, face) in result.faces.iter().enumerate() {
            assert_eq!(face.face_index, index);
        }
        for face in &result.faces[..2] {
            contains(face.area_mm2, area);
            contains(face.centroid_mm[0], first / area);
            contains(face.centroid_mm[1], 0.);
            assert!(face.planar_support.is_some());
        }
        let face = &result.faces[2];
        contains(face.area_mm2, 4. * 13_f64.sqrt());
        for (range, expected) in face.centroid_mm.into_iter().zip([4.5, -3., 2.]) {
            contains(range, expected);
        }
        assert!(face.planar_support.is_none());
        for (i, cone) in result.faces[2 + 3 * grids.len()..].iter().enumerate() {
            let gap = if grids.len() == 1 {
                2. * (pi - alpha)
            } else {
                pi - 2. * alpha
            };
            contains(cone.area_mm2, 20. * gap);
            let center = if grids.len() == 1 {
                [-8. / gap, 0., 2.]
            } else {
                [0., if i == 0 { 6. / gap } else { -6. / gap }, 2.]
            };
            for (r, e) in cone.centroid_mm.into_iter().zip(center) {
                contains(r, e);
            }
            assert!(cone.planar_support.is_none());
        }
        assert_eq!(body.input(), &before);
    }
}

#[test]
fn source_properties_stay_centered_under_proper_far_placement() {
    let origin = [1e6, -1e6, 1e6];
    let axes = [[0.8, 0.6, 0.], [-0.36, 0.48, 0.8], [0.48, -0.64, 0.6]];
    let input = pose(wedge(&[1]), origin, axes);
    let body = certify(input.clone());
    let placed = body.properties_with_interrupt(1e-4, None).unwrap();
    let local = certify(wedge(&[1]))
        .properties_with_interrupt(1e-5, None)
        .unwrap();
    let middle = |r: [f64; 2]| (r[0] + r[1]) * 0.5;
    let inside = |r: [f64; 2], v: f64| assert!(r[0] <= v && v <= r[1], "{r:?} not enclosing {v}");
    let center = local.centroid_mm.map(middle);
    for (i, range) in placed.centroid_mm.into_iter().enumerate() {
        inside(
            range,
            origin[i] + (0..3).map(|j| axes[j][i] * center[j]).sum::<f64>(),
        );
    }
    let local = local.inertia_about_centroid_mm5.map(middle);
    let matrix = [
        [local[0], local[3], local[4]],
        [local[3], local[1], local[5]],
        [local[4], local[5], local[2]],
    ];
    for ((a, b), range) in [(0, 0), (1, 1), (2, 2), (0, 1), (0, 2), (1, 2)]
        .into_iter()
        .zip(placed.inertia_about_centroid_mm5)
    {
        let mut expected = 0.;
        for i in 0..3 {
            for j in 0..3 {
                expected += axes[i][a] * matrix[i][j] * axes[j][b];
            }
        }
        inside(range, expected);
    }
    for (i, range) in placed.faces[2].centroid_mm.into_iter().enumerate() {
        inside(
            range,
            origin[i] + (0..3).map(|j| axes[j][i] * [4.5, -3., 2.][j]).sum::<f64>(),
        );
    }
    for f in &placed.faces {
        if let Surface::Plane(a) = input.surfaces[input.faces[f.face_index].surface] {
            assert_eq!(f.planar_support, Some(a));
        } else {
            assert_eq!(f.planar_support, None);
        }
    }
    assert_eq!(body.input(), &input);
}

#[test]
fn actual_taper_surface_mass_stays_bounded_without_an_inferred_recipe() {
    use multicad_rust_kernel::{helical_thread::Spec, step, step_read, tapered_thread::Thread};
    for (k, left, starts) in [(0., false, 1), (0.03125, true, 2), (-0.03125, false, 1)] {
        let source = Thread::new(
            Spec {
                major_radius_mm: 4.,
                pitch_mm: 1.25,
                length_mm: 8.,
                depth_mm: 0.5,
                crest_width_mm: 0.25,
                lower_flank_deg: 12.,
                upper_flank_deg: 18.,
                starts,
                left_hand: left,
                root_radius_mm: None,
            },
            k,
            1e-7,
            1e-6,
        )
        .unwrap();
        let text =
            step::write_tapered_thread(&source, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7).unwrap();
        let input = step_read::read_radial_boundary(&text, 1e-7)
            .unwrap()
            .input()
            .clone();
        let body = certify(input.clone());
        let p = body.properties_with_interrupt(1e-4, None).unwrap();
        let expected = if k == 0. {
            let exact = multicad_rust_kernel::helical_thread::Thread::new(source.spec())
                .unwrap()
                .volume_mm3();
            [exact, exact]
        } else {
            source.volume_bounds(0.002).unwrap()
        };
        assert!(
            p.volume_mm3[0] <= expected[1] && p.volume_mm3[1] >= expected[0],
            "{k}: {:?} vs {expected:?}",
            p.volume_mm3
        );
        assert_eq!(p.faces.len(), 4 * starts as usize + 2);
        assert!(p.faces.iter().all(|f| f.area_mm2[0] > 0.));
        let centroid = p.centroid_mm.map(|v| (v[0] + v[1]) * 0.5);
        assert_eq!(body.classify(centroid, None).unwrap(), Location::Inside);
        assert_eq!(body.input(), &input);
    }
}
