use multicad_core::{document::EntityId, manufacturing::PlanarEdge, references::TopologyKind};
use multicad_kernel::{GeometryPolicy, PlaneFrame, Solid, StraightRack};
use std::f64::consts::{FRAC_PI_2, PI};

fn specimen(teeth: u32, module: f64) -> StraightRack {
    StraightRack {
        teeth,
        pitch_mm: PI * module,
        pitch_thickness_mm: (PI / 2. - 0.07) * module,
        pressure_angle_rad: 20_f64.to_radians(),
        addendum_mm: module,
        dedendum_mm: 1.25 * module,
        root_fillet_radius_mm: 0.3 * module,
        backing_thickness_mm: 2. * module,
        face_width_mm: 6.,
    }
}
fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} (budget {tolerance})"
    );
}
fn analytical_volume(s: StraightRack) -> f64 {
    // Sharp trapezoid plus two concave tangent-circle corner additions.
    // Corner addition = tangent quadrilateral area minus circular sector area.
    let alpha = s.pressure_angle_rad;
    let corner = s.root_fillet_radius_mm.powi(2)
        * (1. / alpha.cos() - alpha.tan() - (FRAC_PI_2 - alpha) / 2.);
    f64::from(s.teeth)
        * s.face_width_mm
        * (s.pitch_mm * s.backing_thickness_mm
            + (s.addendum_mm + s.dedendum_mm)
                * (s.pitch_thickness_mm + (s.dedendum_mm - s.addendum_mm) * alpha.tan())
            + 2. * corner)
}
fn verify(solid: &Solid, s: StraightRack) {
    let policy = GeometryPolicy::default();
    let tolerance = 16. * policy.linear_tolerance_mm;
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    let half = f64::from(s.teeth) * s.pitch_mm / 2.;
    for (actual, expected) in
        bounds
            .min
            .into_iter()
            .zip([-half, -s.dedendum_mm - s.backing_thickness_mm, 0.])
    {
        close(actual, expected, tolerance);
    }
    for (actual, expected) in bounds
        .max
        .into_iter()
        .zip([half, s.addendum_mm, s.face_width_mm])
    {
        close(actual, expected, tolerance);
    }
    let faces = solid.faces().unwrap();
    assert_eq!(faces.len(), s.teeth as usize * 6 + 6);
    let area: f64 = faces.iter().map(|f| f.area_mm2).sum();
    let volume = solid.volume_mm3().unwrap();
    // Existing native linear policy and unchanged adaptive GK relative budget.
    close(
        volume,
        analytical_volume(s),
        policy.linear_tolerance_mm * area + 1e-7 * volume.abs(),
    );
    let cap = faces
        .iter()
        .find(|f| {
            f.plane
                .is_some_and(|(p, n)| p[2].abs() < tolerance && n[2].abs() > 0.99)
        })
        .unwrap();
    let contours = solid
        .planar_face_contours(cap.index, PlaneFrame::default())
        .unwrap();
    assert_eq!(contours.loops.len(), 1);
    let edges = &contours.loops[0].edges;
    assert_eq!(edges.len(), s.teeth as usize * 6 + 4);
    let flanks: Vec<_> = edges
        .iter()
        .filter_map(|e| match *e {
            PlanarEdge::Line { a, b }
                if (a[1] - b[1]).abs() > tolerance
                    && (a[1].max(b[1]) - s.addendum_mm).abs() < tolerance =>
            {
                Some((a, b))
            }
            _ => None,
        })
        .collect();
    assert_eq!(flanks.len(), 2 * s.teeth as usize);
    for &(a, b) in &flanks {
        close(
            ((b[0] - a[0]) / (b[1] - a[1])).abs().atan(),
            s.pressure_angle_rad,
            tolerance,
        );
    }
    for y in [-0.4 * s.dedendum_mm, 0., 0.5 * s.addendum_mm] {
        let mut measured: Vec<_> = flanks
            .iter()
            .map(|&(a, b)| {
                assert!(y > a[1].min(b[1]) && y < a[1].max(b[1]));
                a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
            })
            .collect();
        measured.sort_by(f64::total_cmp);
        for tooth in 0..s.teeth as usize {
            let center = (tooth as f64 + 0.5 - f64::from(s.teeth) / 2.) * s.pitch_mm;
            let half_width = s.pitch_thickness_mm / 2. - y * s.pressure_angle_rad.tan();
            close(measured[2 * tooth], center - half_width, tolerance);
            close(measured[2 * tooth + 1], center + half_width, tolerance);
        }
    }
    let mut arcs = 0;
    for edge in edges {
        if let PlanarEdge::Arc {
            a,
            b,
            center,
            sweep_rad,
        } = *edge
        {
            arcs += 1;
            close(
                center[1],
                -s.dedendum_mm + s.root_fillet_radius_mm,
                tolerance,
            );
            close(sweep_rad.abs(), FRAC_PI_2 - s.pressure_angle_rad, tolerance);
            for point in [a, b] {
                let radius = [point[0] - center[0], point[1] - center[1]];
                close(
                    radius[0].hypot(radius[1]),
                    s.root_fillet_radius_mm,
                    tolerance,
                );
                let (u, v) = edges
                    .iter()
                    .find_map(|e| match *e {
                        PlanarEdge::Line { a, b }
                            if [a, b]
                                .iter()
                                .any(|q| (q[0] - point[0]).hypot(q[1] - point[1]) < tolerance) =>
                        {
                            Some((a, b))
                        }
                        _ => None,
                    })
                    .expect("Each circle endpoint joins a native straight edge");
                let direction = [v[0] - u[0], v[1] - u[1]];
                close(
                    (radius[0] * direction[0] + radius[1] * direction[1])
                        / direction[0].hypot(direction[1]),
                    0.,
                    tolerance,
                );
            }
        }
    }
    assert_eq!(arcs, 2 * s.teeth);
}
#[test]
fn exact_rack_flanks_root_circles_pitch_and_step_survive_native_readback() {
    for spec in [specimen(8, 1.), specimen(4, 2.)] {
        let feature = EntityId::new();
        let solid = Solid::straight_rack_named(spec, feature, GeometryPolicy::default()).unwrap();
        verify(&solid, spec);
        let topology = solid.topology_snapshot(feature, 0).unwrap();
        assert!(
            topology
                .elements
                .iter()
                .all(|e| !e.origins.is_empty() && e.origins.iter().all(|o| o.source == feature))
        );
        for tooth in 0..spec.teeth {
            for role in [
                "flank/left",
                "flank/right",
                "fillet/left",
                "fillet/right",
                "tip",
            ] {
                let role = format!("rack/tooth/{tooth}/{role}");
                assert!(
                    topology
                        .elements
                        .iter()
                        .filter(|e| e.key.kind == TopologyKind::Face)
                        .any(|e| e.origins.iter().any(|o| o.role == role)),
                    "Missing {role}"
                );
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("straight-rack.step");
        solid.write_step(&path).unwrap();
        let reopened = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        verify(&reopened, spec);
        close(
            solid.volume_mm3().unwrap(),
            reopened.volume_mm3().unwrap(),
            1e-6,
        );
    }
}
#[test]
fn whole_pitch_cell_limit_retains_finite_connected_backing() {
    let spec = specimen(64, 1.);
    let solid =
        Solid::straight_rack_named(spec, EntityId::new(), GeometryPolicy::default()).unwrap();
    verify(&solid, spec);
}
#[test]
fn invalid_nonfinite_or_consumed_rack_sections_are_refused() {
    let base = specimen(8, 1.);
    let mut invalid = vec![];
    for teeth in [0, 3, 65, 128, u32::MAX] {
        invalid.push(StraightRack { teeth, ..base });
    }
    for pressure_angle_rad in [0., -0.1, FRAC_PI_2, f64::NAN, f64::INFINITY] {
        invalid.push(StraightRack {
            pressure_angle_rad,
            ..base
        });
    }
    for value in [0., -1., f64::NAN, f64::INFINITY] {
        invalid.extend([
            StraightRack {
                pitch_mm: value,
                ..base
            },
            StraightRack {
                pitch_thickness_mm: value,
                ..base
            },
            StraightRack {
                addendum_mm: value,
                ..base
            },
            StraightRack {
                dedendum_mm: value,
                ..base
            },
            StraightRack {
                root_fillet_radius_mm: value,
                ..base
            },
            StraightRack {
                backing_thickness_mm: value,
                ..base
            },
            StraightRack {
                face_width_mm: value,
                ..base
            },
        ]);
    }
    invalid.extend([
        StraightRack {
            pitch_thickness_mm: 0.1,
            ..base
        },
        StraightRack {
            root_fillet_radius_mm: 2.,
            ..base
        },
        StraightRack {
            addendum_mm: 10.,
            ..base
        },
        StraightRack {
            pitch_mm: 1.,
            ..base
        },
        StraightRack {
            pitch_mm: 10.,
            dedendum_mm: 0.1,
            root_fillet_radius_mm: 0.8,
            ..base
        },
        StraightRack {
            backing_thickness_mm: 1e8,
            ..base
        },
    ]);
    for spec in invalid {
        assert!(
            Solid::straight_rack_named(spec, EntityId::new(), GeometryPolicy::default()).is_err(),
            "Accepted {spec:?}"
        );
    }
    assert!(Solid::straight_rack_named(base, EntityId::nil(), GeometryPolicy::default()).is_err());
}
