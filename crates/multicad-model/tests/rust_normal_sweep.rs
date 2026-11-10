use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    project_file,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    DrawingProjection, DrawingSolid, GeometryPolicy, MeshOptions, PlaneFrame, Solid,
    project_drawing,
};
use multicad_model::{Compiler, GeometryBackend};
use std::f64::consts::FRAC_PI_2;

#[test]
fn saved_toolcase_floor_profiles_rebuild_and_round_trip_with_analytic_quarter_arcs() {
    let mut bytes = include_bytes!("../../../examples/toolcase.tcd").as_slice();
    let document = project_file::read(
        &mut bytes,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
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
            let vectors = project_drawing(
                &[DrawingSolid {
                    solid: actual,
                    hatch: None,
                }],
                options,
            )
            .unwrap();
            assert_eq!(vectors.len(), 5);
            assert!(
                vectors
                    .iter()
                    .all(|vector| vector.role == VectorRole::Visible)
            );
            let ellipse = vectors
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
