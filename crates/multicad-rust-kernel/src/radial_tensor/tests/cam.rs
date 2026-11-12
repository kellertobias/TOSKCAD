use super::*;
fn line(a: Point, b: Point) -> Curve {
    Curve::Spline {
        spans: 1,
        poles: (0..=9)
            .map(|i| std::array::from_fn(|k| a[k] + (b[k] - a[k]) * i as f64 / 9.))
            .collect(),
    }
}
pub(crate) fn wedge(longitudinal: &[usize]) -> BoundaryInput {
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
