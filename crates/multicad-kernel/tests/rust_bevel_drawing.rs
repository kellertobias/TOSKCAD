//! Exact axial Rust bevel drawings against native OpenCascade projections.
use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
};
use multicad_kernel::{
    DrawingHatch, DrawingProjection, DrawingSolid, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    ProjectedVector, RigidTransform, Solid, SphericalBevelGear, project_drawing,
};
use std::f64::consts::PI;

fn specification(module: f64, teeth: u32, mate: u32, width: f64, bore: f64) -> SphericalBevelGear {
    let (z, mate) = (f64::from(teeth), f64::from(mate));
    let alpha = 20_f64.to_radians();
    let delta = 1_f64.atan2(mate / z);
    let distance = module * z / (2. * delta.sin());
    let beta = (delta.sin() * alpha.cos()).asin();
    let inv = |g: f64| {
        let sigma = (g.cos() / beta.cos()).acos();
        sigma / beta.sin() - sigma.sin().atan2(beta.sin() * sigma.cos())
    };
    SphericalBevelGear {
        teeth,
        pitch_cone_angle_rad: delta,
        base_cone_angle_rad: beta,
        tip_cone_angle_rad: delta + (module / distance).atan(),
        root_cone_angle_rad: delta - (1.25 * module / distance).atan(),
        outer_cone_distance_mm: distance,
        face_width_mm: width,
        bore_radius_mm: bore,
        half_base_tooth_angle_rad: (PI * module / 2. - 0.05) / (module * z) + inv(delta),
        root_fillet_radius_mm: module * 0.2,
    }
}

const TRANSFORM: RigidTransform = RigidTransform {
    rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
    translation_mm: [10., -20., 30.],
};

/// View frame looking along the placed gear axis from either side.
fn axial_frame(heel_side: bool) -> PlaneFrame {
    // Local axis (0,0,1) maps to the third rotation column.
    let axis = [0.6, 0., 0.8];
    let toward = if heel_side { axis } else { axis.map(|v| -v) };
    let u = [0., 1., 0.];
    let v = [
        toward[1] * u[2] - toward[2] * u[1],
        toward[2] * u[0] - toward[0] * u[2],
        toward[0] * u[1] - toward[1] * u[0],
    ];
    PlaneFrame {
        origin: [1., 2., 3.],
        u,
        v,
    }
}

fn length(vector: &ProjectedVector) -> f64 {
    vector
        .preview_mm
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

fn totals(vectors: &[ProjectedVector]) -> [f64; 3] {
    let mut sum = [0.; 3];
    for vector in vectors {
        let index = match vector.role {
            VectorRole::Visible => 0,
            VectorRole::Hidden => 1,
            VectorRole::Section => 2,
            _ => continue,
        };
        sum[index] += length(vector);
    }
    sum
}

fn project(solid: &Solid, options: DrawingProjection) -> Vec<ProjectedVector> {
    project_drawing(&[DrawingSolid { solid, hatch: None }], options).unwrap()
}

/// Largest distance from any preview point of `from` to the polylines of
/// `onto`; zero when `from` is empty.
fn excess(from: &[&ProjectedVector], onto: &[&ProjectedVector]) -> f64 {
    let segments: Vec<[[f64; 2]; 2]> = onto
        .iter()
        .flat_map(|v| v.preview_mm.windows(2).map(|w| [w[0], w[1]]))
        .collect();
    let distance = |p: [f64; 2]| {
        segments
            .iter()
            .map(|&[a, b]| {
                let d = [b[0] - a[0], b[1] - a[1]];
                let length = d[0] * d[0] + d[1] * d[1];
                let t = if length > 0. {
                    (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length).clamp(0., 1.)
                } else {
                    0.
                };
                (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
            })
            .fold(f64::INFINITY, f64::min)
    };
    from.iter()
        .flat_map(|v| v.preview_mm.iter().copied())
        .map(distance)
        .fold(0., f64::max)
}

fn roles<'a>(vectors: &'a [ProjectedVector], roles: &[VectorRole]) -> Vec<&'a ProjectedVector> {
    vectors.iter().filter(|v| roles.contains(&v.role)).collect()
}

/// Two-way coverage between Rust and native views: each Rust curve lies on
/// a native curve of the same role, and each native curve lies on a Rust
/// curve of that role or, for duplicated outlines, of a covering role.
fn same_drawing(rust: &[ProjectedVector], native: &[ProjectedVector]) {
    use VectorRole::{Hidden, Section, Visible};
    let tolerance = 0.03;
    for (role, cover) in [
        (Visible, vec![Visible, Section]),
        (Hidden, vec![Hidden, Visible, Section]),
        (Section, vec![Section]),
    ] {
        let mine = roles(rust, &[role]);
        let theirs = roles(native, &[role]);
        assert!(
            theirs.is_empty() <= mine.is_empty() || role == Hidden,
            "{role:?}"
        );
        let forward = excess(&mine, &theirs);
        let backward = excess(&theirs, &roles(rust, &cover));
        assert!(
            forward <= tolerance && backward <= tolerance,
            "{role:?} {forward} {backward}"
        );
    }
}

fn pair(spec: SphericalBevelGear) -> (Solid, Option<Solid>) {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let rust = Solid::rust_spherical_bevel_gear_named(spec, feature, policy)
        .unwrap()
        .transformed(TRANSFORM)
        .unwrap();
    let native = multicad_kernel::OCCT_LINKED.then(|| {
        Solid::spherical_bevel_gear_named(spec, feature, policy)
            .unwrap()
            .transformed(TRANSFORM)
            .unwrap()
    });
    (rust, native)
}

fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs().max(1.),
        "{actual} != {expected}"
    );
}

/// Sheet depth of a local axial height for a view from either side.
fn depth(frame: PlaneFrame, height: f64, heel_side: bool) -> f64 {
    let toward = [
        frame.u[1] * frame.v[2] - frame.u[2] * frame.v[1],
        frame.u[2] * frame.v[0] - frame.u[0] * frame.v[2],
        frame.u[0] * frame.v[1] - frame.u[1] * frame.v[0],
    ];
    let offset: f64 = (0..3)
        .map(|i| (TRANSFORM.translation_mm[i] - frame.origin[i]) * toward[i])
        .sum();
    offset + if heel_side { height } else { -height }
}

#[test]
fn axial_bevel_views_match_native_visible_and_hidden_lengths() {
    for (spec, bore) in [
        (specification(2., 32, 24, 6., 4.), 4.),
        (specification(1., 24, 64, 2., 0.), 0.),
    ] {
        let (rust, native) = pair(spec);
        for heel_side in [true, false] {
            for hidden_lines in [false, true] {
                let options = DrawingProjection {
                    frame: axial_frame(heel_side),
                    hidden_lines,
                    ..Default::default()
                };
                let actual = project(&rust, options);
                let [visible, hidden, _] = totals(&actual);
                assert!(visible > 0.);
                // The heel cap hides everything behind it; from the apex side
                // the only rear edge is the bore rim under the toe rim.
                assert_eq!(hidden > 0., heel_side && hidden_lines);
                // Every visible flank is an exact cubic B-spline.
                assert!(
                    actual
                        .iter()
                        .any(|v| matches!(v.curve, VectorCurve::BSpline { .. }))
                );
                let Some(native) = &native else {
                    continue;
                };
                let reference = project(native, options);
                let [native_visible, native_hidden, _] = totals(&reference);
                println!(
                    "{{\"heel_side\":{heel_side},\"hidden\":{hidden_lines},\"rust\":[{visible},{hidden}],\"native\":[{native_visible},{native_hidden}]}}"
                );
                // Edge-on meridian planes add coincident native outlines,
                // so only the gear without meridians compares lengths.
                if bore > 0. {
                    close(visible, native_visible, 1e-3);
                } else {
                    assert!(visible <= native_visible * (1. + 1e-3));
                }
                assert!(hidden <= native_hidden + 1e-9);
                // OpenCascade repeats hidden edges under coincident visible
                // ones, such as the rear bore rim; the Rust view omits them.
                same_drawing(&actual, &reference);
            }
        }
    }
}

#[test]
fn bevel_sections_and_retained_views_match_native_cuts() {
    let spec = specification(2., 32, 24, 6., 4.);
    let (rust, native) = pair(spec);
    let hatch = DrawingHatch {
        pattern: multicad_core::materials::HatchPattern::Lines,
        angle_rad: 0.7,
        spacing_mm: 1.5,
        line_width_mm: 0.1,
    };
    // Planes through the root band, the tooth tops and near the toe apex.
    let middle = spec.outer_cone_distance_mm - spec.face_width_mm / 2.;
    let heights = [
        middle * spec.pitch_cone_angle_rad.cos(),
        (spec.outer_cone_distance_mm - spec.face_width_mm) * spec.root_cone_angle_rad.cos() * 1.01,
    ];
    for heel_side in [true, false] {
        let frame = axial_frame(heel_side);
        for height in heights {
            let offset = depth(frame, height, heel_side);
            for (section_only, limit) in [(true, None), (false, None), (false, Some(1.5))] {
                let options = DrawingProjection {
                    frame,
                    section_offset_mm: Some(offset),
                    section_only,
                    section_depth_mm: limit,
                    hidden_lines: true,
                    ..Default::default()
                };
                let actual = project(&rust, options);
                let [visible, hidden, section] = totals(&actual);
                assert!(section > 0.);
                let hatched = project_drawing(
                    &[DrawingSolid {
                        solid: &rust,
                        hatch: Some(hatch),
                    }],
                    options,
                )
                .unwrap();
                assert!(hatched.iter().any(|v| v.role == VectorRole::Hatch));
                let Some(native) = &native else {
                    continue;
                };
                let reference = project(native, options);
                let [native_visible, native_hidden, native_section] = totals(&reference);
                println!(
                    "{{\"heel_side\":{heel_side},\"height\":{height},\"only\":{section_only},\"depth\":{limit:?},\"rust\":[{visible},{hidden},{section}],\"native\":[{native_visible},{native_hidden},{native_section}]}}"
                );
                close(section, native_section, 1e-3);
                // OpenCascade also lists the cut outline as visible edges.
                close(
                    visible + if section_only { 0. } else { section },
                    native_visible,
                    1e-3,
                );
                // Native views also repeat the rear bore rim under the
                // section bore rim from the apex side.
                assert!(hidden <= native_hidden * (1. + 1e-3));
                same_drawing(&actual, &reference);
            }
        }
    }
}

#[test]
fn oblique_cropped_and_mixed_bevel_drawings_are_refused() {
    let spec = specification(2., 32, 24, 6., 4.);
    let (rust, _) = pair(spec);
    let refused =
        |solids: &[DrawingSolid<'_>], options| project_drawing(solids, options).unwrap_err().kind;
    let one = [DrawingSolid {
        solid: &rust,
        hatch: None,
    }];
    assert_eq!(
        refused(&one, DrawingProjection::default()),
        GeometryErrorKind::Unsupported
    );
    for options in [
        DrawingProjection {
            frame: axial_frame(true),
            detail: Some(([0., 0.], 5.)),
            ..Default::default()
        },
        DrawingProjection {
            frame: axial_frame(true),
            section_offset_mm: Some(depth(axial_frame(true), 40., true)),
            section_u_mm: Some([-5., 5.]),
            ..Default::default()
        },
    ] {
        assert_eq!(refused(&one, options), GeometryErrorKind::Unsupported);
    }
    let two = [
        DrawingSolid {
            solid: &rust,
            hatch: None,
        },
        DrawingSolid {
            solid: &rust,
            hatch: None,
        },
    ];
    assert_eq!(
        refused(
            &two,
            DrawingProjection {
                frame: axial_frame(true),
                ..Default::default()
            }
        ),
        GeometryErrorKind::Unsupported
    );
    // Budgets are enforced on the exact curves.
    let options = DrawingProjection {
        frame: axial_frame(true),
        maximum_points: 50,
        ..Default::default()
    };
    assert_eq!(refused(&one, options), GeometryErrorKind::Limit);
    let options = DrawingProjection {
        frame: axial_frame(true),
        maximum_curves: 5,
        ..Default::default()
    };
    assert_eq!(refused(&one, options), GeometryErrorKind::Limit);
}

/// Median wall time in milliseconds of five runs.
fn median_ms(mut run: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..5)
        .map(|_| {
            let started = std::time::Instant::now();
            run();
            started.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[2]
}

#[test]
fn matched_bevel_build_step_and_drawing_timings() {
    let policy = GeometryPolicy::default();
    let spec = specification(2., 32, 24, 6., 4.);
    let directory = tempfile::tempdir().unwrap();
    let options = DrawingProjection {
        frame: axial_frame(true),
        hidden_lines: true,
        ..Default::default()
    };
    let mut stages = vec![];
    let rust_build = median_ms(|| {
        Solid::rust_spherical_bevel_gear_named(spec, EntityId::new(), policy).unwrap();
    });
    let (rust, native) = pair(spec);
    let rust_step = median_ms(|| rust.write_step(&directory.path().join("r.step")).unwrap());
    let rust_drawing = median_ms(|| {
        project(&rust, options);
    });
    stages.push(("rust", [rust_build, rust_step, rust_drawing]));
    if let Some(native) = native {
        let native_build = median_ms(|| {
            Solid::spherical_bevel_gear_named(spec, EntityId::new(), policy).unwrap();
        });
        let native_step =
            median_ms(|| native.write_step(&directory.path().join("n.step")).unwrap());
        let native_drawing = median_ms(|| {
            project(&native, options);
        });
        stages.push(("native", [native_build, native_step, native_drawing]));
    }
    for (backend, [build, step, drawing]) in stages {
        println!(
            "{{\"backend\":\"{backend}\",\"profile\":\"{}\",\"build_ms\":{build},\"step_ms\":{step},\"axial_drawing_ms\":{drawing}}}",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
        );
    }
}
