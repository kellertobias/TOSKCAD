use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    project_file,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    DrawingProjection, DrawingSolid, GeometryPolicy, MeshOptions, PlaneFrame, ProjectedVector,
    Solid, project_drawing,
};
use multicad_model::{Compiler, GeometryBackend};
use std::f64::consts::FRAC_PI_2;

fn toolcase() -> multicad_core::document::Document {
    project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document
}

fn same_projected_edge(actual: &ProjectedVector, expected: &ProjectedVector) -> bool {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let same_point = |a: [f64; 2], b: [f64; 2]| near(a[0], b[0]) && near(a[1], b[1]);
    let ends =
        |vector: &ProjectedVector| [vector.preview_mm[0], *vector.preview_mm.last().unwrap()];
    let [a, b] = ends(actual);
    let [c, d] = ends(expected);
    if !(same_point(a, c) && same_point(b, d) || same_point(a, d) && same_point(b, c)) {
        return false;
    }
    let quarter_arc = |curve: &VectorCurve| match curve {
        VectorCurve::Ellipse {
            center: [x, y],
            major_mm,
            minor_mm,
            start_rad,
            end_rad,
            ..
        } => Some([*x, *y, *major_mm, *minor_mm, end_rad - start_rad]),
        _ => None,
    };
    match (quarter_arc(&actual.curve), quarter_arc(&expected.curve)) {
        (Some(a), Some(b)) => a.into_iter().zip(b).all(|(a, b)| near(a, b)),
        (None, None) => matches!(
            (&actual.curve, &expected.curve),
            (VectorCurve::Line { .. }, VectorCurve::Line { .. })
        ),
        _ => false,
    }
}

#[test]
fn saved_toolcase_floor_profiles_rebuild_and_round_trip_with_analytic_quarter_arcs() {
    let document = toolcase();
    let features = [
        "015a44ba-56d1-477e-aaa0-c9016233401a",
        "16cc5d02-e3cc-472f-a2bd-9a1334340d67",
    ]
    .map(|id| EntityId::parse(id).unwrap());
    let compile = |backend| {
        Compiler::default()
            .compile_sources_with_backend(&document, 0, &features, backend, None)
            .unwrap_or_else(|cause| panic!("{backend:?}: {cause}"))
    };
    let rust = compile(GeometryBackend::Rust);
    let native = compile(GeometryBackend::OpenCascade);
    let dir = tempfile::tempdir().unwrap();
    let near = |actual: f64, expected: f64| {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    };
    for (index, feature) in features.into_iter().enumerate() {
        let actual = rust.solid(feature).unwrap();
        let reference = native.solid(feature).unwrap();
        assert!(actual.uses_rust_geometry());
        assert_eq!(actual.solid_count().unwrap(), 1);
        let expected_volume = reference.volume_mm3().unwrap();
        near(actual.volume_mm3().unwrap(), expected_volume);
        let (a, b) = (actual.bounds().unwrap(), reference.bounds().unwrap());
        for axis in 0..3 {
            near(a.min[axis], b.min[axis]);
            near(a.max[axis], b.max[axis]);
        }
        let topology = actual.topology_snapshot(feature, 0).unwrap();
        let arcs = topology
            .elements
            .iter()
            .filter_map(|element| {
                if let Some(AnalyticGeometry::CircularArc {
                    radius_mm,
                    sweep_rad,
                    ..
                }) = element.geometry
                {
                    assert_eq!(element.key.kind, TopologyKind::Edge);
                    Some((radius_mm, sweep_rad.abs()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(arcs.len(), 2);
        for (radius, sweep) in arcs {
            near(radius, [4., 2.][index]);
            near(sweep, FRAC_PI_2);
        }
        assert_eq!(actual.faces().unwrap().len(), 7);
        for sign in [-1., 1.] {
            let options = DrawingProjection {
                frame: PlaneFrame {
                    v: [0., 0., sign],
                    ..PlaneFrame::default()
                },
                hidden_lines: true,
                ..DrawingProjection::default()
            };
            let project = |solid, options| {
                project_drawing(&[DrawingSolid { solid, hatch: None }], options).unwrap()
            };
            let vectors = project(actual, options);
            let native_vectors = project(reference, options);
            for role in [VectorRole::Visible, VectorRole::Hidden] {
                let actual_layer: Vec<_> = vectors.iter().filter(|v| v.role == role).collect();
                let native_layer: Vec<_> =
                    native_vectors.iter().filter(|v| v.role == role).collect();
                assert_eq!([actual_layer.len(), native_layer.len()], [5, 5]);
                for vector in native_layer {
                    assert_eq!(
                        actual_layer
                            .iter()
                            .filter(|target| same_projected_edge(target, vector))
                            .count(),
                        1,
                        "{role:?} native curve {:?} must match one Rust edge",
                        vector.curve
                    );
                }
            }
            let visible_only = project(
                actual,
                DrawingProjection {
                    hidden_lines: false,
                    ..options
                },
            );
            assert_eq!(
                visible_only.iter().collect::<Vec<_>>(),
                vectors
                    .iter()
                    .filter(|v| v.role == VectorRole::Visible)
                    .collect::<Vec<_>>()
            );
            let ellipse = visible_only
                .iter()
                .find_map(|vector| match vector.curve {
                    VectorCurve::Ellipse {
                        major_mm,
                        minor_mm,
                        start_rad,
                        end_rad,
                        ..
                    } => Some((major_mm, minor_mm, end_rad - start_rad)),
                    _ => None,
                })
                .unwrap();
            near(ellipse.0, [4., 2.][index]);
            near(ellipse.1, ellipse.0);
            near(ellipse.2, FRAC_PI_2);
        }
        let receipt = actual
            .manufacturing_mesh_with_receipt(MeshOptions::default())
            .unwrap();
        assert!(receipt.edge_pairs > 0 && receipt.vertex_links > 0);
        let path = dir.path().join(format!("floor-{index}.step"));
        actual.write_step(&path).unwrap();
        let policy = GeometryPolicy::default();
        for readback in [
            Solid::read_step(&path, policy).unwrap(),
            Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), feature, policy)
                .unwrap(),
        ] {
            assert_eq!(readback.solid_count().unwrap(), 1);
            near(readback.volume_mm3().unwrap(), expected_volume);
        }
    }
}

#[test]
fn saved_toolcase_ribs_and_nut_pockets_rebuild_with_bounded_straight_sweeps() {
    let document = toolcase();
    let features = document
        .entities()
        .filter(|entity| {
            matches!(
                entity.name.as_str(),
                "Small drawer sliding rib extrusion" | "M4 captive nut pocket · 7.4 AF extrusion"
            )
        })
        .map(|entity| entity.id)
        .collect::<Vec<_>>();
    assert!(!features.is_empty());
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        Compiler::default()
            .compile_sources_with_backend(&document, 0, &features, backend, None)
            .unwrap_or_else(|cause| panic!("{backend:?}: {cause}"))
    });
    for feature in features {
        let a = rust.solid(feature).unwrap();
        let b = native.solid(feature).unwrap();
        assert_eq!(a.solid_count().unwrap(), 1);
        assert!((a.volume_mm3().unwrap() - b.volume_mm3().unwrap()).abs() < 1e-6);
        let (a, b) = (a.bounds().unwrap(), b.bounds().unwrap());
        for axis in 0..3 {
            assert!((a.min[axis] - b.min[axis]).abs() < 1e-6);
            assert!((a.max[axis] - b.max[axis]).abs() < 1e-6);
        }
    }
}
