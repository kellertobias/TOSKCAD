use super::*;

const MINIMUM: f64 = 1e-5;
const TOLERANCE: f64 = 1e-7;

fn spec(teeth: u32, pressure_deg: f64) -> Spec {
    let pressure = pressure_deg.to_radians();
    let pitch = f64::from(teeth);
    Spec {
        teeth,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * pressure.cos(),
        tip_radius_mm: pitch + 2.,
        root_radius_mm: pitch - 2.5,
        root_fillet_radius_mm: 0.4,
        half_base_angle_rad: PI / (2. * pitch) + pressure.tan() - pressure - 0.05 / (2. * pitch),
        face_width_mm: 8.,
        bore_radius_mm: 2.,
    }
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} within {tolerance}"
    );
}

fn point(edge: Edge, t: f64) -> Point {
    match edge {
        Edge::Planar(edge) => edge.point(t),
        Edge::Involute(edge) => edge.point(t),
    }
}

fn norm([x, y]: Point) -> f64 {
    x.hypot(y)
}

fn arc_tangent(edge: Edge, at_start: bool) -> Point {
    let Edge::Planar(normal_sweep::Edge::Arc {
        center,
        a,
        b,
        clockwise,
    }) = edge
    else {
        panic!("Expected circle");
    };
    let [x, y] = sub(if at_start { a } else { b }, center);
    scale([-y, x], if clockwise { -1. } else { 1. } / x.hypot(y))
}

#[test]
fn exact_profiles_cover_both_native_fillet_constructions() {
    for (spec, radial) in [
        (spec(20, 20.), true),
        (spec(64, 20.), false),
        (spec(128, 15.), false),
    ] {
        let profile = Profile::new(spec, MINIMUM, MAX_COORDINATE_MM).unwrap();
        let edges = profile.outline();
        let edges_per_tooth = if radial { 8 } else { 6 };
        assert_eq!(edges.len(), spec.teeth as usize * edges_per_tooth);
        assert_eq!(profile.height_mm(), spec.face_width_mm);
        assert_eq!(
            profile.bore(),
            Some(normal_sweep::Edge::Circle {
                center: [0.; 2],
                radius_mm: spec.bore_radius_mm,
                clockwise: true,
            })
        );
        for (tooth, sector) in edges.chunks_exact(edges_per_tooth).enumerate() {
            let mut roles = vec![
                ToothRole::LeftFillet,
                ToothRole::LeftFlank,
                ToothRole::Tip,
                ToothRole::RightFlank,
                ToothRole::RightFillet,
                ToothRole::Root,
            ];
            if radial {
                roles.insert(1, ToothRole::LeftRadial);
                roles.insert(5, ToothRole::RightRadial);
            }
            assert_eq!(sector.iter().map(|e| e.role).collect::<Vec<_>>(), roles);
            assert!(sector.iter().all(|e| e.tooth as usize == tooth));
            for (i, edge) in sector.iter().enumerate() {
                let next = edges[(tooth * edges_per_tooth + i + 1) % edges.len()];
                close(
                    norm(sub(point(edge.edge, 1.), point(next.edge, 0.))),
                    0.,
                    1e-12,
                );
            }
            let flank_index = if radial { 2 } else { 1 };
            let Edge::Involute(flank) = sector[flank_index].edge else {
                panic!("Expected analytic involute");
            };
            let Edge::Planar(fillet) = sector[0].edge else {
                panic!("Expected circular root fillet");
            };
            close(
                fillet.radius_mm().unwrap(),
                spec.root_fillet_radius_mm,
                1e-12,
            );
            close(norm(fillet.start()), spec.root_radius_mm, 1e-12);
            close(norm(flank.point(1.)), spec.tip_radius_mm, 1e-12);
            let [a, b] = flank.interval();
            let t = (spec.base_radius_mm / spec.pitch_radius_mm).acos().tan();
            let phase = TAU * tooth as f64 / f64::from(spec.teeth);
            let pitch_point = rotate(flank.point((t - a) / (b - a)), -phase);
            close(norm(pitch_point), spec.pitch_radius_mm, 1e-12);
            close(
                pitch_point[1].atan2(pitch_point[0]),
                -spec.half_base_angle_rad + t - t.atan(),
                1e-12,
            );
            close(
                norm(sub(arc_tangent(sector[0].edge, false), flank.tangent(0.))),
                0.,
                1e-12,
            );
            let root = sector[edges_per_tooth - 1].edge;
            close(
                norm(sub(
                    arc_tangent(root, false),
                    arc_tangent(
                        edges[(tooth * edges_per_tooth + edges_per_tooth) % edges.len()].edge,
                        true,
                    ),
                )),
                0.,
                1e-12,
            );
        }

        let mut sampled_area = Sum::default();
        for entry in edges {
            let count = 1024;
            for i in 0..count {
                let a = point(entry.edge, f64::from(i) / f64::from(count));
                let b = point(entry.edge, f64::from(i + 1) / f64::from(count));
                sampled_area.add(cross(a, b) / 2.);
            }
        }
        let expected_area = sampled_area.value() - PI * spec.bore_radius_mm.powi(2);
        close(profile.area_mm2(), expected_area, expected_area * 1e-7);
        close(
            profile.volume_mm3(),
            profile.area_mm2() * spec.face_width_mm,
            1e-12,
        );
    }
}

fn spline_point(spline: &CubicSpline, fraction: f64) -> Point {
    let knots: Vec<_> = spline
        .knots
        .iter()
        .copied()
        .zip(&spline.multiplicities)
        .flat_map(|(knot, &n)| std::iter::repeat_n(knot, n as usize))
        .collect();
    let last = spline.poles_mm.len() - 1;
    let k = if fraction == 1. {
        last
    } else {
        (3..=last)
            .find(|&k| knots[k] <= fraction && fraction < knots[k + 1])
            .unwrap()
    };
    let mut work: [Point; 4] = spline.poles_mm[k - 3..=k].try_into().unwrap();
    for level in 1..=3 {
        for i in (level..=3).rev() {
            let j = k - 3 + i;
            let f = (fraction - knots[j]) / (knots[j + 4 - level] - knots[j]);
            work[i] = add(scale(work[i - 1], 1. - f), scale(work[i], f));
        }
    }
    work[3]
}

#[test]
fn evaluated_composite_spline_stays_within_certified_error() {
    let flank = Involute::new(36., [0., 0.72], -0.12).unwrap();
    for curve in [
        flank,
        flank.reversed(),
        flank.reflected().rotated(1.5).unwrap(),
    ] {
        let spline = curve.cubic_spline(TOLERANCE).unwrap();
        let count = spline.knots.len() - 1;
        assert!((2..=127).contains(&count));
        assert_eq!(spline.poles_mm.len(), 2 * count + 2);
        assert_eq!(spline.multiplicities[0], 4);
        assert_eq!(spline.multiplicities[count], 4);
        assert!(spline.multiplicities[1..count].iter().all(|&n| n == 2));
        assert!(spline.error_bound_mm <= TOLERANCE);
        for i in 0..=1000 {
            let t = f64::from(i) / 1000.;
            assert!(norm(sub(curve.point(t), spline_point(&spline, t))) <= spline.error_bound_mm);
        }
    }
    let short = Involute::new(1., [0.2, 0.20001], 0.).unwrap();
    assert_eq!(short.cubic_spline(TOLERANCE).unwrap().poles_mm.len(), 4);
    close(flank.length_mm(), 36. * 0.72_f64.powi(2) / 2., 1e-12);
    close(
        flank.signed_area_mm2(),
        36_f64.powi(2) * 0.72_f64.powi(3) / 6.,
        1e-12,
    );
    close(
        flank.reflected().signed_area_mm2(),
        -flank.signed_area_mm2(),
        1e-12,
    );
    close(
        flank.reversed().signed_area_mm2(),
        -flank.signed_area_mm2(),
        1e-12,
    );
}

#[test]
fn rejects_invalid_or_unresolvable_geometry() {
    let nominal = spec(20, 20.);
    assert_eq!(
        Profile::new(
            Spec {
                teeth: 129,
                ..nominal
            },
            MINIMUM,
            MAX_COORDINATE_MM
        ),
        Err(Error::Limit)
    );
    for invalid in [
        Spec {
            base_radius_mm: f64::NAN,
            ..nominal
        },
        Spec {
            base_radius_mm: 21.,
            ..nominal
        },
        Spec {
            pitch_radius_mm: 15.,
            ..nominal
        },
        Spec {
            tip_radius_mm: 19.,
            ..nominal
        },
        Spec {
            root_fillet_radius_mm: 5.,
            ..nominal
        },
        Spec {
            half_base_angle_rad: 0.,
            ..nominal
        },
        Spec {
            bore_radius_mm: nominal.root_radius_mm,
            ..nominal
        },
        Spec {
            bore_radius_mm: MINIMUM / 2.,
            ..nominal
        },
    ] {
        assert_eq!(
            Profile::new(invalid, MINIMUM, MAX_COORDINATE_MM),
            Err(Error::InvalidDimensions)
        );
    }
    assert_eq!(
        Profile::new(nominal, 0., MAX_COORDINATE_MM),
        Err(Error::InvalidDimensions)
    );
    assert_eq!(
        Profile::new(nominal, MINIMUM, MINIMUM),
        Err(Error::InvalidDimensions)
    );
    for invalid in [
        Spec {
            half_base_angle_rad: 0.01,
            ..nominal
        },
        Spec {
            half_base_angle_rad: 0.155,
            ..nominal
        },
        Spec {
            base_radius_mm: (nominal.root_radius_mm
                * (nominal.root_radius_mm + 2. * nominal.root_fillet_radius_mm))
                .sqrt()
                + MINIMUM / 2.,
            ..nominal
        },
    ] {
        assert_eq!(
            Profile::new(invalid, MINIMUM, MAX_COORDINATE_MM),
            Err(Error::InvalidProfile)
        );
    }
    let unbored = Profile::new(
        Spec {
            bore_radius_mm: 0.,
            ..nominal
        },
        MINIMUM,
        MAX_COORDINATE_MM,
    )
    .unwrap();
    assert_eq!(unbored.bore(), None);

    assert_eq!(
        Involute::new(0., [0., 1.], 0.),
        Err(Error::InvalidDimensions)
    );
    assert_eq!(
        Involute::new(1., [0.5, 0.2], 0.),
        Err(Error::InvalidDimensions)
    );
    assert_eq!(
        Involute::new(1., [0., 1.], f64::NAN),
        Err(Error::InvalidDimensions)
    );
    let curve = Involute::new(40., [0., 1.], 0.).unwrap();
    assert_eq!(curve.rotated(f64::INFINITY), Err(Error::InvalidDimensions));
    assert_eq!(curve.cubic_spline(f64::NAN), Err(Error::InvalidDimensions));
    assert_eq!(curve.cubic_spline(0.), Err(Error::InvalidDimensions));
    assert_eq!(curve.cubic_spline(1e-13), Err(Error::Limit));
}

#[test]
fn sweep_mesh_preserves_bore_caps_face_ordinals_and_volume() {
    use crate::{display::Tessellation, manufacturing::certify};

    let options = Tessellation {
        deflection: 0.005,
        angle: 0.15,
        triangle_limit: 200_000,
    };
    let specimen = spec(20, 20.);
    // A second bore nearly touches the root circle, so the outer chords
    // need tighter sampling even though the requested tessellation is coarse.
    for radius in [0., 2., specimen.root_radius_mm - 1e-3] {
        let spec = Spec {
            bore_radius_mm: radius,
            ..specimen
        };
        let profile = Profile::new(spec, MINIMUM, MAX_COORDINATE_MM).unwrap();
        let mesh = profile.mesh(options).unwrap();
        let proof = certify(&mesh).unwrap();
        let lateral_area = profile.surface_area_mm2() - 2. * profile.area_mm2();
        close(
            proof.signed_volume_mm3,
            profile.volume_mm3(),
            lateral_area * options.deflection,
        );

        let total_faces = profile.outline().len() + 2 + usize::from(radius > 0.);
        let vertex = |i: u32| {
            let offset = i as usize * 3;
            let values = &mesh.vertices[offset..offset + 3];
            ([values[0], values[1]], values[2])
        };
        let mut by_face = vec![0; total_faces];
        for (indices, &face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            by_face[face as usize] += 1;
            let [(a, za), (b, zb), (c, zc)] = [indices[0], indices[1], indices[2]].map(vertex);
            if face < 2 {
                let z = f64::from(face) * spec.face_width_mm;
                assert_eq!([za, zb, zc], [z; 3]);
                assert_eq!(cross(sub(b, a), sub(c, a)).is_sign_positive(), face == 1);
            } else {
                let z = [za, zb, zc];
                assert!(z.contains(&0.) && z.contains(&spec.face_width_mm));
                let planar_edge = if za == zb { [a, b] } else { [c, b] };
                if radius > 0. && face as usize == total_faces - 1 {
                    for p in [a, b, c] {
                        close(norm(p), radius, 1e-12);
                    }
                    assert!(cross(planar_edge[0], planar_edge[1]) < 0.);
                }
            }
        }
        assert!(by_face.iter().all(|&n| n > 0));
        if radius == 0. {
            let reference_area = 2. * profile.area_mm2()
                + spec.face_width_mm
                    * profile
                        .outline()
                        .iter()
                        .map(|entry| entry.edge.length_mm())
                        .sum::<f64>();
            close(profile.surface_area_mm2(), reference_area, 1e-10);
        }
    }

    let direct = Profile::new(spec(64, 20.), MINIMUM, MAX_COORDINATE_MM).unwrap();
    assert!(certify(&direct.mesh(options).unwrap()).is_ok());
}

#[test]
fn support_and_arc_length_centroid_use_the_analytic_flank() {
    use crate::revolution::dot;

    let curve = Involute::new(21., [0.2, 0.8], 0.).unwrap();
    let direction = polar(1., 0.5 - FRAC_PI_2);
    let expected = dot(direction, curve.point(0.5));
    assert!(expected > dot(direction, curve.point(0.)));
    assert!(expected > dot(direction, curve.point(1.)));
    close(Edge::Involute(curve).support(direction), expected, 1e-12);

    for curve in [
        curve,
        curve.reversed().reflected().rotated(2.).unwrap(),
        Involute::new(20., [0.08, 0.08001], 0.2).unwrap(),
    ] {
        let edge = Edge::Involute(curve);
        let mut sum = [0.; 2];
        let mut weight = 0.;
        let [a, b] = curve.interval();
        let [u, v] = curve.basis();
        close(
            curve.base_radius_mm(),
            norm(curve.point(0.)) / a.hypot(1.),
            1e-12,
        );
        close(dot(u, v), 0., 1e-12);
        close(norm(u), 1., 1e-12);
        close(norm(v), 1., 1e-12);
        for i in 0..10_000 {
            let f = (f64::from(i) + 0.5) / 10_000.;
            let t = a + f * (b - a);
            sum = add(sum, scale(edge.point(f), t));
            weight += t;
        }
        let approximate_center = scale(sum, 1. / weight);
        close(norm(sub(edge.centroid(), approximate_center)), 0., 1e-7);
    }

    let profile = Profile::new(spec(20, 20.), MINIMUM, MAX_COORDINATE_MM).unwrap();
    let [min, max] = profile.bounds();
    assert_eq!(min[2], 0.);
    assert_eq!(max[2], profile.height_mm());
    // A tooth lies on each cardinal axis for a 20-tooth centered profile.
    for value in [min[0], min[1], max[0], max[1]] {
        close(value.abs(), 22., 1e-12);
    }
    let direction = [1.3, -0.9, 0.4];
    let support = profile.support(direction);
    let mut sampled = f64::NEG_INFINITY;
    for entry in profile.outline() {
        for i in 0..=1024 {
            let p = entry.edge.point(f64::from(i) / 1024.);
            sampled = sampled.max(dot([direction[0], direction[1]], p));
        }
    }
    close(support, sampled + direction[2] * profile.height_mm(), 1e-5);
    assert_eq!(profile.support([0.; 3]), 0.);
    assert!(profile.support([f64::NAN, 1., 0.]).is_nan());

    let line = Edge::Planar(normal_sweep::Edge::Line {
        a: [1., 1.],
        b: [1., 3.],
    });
    assert_eq!(line.centroid(), [1., 2.]);
    assert_eq!(line.tangent(0.5), [0., 1.]);
    let bore = Edge::Planar(profile.bore().unwrap());
    assert_eq!(bore.tangent(0.), [0., -1.]);
    let involute = Edge::Involute(curve);
    assert_eq!(involute.tangent(0.5), curve.tangent(0.5));
}

#[test]
fn detached_samples_respect_chord_and_angle_and_refuse_excess_work() {
    use crate::display::{MeshError, Tessellation};

    let profile = Profile::new(spec(20, 20.), MINIMUM, MAX_COORDINATE_MM).unwrap();
    let deflection = 0.003;
    let angular = 0.11;
    for entry in &profile.outline()[..8] {
        let edge = entry.edge;
        let sampled = edge.sample(deflection, angular).unwrap();
        assert_eq!(sampled.first(), Some(&edge.point(0.)));
        assert_eq!(sampled.last(), Some(&edge.point(1.)));
        let count = sampled.len() - 1;
        for (i, pair) in sampled.windows(2).enumerate() {
            let a = i as f64 / count as f64;
            let b = (i + 1) as f64 / count as f64;
            let d = edge.point(a.midpoint(b));
            let chord_mid = scale(add(pair[0], pair[1]), 0.5);
            assert!(norm(sub(d, chord_mid)) <= deflection + 1e-12);
            let [p, q] = [edge.tangent(a), edge.tangent(b)];
            assert!(cross(p, q).atan2(crate::revolution::dot(p, q)).abs() <= angular + 1e-12);
        }
    }
    let curve = Edge::Involute(Involute::new(20., [0., 0.7], 0.).unwrap());
    assert_eq!(curve.sample(0., angular), Err(MeshError::InvalidOptions));
    assert_eq!(curve.sample(1e-25, angular), Err(MeshError::BudgetExceeded));
    let arc = profile.outline()[0].edge;
    assert_eq!(arc.sample(1e-25, angular), Err(MeshError::BudgetExceeded));
    let options = Tessellation {
        deflection,
        angle: angular,
        triangle_limit: 1,
    };
    assert_eq!(profile.mesh(options), Err(MeshError::BudgetExceeded));
    assert_eq!(
        profile.mesh(Tessellation {
            triangle_limit: 100,
            ..options
        }),
        Err(MeshError::BudgetExceeded)
    );
    let unbored = Profile::new(
        Spec {
            bore_radius_mm: 0.,
            ..spec(20, 20.)
        },
        MINIMUM,
        MAX_COORDINATE_MM,
    )
    .unwrap();
    let mesh = unbored
        .mesh(Tessellation {
            triangle_limit: 20_000,
            ..options
        })
        .unwrap();
    assert_eq!(
        profile.mesh(Tessellation {
            triangle_limit: mesh.face_indices.len(),
            ..options
        }),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        profile.mesh(Tessellation {
            angle: f64::NAN,
            ..options
        }),
        Err(MeshError::InvalidOptions)
    );
}

#[test]
fn hub_extends_the_authored_band_with_one_continuous_bore() {
    use crate::{display::Tessellation, manufacturing::certify};

    let options = Tessellation {
        deflection: 0.003,
        angle: 0.15,
        triangle_limit: 200_000,
    };
    let nominal = spec(20, 20.);
    for (radius, axial_start_mm, axial_end_mm, bore_radius_mm) in [
        (10., -3., 11., 2.),
        (10., -3., 6., 0.),
        (10., 3., 11., 2.),
        (10., 0., 8., 2.),
        (nominal.root_radius_mm - 0.001, -2., 6., 2.),
        (2.001, -3., 11., 2.),
    ] {
        let source = Spec {
            bore_radius_mm,
            ..nominal
        };
        let base = Profile::new(source, MINIMUM, MAX_COORDINATE_MM).unwrap();
        let hub = Hub {
            radius_mm: radius,
            axial_start_mm,
            axial_end_mm,
        };
        let profile = base
            .clone()
            .with_hub(hub, MINIMUM, MAX_COORDINATE_MM)
            .unwrap();
        assert_eq!(profile.hub(), Some(hub));
        let low = axial_start_mm.min(0.);
        let high = axial_end_mm.max(nominal.face_width_mm);
        assert_eq!(profile.axial_bounds(), [low, high]);
        assert_eq!(
            profile.bounds(),
            [
                [base.bounds()[0][0], base.bounds()[0][1], low],
                [base.bounds()[1][0], base.bounds()[1][1], high],
            ]
        );
        assert_eq!(profile.height_mm(), nominal.face_width_mm);
        assert_eq!(profile.area_mm2(), base.area_mm2());
        let added_height = high - low - nominal.face_width_mm;
        close(
            profile.volume_mm3(),
            base.volume_mm3() + PI * (radius.powi(2) - bore_radius_mm.powi(2)) * added_height,
            1e-9,
        );
        close(
            profile.surface_area_mm2(),
            base.surface_area_mm2() + TAU * (radius + bore_radius_mm) * added_height,
            1e-9,
        );
        let direction = [0.3, -0.4, 12.];
        close(
            profile.support(direction),
            base.support(direction).max(radius * 0.5 + high * 12.),
            1e-11,
        );
        let direction = [0.3, -0.4, -12.];
        close(
            profile.support(direction),
            base.support(direction).max(radius * 0.5 - low * 12.),
            1e-11,
        );

        let mesh = profile.mesh(options).unwrap();
        let proof = certify(&mesh).unwrap();
        close(
            proof.signed_volume_mm3,
            profile.volume_mm3(),
            profile.surface_area_mm2() * options.deflection,
        );
        if added_height == 0. {
            assert_eq!(mesh, base.mesh(options).unwrap());
            assert_eq!(profile.hub_extensions(), [None; 2]);
            continue;
        }

        let bore_face = profile.outline().len() as u32 + 2;
        let extensions = profile.hub_extensions();
        let total_faces = bore_face as usize
            + usize::from(bore_radius_mm > 0.)
            + 2 * extensions.iter().flatten().count();
        let mut seen = vec![false; total_faces];
        let mut next_extension_face = bore_face + u32::from(bore_radius_mm > 0.);
        for (i, ext) in extensions.into_iter().enumerate() {
            let cap_z = [0., nominal.face_width_mm][i];
            let outer_z = [low, high][i];
            if let Some(ext) = ext {
                assert_eq!(ext.radius_mm, radius);
                assert_eq!(ext.gear_z_mm, cap_z);
                assert_eq!(ext.end_z_mm, outer_z);
                assert_eq!(ext.wall_face, next_extension_face);
                assert_eq!(ext.cap_face, next_extension_face + 1);
                next_extension_face += 2;
            } else {
                assert_eq!(cap_z, outer_z);
            }
        }

        for (indices, &face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            seen[face as usize] = true;
            let points = indices.map(|i| {
                let i = 3 * i as usize;
                [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]]
            });
            if face == bore_face && bore_radius_mm > 0. {
                for [x, y, z] in points {
                    close(x.hypot(y), bore_radius_mm, 1e-12);
                    assert!(z == low || z == high);
                }
            }
            for ext in extensions.iter().flatten() {
                if face == ext.wall_face {
                    for [x, y, z] in points {
                        close(x.hypot(y), radius, 1e-12);
                        assert!(z == ext.gear_z_mm || z == ext.end_z_mm);
                    }
                }
                if face == ext.cap_face {
                    assert!(points.iter().all(|p| p[2] == ext.end_z_mm));
                }
            }
        }
        assert!(seen.iter().all(|&found| found));
    }
}

#[test]
fn hub_input_and_tessellation_budget_are_explicit() {
    use crate::display::{MeshError, Tessellation};

    let base = Profile::new(spec(20, 20.), MINIMUM, MAX_COORDINATE_MM).unwrap();
    let hub = Hub {
        radius_mm: 10.,
        axial_start_mm: -3.,
        axial_end_mm: 11.,
    };
    for bad in [
        Hub {
            radius_mm: f64::NAN,
            ..hub
        },
        Hub {
            axial_start_mm: f64::NEG_INFINITY,
            ..hub
        },
        Hub {
            axial_start_mm: -MAX_COORDINATE_MM,
            ..hub
        },
        Hub {
            axial_end_mm: hub.axial_start_mm,
            ..hub
        },
    ] {
        assert_eq!(
            base.clone().with_hub(bad, MINIMUM, MAX_COORDINATE_MM),
            Err(Error::InvalidDimensions)
        );
    }
    for bad in [
        Hub {
            radius_mm: 17.5,
            ..hub
        },
        Hub {
            radius_mm: 2.,
            ..hub
        },
        Hub {
            axial_start_mm: 9.,
            axial_end_mm: 11.,
            ..hub
        },
        Hub {
            axial_start_mm: -MINIMUM / 2.,
            ..hub
        },
    ] {
        assert_eq!(
            base.clone().with_hub(bad, MINIMUM, MAX_COORDINATE_MM),
            Err(Error::InvalidProfile)
        );
    }
    assert_eq!(
        base.clone().with_hub(hub, 0., MAX_COORDINATE_MM),
        Err(Error::InvalidDimensions)
    );

    let profile = base
        .clone()
        .with_hub(hub, MINIMUM, MAX_COORDINATE_MM)
        .unwrap();
    let options = Tessellation {
        deflection: 0.01,
        angle: 0.15,
        triangle_limit: 50_000,
    };
    let base_triangles = base.mesh(options).unwrap().face_indices.len();
    assert_eq!(
        profile.mesh(Tessellation {
            triangle_limit: base_triangles,
            ..options
        }),
        Err(MeshError::BudgetExceeded)
    );
}
