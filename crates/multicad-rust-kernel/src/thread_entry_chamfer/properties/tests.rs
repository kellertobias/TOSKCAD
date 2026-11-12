use super::*;
#[test]
fn source_properties_have_positive_faces_and_enclose_independent_volume() {
    let body = Chamfer::new(super::super::tests::tool(false, 1, false), 0.3, None).unwrap();
    let p = body
        .properties(PropertiesOptions {
            relative_tolerance: 1e-5,
            ..Default::default()
        })
        .unwrap();
    let v = body
        .volume_mm3(QueryOptions {
            tolerance: 1e-7,
            ..Default::default()
        })
        .unwrap();
    assert!(p.volume_mm3.lower <= v.upper && v.lower <= p.volume_mm3.upper);
    assert_eq!(p.faces.len(), body.face_count() as usize);
    for f in p.faces {
        assert!(f.area_mm2.lower > 0.);
    }
}
fn includes(v: Enclosure, x: f64, slack: f64) -> bool {
    v.lower - slack <= x && x <= v.upper + slack
}
fn close(a: Enclosure, b: Enclosure, sign: f64) {
    assert!(
        a.lower <= sign * b.lower.max(b.upper) + 1e-5
            && a.upper >= sign * b.lower.min(b.upper) - 1e-5
    );
}
const GAUSS: [(f64, f64); 4] = [
    (0.06943184420297371, 0.17392742256872693),
    (0.33000947820757187, 0.32607257743127305),
    (0.6699905217924281, 0.32607257743127305),
    (0.9305681557970262, 0.17392742256872693),
];
/// Independent fixed-order polar-volume and cross-product surface quadrature.
/// This numerical reference is only a test oracle; published bounds come from
/// the separate full derivative certificates.
fn reference(body: &Chamfer) -> ([f64; 11], Vec<[f64; 4]>) {
    let mut sum = [0.; 11];
    let mut faces = vec![[0.; 4]; body.face_count() as usize];
    let spec = body.tool.spec();
    let h = body.tool.helical_rate();
    for start in 0..spec.starts {
        for segment in body.phase_segments().unwrap() {
            let ends = segment.ends.map(|k| k.nominal);
            let dq = (ends[1] - ends[0]) / 16.;
            for cell in 0..16 {
                for (a, wa) in GAUSS {
                    let q = ends[0] + (cell as f64 + a) * dq;
                    let r = body.tool.radius_at_phase(q);
                    let rp = body
                        .profile(segment.band, I::point(q))
                        .unwrap()
                        .first
                        .lo
                        .midpoint(body.profile(segment.band, I::point(q)).unwrap().first.hi);
                    let g = body.seam_station(q);
                    let theta0 = -(q + f64::from(start) * spec.pitch_mm) / h;
                    let r0 = r.min(body.entry_radius_mm());
                    let neck = body.terminal.map_or(0., |n| n.neck_radius_mm());
                    for (index, z, outer, inner) in
                        [(0, 0., r0, 0.), (1, body.material_length_mm(), r, neck)]
                    {
                        let angle = theta0 + z / h;
                        let area = (outer * outer - inner * inner) / (2. * h.abs()) * dq * wa;
                        let radial = (outer.powi(3) - inner.powi(3)) / (3. * h.abs()) * dq * wa;
                        let values = [area, radial * angle.cos(), radial * angle.sin(), area * z];
                        sum[10] += area;
                        for (i, value) in values.into_iter().enumerate() {
                            faces[index][i] += value;
                        }
                    }
                    for (cone, lo, hi) in [(true, 0., g), (false, g, body.material_length_mm())] {
                        let dz = (hi - lo) / 32.;
                        for cell in 0..32 {
                            for (b, wb) in GAUSS {
                                let z = lo + (cell as f64 + b) * dz;
                                let theta = theta0 + z / h;
                                let (s, c) = theta.sin_cos();
                                let radius = if cone { body.cone_radius(z) } else { r };
                                let w = dq * dz * wa * wb / h.abs();
                                let v = radius * radius / 2. * w;
                                for (i, value) in [
                                    v,
                                    radius.powi(3) / 3. * c * w,
                                    radius.powi(3) / 3. * s * w,
                                    z * v,
                                    radius.powi(4) / 4. * c * c * w,
                                    radius.powi(4) / 4. * s * s * w,
                                    z * z * v,
                                    radius.powi(4) / 4. * c * s * w,
                                    z * radius.powi(3) / 3. * c * w,
                                    z * radius.powi(3) / 3. * s * w,
                                ]
                                .into_iter()
                                .enumerate()
                                {
                                    sum[i] += value;
                                }
                                let (u, v) = if cone {
                                    (
                                        [radius * s / h, -radius * c / h, 0.],
                                        [
                                            body.cone_slope().unwrap().lo * c - radius * s / h,
                                            body.cone_slope().unwrap().lo * s + radius * c / h,
                                            1.,
                                        ],
                                    )
                                } else {
                                    (
                                        [rp * c + r * s / h, rp * s - r * c / h, 0.],
                                        [-r * s / h, r * c / h, 1.],
                                    )
                                };
                                let cross = [
                                    u[1] * v[2] - u[2] * v[1],
                                    u[2] * v[0] - u[0] * v[2],
                                    u[0] * v[1] - u[1] * v[0],
                                ];
                                let area =
                                    cross[0].hypot(cross[1]).hypot(cross[2]) * dq * dz * wa * wb;
                                sum[10] += area;
                                let index = if cone {
                                    body.cone_face_index(start, segment.half)
                                } else {
                                    body.wall_face_index(start, segment.band)
                                }
                                .unwrap() as usize;
                                for (i, value) in
                                    [area, area * radius * c, area * radius * s, area * z]
                                        .into_iter()
                                        .enumerate()
                                {
                                    faces[index][i] += value;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(tail) = body.terminal {
        let r = tail.neck_radius_mm();
        let lo = tail.material_length_mm();
        let hi = tail.full_length_mm();
        let v = std::f64::consts::PI * r * r * (hi - lo);
        sum[0] += v;
        sum[3] += v * (lo + hi) / 2.;
        sum[4] += v * r * r / 4.;
        sum[5] += v * r * r / 4.;
        sum[6] += v * (lo * lo + lo * hi + hi * hi) / 3.;
        let wall = std::f64::consts::PI * 2. * r * (hi - lo);
        let disk = std::f64::consts::PI * r * r;
        faces[(2 + 6 * spec.starts) as usize] = [wall, 0., 0., wall * (lo + hi) / 2.];
        faces[(3 + 6 * spec.starts) as usize] = [disk, 0., 0., disk * hi];
        sum[10] += wall + disk;
    }
    (sum, faces)
}
#[test]
fn independent_polar_and_surface_controls_cover_hands_starts_roots_and_terminal_neck() {
    for (left, starts, rounded, depth, terminal) in [
        (false, 1, false, 0.3, false),
        (true, 1, true, 0.3, true),
        (false, 2, true, 0.8, false),
        (true, 3, false, 1.2, true),
    ] {
        let tool = super::super::tests::tool(left, starts, rounded);
        let body = Chamfer::new(
            tool,
            depth,
            terminal.then_some(TerminalIntent {
                width_mm: 0.6,
                radial_depth_mm: 0.1,
            }),
        )
        .unwrap();
        let p = body.properties(PropertiesOptions::default()).unwrap();
        let (expected, face_expected) = reference(&body);
        let actual = [
            p.volume_mm3,
            p.first_moments_mm4[0],
            p.first_moments_mm4[1],
            p.first_moments_mm4[2],
            p.second_moments_mm5[0][0],
            p.second_moments_mm5[1][1],
            p.second_moments_mm5[2][2],
            p.second_moments_mm5[0][1],
            p.second_moments_mm5[0][2],
            p.second_moments_mm5[1][2],
        ];
        for (i, v) in actual.into_iter().enumerate() {
            assert!(
                includes(v, expected[i], 2e-6),
                "{left}/{starts}/{rounded} moment{i}: {v:?} vs {}",
                expected[i]
            );
        }
        for (face, expected) in p.faces.iter().zip(face_expected) {
            let values = [
                face.area_mm2,
                face.first_moments_mm3[0],
                face.first_moments_mm3[1],
                face.first_moments_mm3[2],
            ];
            for (i, value) in values.into_iter().enumerate() {
                assert!(
                    includes(value, expected[i], 2e-6),
                    "face {:?} quantity{i}: {value:?} vs{}",
                    face.face,
                    expected[i]
                );
            }
        }
        let raw = [
            [expected[4], expected[7], expected[8]],
            [expected[7], expected[5], expected[9]],
            [expected[8], expected[9], expected[6]],
        ];
        let centered: [[f64; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| raw[i][j] - expected[i + 1] * expected[j + 1] / expected[0])
        });
        for i in 0..3 {
            for j in 0..3 {
                let target = if i == j {
                    centered[(i + 1) % 3][(i + 1) % 3] + centered[(i + 2) % 3][(i + 2) % 3]
                } else {
                    -centered[i][j]
                };
                assert!(includes(p.centroidal_inertia_mm5[i][j], target, 1e-4));
            }
        }
        let area = p.faces.iter().fold(I::point(0.), |a, f| {
            a + I::new(f.area_mm2.lower, f.area_mm2.upper)
        });
        assert!(
            includes(enclosure(area), expected[10], 2e-6),
            "area {area:?} vs {}",
            expected[10]
        );
        for start in 0..starts {
            let crest = &p.faces[(2 + 4 * start + 3) as usize];
            let breaks = tool.profile_breaks();
            let width = breaks[1] + tool.spec().pitch_mm - breaks[4];
            let expected = tool.spec().major_radius_mm / tool.helical_rate().abs()
                * width
                * (body.material_length_mm() - body.entry_length_mm());
            assert!(includes(crest.area_mm2, expected, 1e-10));
            assert!(includes(
                crest.centroid_mm[2],
                (body.material_length_mm() + body.entry_length_mm()) / 2.,
                1e-10
            ));
        }
        if starts >= 3 {
            for i in 0..2 {
                assert!(includes(p.centroid_mm[i], 0., 1e-10));
            }
            assert!(includes(p.second_moments_mm5[0][1], 0., 1e-10));
        }
        let end = p.faces[1].area_mm2;
        let neck = body
            .terminal
            .map_or(0., |n| std::f64::consts::PI * n.neck_radius_mm().powi(2));
        assert!(includes(
            end,
            tool.volume_mm3() / tool.spec().length_mm - neck,
            1e-8
        ));
    }
}
#[test]
fn handedness_reflects_material_moments_and_typed_failures_leave_source_unchanged() {
    let a = Chamfer::new(super::super::tests::tool(false, 1, true), 0.8, None).unwrap();
    let b = Chamfer::new(super::super::tests::tool(true, 1, true), 0.8, None).unwrap();
    let p = a.properties(PropertiesOptions::default()).unwrap();
    let q = b.properties(PropertiesOptions::default()).unwrap();
    for i in 0..3 {
        close(
            p.centroid_mm[i],
            q.centroid_mm[i],
            if i == 1 { -1. } else { 1. },
        );
    }
    for bad in [f64::NAN, 0., 1., -1.] {
        assert_eq!(
            a.properties(PropertiesOptions {
                relative_tolerance: bad,
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid
        );
    }
    for limit in [0, 1, 1_000_001] {
        assert_eq!(
            a.properties(PropertiesOptions {
                maximum_cells: limit,
                ..Default::default()
            })
            .unwrap_err(),
            Error::Limit
        );
    }
    assert_eq!(
        a.properties_with_interrupt(PropertiesOptions::default(), Some(&|| true))
            .unwrap_err(),
        Error::Cancelled
    );
    assert_eq!(
        a.source_thread(),
        &super::super::tests::tool(false, 1, true)
    );
    assert!(
        a.properties(PropertiesOptions {
            relative_tolerance: 1e-5,
            ..Default::default()
        })
        .is_ok()
    );
}
#[test]
fn dimensional_scaling_and_multistart_symmetry_preserve_mass_and_surface_units() {
    let source = super::super::tests::tool(false, 8, true);
    let base = Chamfer::new(
        source,
        0.3,
        Some(TerminalIntent {
            width_mm: 0.6,
            radial_depth_mm: 0.1,
        }),
    )
    .unwrap();
    let p = base
        .properties(PropertiesOptions {
            relative_tolerance: 1e-5,
            ..Default::default()
        })
        .unwrap();
    for factor in [0.5, 32.] {
        let mut spec = source.spec();
        spec.major_radius_mm *= factor;
        spec.pitch_mm *= factor;
        spec.length_mm *= factor;
        spec.depth_mm *= factor;
        spec.crest_width_mm *= factor;
        spec.root_radius_mm = spec.root_radius_mm.map(|r| r * factor);
        let body = Chamfer::new(
            Thread::new(spec).unwrap(),
            0.3 * factor,
            Some(TerminalIntent {
                width_mm: 0.6 * factor,
                radial_depth_mm: 0.1 * factor,
            }),
        )
        .unwrap();
        let q = body
            .properties(PropertiesOptions {
                relative_tolerance: 1e-5,
                ..Default::default()
            })
            .unwrap();
        let overlap = |a: Enclosure, b: Enclosure, n: i32| {
            a.lower * factor.powi(n) <= b.upper && b.lower <= a.upper * factor.powi(n)
        };
        assert!(overlap(p.volume_mm3, q.volume_mm3, 3));
        for (i, j) in [(0, 0), (1, 1), (2, 2)] {
            assert!(overlap(
                p.second_moments_mm5[i][j],
                q.second_moments_mm5[i][j],
                5
            ));
        }
        for i in 0..3 {
            assert!(overlap(p.first_moments_mm4[i], q.first_moments_mm4[i], 4));
        }
        for (a, b) in p.faces.iter().zip(q.faces) {
            assert_eq!(a.face, b.face);
            assert!(overlap(a.area_mm2, b.area_mm2, 2));
            for i in 0..3 {
                assert!(overlap(a.first_moments_mm3[i], b.first_moments_mm3[i], 3));
            }
        }
    }
}
#[test]
fn entry_mesh_roles_agree_with_source_phase_and_retained_supports_for_every_hand() {
    use crate::display::Tessellation;
    for (left, starts, rounded, factor) in [
        (false, 1, false, 0.1),
        (true, 1, true, 1.2),
        (false, 2, true, 0.9),
        (true, 2, true, 0.9),
    ] {
        let source = super::super::tests::tool(left, starts, rounded);
        let body = Chamfer::new(source, source.spec().depth_mm * factor, None).unwrap();
        assert_eq!(body.wall_face_index(starts, 0), None);
        assert_eq!(body.wall_face_index(0, 5), None);
        assert_eq!(body.cone_face_index(starts, ConeHalf::Rising), None);
        assert_eq!(body.cone_face_index(u32::MAX, ConeHalf::Falling), None);
        assert_eq!(body.wall_face_index(u32::MAX, usize::MAX), None);
        let options = Tessellation {
            deflection: 0.03,
            angle: 0.3,
            triangle_limit: 5_000_000,
        };
        let mesh = tessellate(&body, options).unwrap();
        let breaks = body.tool.profile_breaks();
        let pitch = body.tool.spec().pitch_mm;
        let period = pitch * f64::from(starts);
        let mut checked = vec![0usize; body.face_count() as usize];
        for (triangle, &face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            let p: [f64; 3] = std::array::from_fn(|axis| {
                triangle
                    .iter()
                    .map(|&i| mesh.vertices[3 * i as usize + axis] / 3.)
                    .sum()
            });
            let theta = p[1].atan2(p[0]);
            let global = (p[2] - body.tool.helical_rate() * theta).rem_euclid(period);
            let start = (global / pitch).floor() as u32;
            let phase = global - f64::from(start) * pitch;
            if breaks.iter().any(|&q| (q - phase).abs() < 0.01 * pitch) {
                continue;
            }
            match body.face(face).unwrap() {
                Face::ThreadBand { .. } => {
                    let band = breaks
                        .windows(2)
                        .position(|q| q[0] < phase && phase < q[1])
                        .unwrap();
                    assert_eq!(
                        Some(face),
                        body.wall_face_index(start, band),
                        "left{left} starts{starts} phase{phase}"
                    );
                    checked[face as usize] += 1;
                }
                Face::ConeHalf { .. } => {
                    let valley = body
                        .tool
                        .root_circle()
                        .map_or(breaks[2].midpoint(breaks[3]), |r| r.phase);
                    if (phase - valley).abs() < 0.01 * pitch {
                        continue;
                    }
                    let half = if phase >= valley {
                        ConeHalf::Rising
                    } else {
                        ConeHalf::Falling
                    };
                    assert_eq!(Some(face), body.cone_face_index(start, half));
                    checked[face as usize] += 1;
                }
                _ => {}
            }
        }
        for (i, &count) in checked.iter().enumerate().skip(2) {
            assert!(
                count > 0,
                "unchecked face{i} left{left} starts{starts} rounded{rounded}"
            );
        }
    }
}
#[test]
fn positive_denominator_guards_and_late_cancellation_are_atomic() {
    for v in [I::point(0.), I::new(-1., 1.), I::point(f64::INFINITY)] {
        assert_eq!(positive(v), Err(Error::Unresolved));
    }
    assert_eq!(positive(I::point(1.)).unwrap(), I::point(1.));
    let body = Chamfer::new(super::super::tests::tool(false, 1, false), 0.5, None).unwrap();
    let counter = std::cell::Cell::new(0usize);
    assert_eq!(
        body.properties_with_interrupt(
            PropertiesOptions::default(),
            Some(&|| {
                counter.set(counter.get() + 1);
                counter.get() > 20
            })
        )
        .unwrap_err(),
        Error::Cancelled
    );
    assert!(counter.get() > 20);
    assert!(
        body.properties(PropertiesOptions {
            relative_tolerance: 1e-5,
            ..Default::default()
        })
        .is_ok()
    );
}
#[test]
fn short_wide_material_uses_area_units_independently_of_its_volume_scale() {
    let mut spec = super::super::tests::tool(false, 1, false).spec();
    spec.major_radius_mm = 1000.;
    spec.pitch_mm *= 0.001;
    spec.length_mm *= 0.001;
    spec.depth_mm *= 0.001;
    spec.crest_width_mm *= 0.001;
    let body = Chamfer::new(Thread::new(spec).unwrap(), 0.001, None).unwrap();
    let result = body
        .properties(PropertiesOptions {
            relative_tolerance: 1e-9,
            ..Default::default()
        })
        .unwrap();
    assert!(result.faces[0].area_mm2.lower > 3_000_000.);
    assert!(result.faces[1].area_mm2.lower > 3_000_000.);
}
#[test]
fn derived_property_widths_are_certified_after_division_and_centering() {
    let mut spec = super::super::tests::tool(false, 1, false).spec();
    spec.major_radius_mm = 1000.;
    let body = Chamfer::new(Thread::new(spec).unwrap(), 0.001, None).unwrap();
    let mut exercised = false;
    for tolerance in [1e-5, 1e-7] {
        let options = PropertiesOptions {
            relative_tolerance: tolerance,
            ..Default::default()
        };
        let mut work = Work::new(QueryOptions::default(), None).unwrap();
        let raw = body.integrated_properties(tolerance, &mut work).unwrap();
        let ratio = raw.quality(&body, tolerance).unwrap();
        if ratio > 1. {
            let consumed = QueryOptions::default().maximum_cells - work.remaining + 1;
            assert_eq!(
                body.properties(PropertiesOptions {
                    maximum_cells: consumed,
                    ..options
                })
                .unwrap_err(),
                Error::Limit
            );
        }
        exercised |= ratio > 1.;
        let checked = body.properties(options).unwrap();
        assert!(checked.quality(&body, tolerance).unwrap() <= 1.);
        let lengths = [1000., 1000., 6.6];
        for face in &checked.faces {
            for (i, l) in lengths.into_iter().enumerate() {
                assert!(face.centroid_mm[i].upper - face.centroid_mm[i].lower <= tolerance * l);
            }
        }
        for (i, l) in lengths.into_iter().enumerate() {
            assert!(checked.centroid_mm[i].upper - checked.centroid_mm[i].lower <= tolerance * l);
            for j in 0..3 {
                let square = if i == j {
                    lengths[(i + 1) % 3].powi(2) + lengths[(i + 2) % 3].powi(2)
                } else {
                    l * lengths[j]
                };
                let value = checked.centroidal_inertia_mm5[i][j];
                assert!(value.upper - value.lower <= tolerance * 1000. * 1000. * 6.6 * square);
            }
        }
    }
    assert!(exercised);
    assert_eq!(
        body.properties(PropertiesOptions {
            relative_tolerance: 1e-9,
            ..Default::default()
        })
        .unwrap_err(),
        Error::Unresolved
    );
    for (current, ratio) in [
        (f64::from_bits(1), 2.),
        (1., f64::INFINITY),
        (1., f64::NAN),
        (1., 0.1),
    ] {
        assert_eq!(tightened_tolerance(current, ratio), Err(Error::Unresolved));
    }
    assert_eq!(tightened_tolerance(1., 2.).unwrap(), 0.125);
    assert_eq!(
        centroid([I::point(1.); 3], I::point(0.)),
        Err(Error::Unresolved)
    );
}
