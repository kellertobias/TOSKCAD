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
    let mut input = wedge(&[1]);
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
