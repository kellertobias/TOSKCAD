//! OCCT reference observations for the initial Rust kernel comparison slice.
use multicad_core::manufacturing::PlanarEdge;
use multicad_kernel::{
    BooleanOperation, GeometryErrorKind, GeometryPolicy, PlaneFrame, Profile, ProfileEdge,
    RigidTransform, Solid, source_identity,
};
use serde_json::{Value, json};
use std::{error::Error, f64::consts::PI, time::Instant};

fn near(label: &str, actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: expected {expected}, got {actual} (tolerance {tolerance})"
    );
}

fn round_panel(height: f64, radius: f64, quarter_turn: bool) -> Result<Value, Box<dyn Error>> {
    let policy = GeometryPolicy::default();
    let profile = Profile {
        frame: PlaneFrame::default(),
        outer: vec![
            ProfileEdge::Line {
                a: [0., 0.],
                b: [40., 0.],
            },
            ProfileEdge::Arc {
                center: [40., 10.],
                a: [40., 0.],
                b: [40., 20.],
                clockwise: false,
            },
            ProfileEdge::Line {
                a: [40., 20.],
                b: [0., 20.],
            },
            ProfileEdge::Line {
                a: [0., 20.],
                b: [0., 0.],
            },
        ],
        holes: vec![vec![ProfileEdge::Circle {
            center: [12., 10.],
            radius_mm: 3.,
        }]],
    };
    let started = Instant::now();
    let panel = Solid::extrude_profile(&profile, height, policy)?;
    let extrusion_ns = started.elapsed().as_nanos();

    let mut cutter_frame = PlaneFrame {
        origin: [28., 10., -1.],
        ..Default::default()
    };
    if quarter_turn {
        cutter_frame.u = [0., 1., 0.];
        cutter_frame.v = [-1., 0., 0.];
    }
    let started = Instant::now();
    let cutter = Solid::cylinder(cutter_frame, radius, height + 2., policy)?;
    let cutter_ns = started.elapsed().as_nanos();
    let started = Instant::now();
    let cut = panel
        .boolean(&cutter, BooleanOperation::Subtract, policy)?
        .ok_or("Through cut removed the entire panel")?;
    let subtraction_ns = started.elapsed().as_nanos();

    let started = Instant::now();
    let before = panel.volume_mm3()?;
    let after = cut.volume_mm3()?;
    let original_area = 40. * 20. + PI * (10_f64.powi(2) / 2. - 3_f64.powi(2));
    let bore_area = PI * radius.powi(2);
    let expected_area = original_area - bore_area;
    near("original volume", before, original_area * height, 1e-6);
    near("removed material", before - after, bore_area * height, 1e-6);
    assert_eq!(cut.solid_count()?, 1);

    let faces = cut.faces()?;
    let top = faces
        .iter()
        .find(|face| {
            face.plane.is_some_and(|(point, normal)| {
                (point[2] - height).abs() < 1e-7 && normal[2] > 0.999999
            })
        })
        .ok_or("Missing outward top cap after cut")?;
    let contours = cut.planar_face_contours(
        top.index,
        PlaneFrame {
            origin: [0., 0., height],
            ..Default::default()
        },
    )?;
    near("top cap area", contours.area_mm2(), expected_area, 1e-6);
    assert_eq!(contours.loops.len(), 3);
    assert_eq!(contours.loops.iter().filter(|wire| wire.outer).count(), 1);
    let mut holes = Vec::new();
    for wire in &contours.loops {
        let arcs = wire.edges.iter().filter_map(|edge| match edge {
            PlanarEdge::Arc {
                a,
                center,
                sweep_rad,
                ..
            } => Some((
                *center,
                (a[0] - center[0]).hypot(a[1] - center[1]),
                sweep_rad.abs(),
            )),
            PlanarEdge::Line { .. } => None,
        });
        if wire.outer {
            let [(center, measured_radius, sweep)] = arcs
                .collect::<Vec<_>>()
                .try_into()
                .map_err(|_| "The outer profile must retain one analytic semicircle")?;
            near("outer arc center X", center[0], 40., 1e-7);
            near("outer arc center Y", center[1], 10., 1e-7);
            near("outer arc radius", measured_radius, 10., 1e-7);
            near("outer arc sweep", sweep, PI, 1e-7);
        } else {
            let [(center, measured_radius, sweep)] = arcs
                .collect::<Vec<_>>()
                .try_into()
                .map_err(|_| "A hole must retain one analytic circle")?;
            near("hole sweep", sweep, 2. * PI, 1e-7);
            holes.push((center, measured_radius));
        }
    }
    holes.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    assert_eq!(holes.len(), 2);
    for ((center, measured_radius), (expected_x, expected_radius)) in
        holes.iter().zip([(12., 3.), (28., radius)])
    {
        near("hole center X", center[0], expected_x, 1e-7);
        near("hole center Y", center[1], 10., 1e-7);
        near("hole radius", *measured_radius, expected_radius, 1e-7);
    }
    let validation_ns = started.elapsed().as_nanos();
    Ok(json!({
        "height_mm": height, "cutter_radius_mm": radius, "cutter_seam_quarter_turn": quarter_turn,
        "volume_before_mm3": before, "volume_after_mm3": after,
        "expected_volume_after_mm3": expected_area * height,
        "top_area_mm2": contours.area_mm2(), "holes_center_and_radius_mm": holes,
        "faces": faces.len(),
        "timings_ns": {"extrusion": extrusion_ns, "cutter": cutter_ns,
                       "subtraction": subtraction_ns, "independent_checks": validation_ns},
    }))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut round_cases = Vec::new();
    for (height, radius) in [(8., 4.), (10. / 3., 4.25)] {
        for quarter_turn in [false, true] {
            round_cases.push(round_panel(height, radius, quarter_turn)?);
        }
    }
    let policy = GeometryPolicy::default();
    let source = Solid::box_solid([10.; 3], policy)?;
    let mut contacts = Vec::new();
    for (name, offset, expected) in [
        ("gap", 10.01, None),
        ("contact", 10., None),
        ("overlap", 9.99, Some(1.)),
    ] {
        let other = source.transformed(RigidTransform {
            translation_mm: [offset, 0., 0.],
            ..Default::default()
        })?;
        let started = Instant::now();
        let result = source.boolean(&other, BooleanOperation::Intersect, policy)?;
        let intersection_ns = started.elapsed().as_nanos();
        let actual = result.as_ref().map(Solid::volume_mm3).transpose()?;
        match (actual, expected) {
            (Some(actual), Some(expected)) => near(name, actual, expected, 1e-7),
            (None, None) => (),
            _ => {
                return Err(format!("Unexpected solid intersection for {name}: {actual:?}").into());
            }
        }
        contacts.push(json!({"case":name,"offset_mm":offset,"volume_mm3":actual,"intersection_ns":intersection_ns}));
    }
    assert!(matches!(
        Solid::cylinder(PlaneFrame::default(), 0., 10., policy),
        Err(error) if error.kind == GeometryErrorKind::Input
    ));
    println!(
        "{}",
        json!({
            "schema": "multicad.kernel-slice-baseline/1", "occt_source": source_identity(),
            "target": [std::env::consts::OS, std::env::consts::ARCH],
            "linear_tolerance_mm": policy.linear_tolerance_mm, "boolean_fuzzy_mm": policy.boolean_fuzzy_mm,
            "round_panel": round_cases, "contact": contacts, "zero_radius_rejected": true,
        })
    );
    Ok(())
}
