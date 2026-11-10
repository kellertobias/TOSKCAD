use multicad_core::{document::EntityId, project_file, references::AnalyticGeometry};
use multicad_kernel::{PlaneFrame, Solid};
use multicad_model::{Compiler, GeometryBackend};

#[test]
fn saved_toolcase_handle_joins_tangent_posts_into_one_manufacturable_solid() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let feature: EntityId =
        serde_json::from_str(r#""1d27a5a5-e8e2-4834-9868-8bfcdd4020ba""#).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_sources_with_backend(&document, 0, &[feature], backend, None)
            .unwrap()
    });
    let [solid, reference] = [&rust, &native].map(|model| model.solid(feature).unwrap());
    assert!(solid.uses_rust_geometry());
    assert_eq!(solid.solid_count().unwrap(), 1);
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    near(solid.volume_mm3().unwrap(), reference.volume_mm3().unwrap());
    let [rust_faces, native_faces] = [solid, reference].map(|body| body.faces().unwrap());
    let area = |faces: &[multicad_kernel::Face]| faces.iter().map(|face| face.area_mm2).sum();
    near(area(&rust_faces), area(&native_faces));
    let [bounds, reference] = [solid, reference].map(|body| body.bounds().unwrap());
    for i in 0..3 {
        near(bounds.min[i], reference.min[i]);
        near(bounds.max[i], reference.max[i]);
    }
    assert!(
        solid
            .topology_snapshot(feature, 0)
            .unwrap()
            .elements
            .iter()
            .any(|element| matches!(element.geometry, Some(AnalyticGeometry::CircularArc { .. })))
    );
    let mut transitions = 0;
    for face in rust_faces {
        let Some((origin, normal)) = face.plane else {
            continue;
        };
        if normal[0].abs() != 1. || origin[0] == bounds.min[0] || origin[0] == bounds.max[0] {
            continue;
        }
        let frame = PlaneFrame {
            origin,
            u: [0., 1., 0.],
            v: [0., 0., normal[0]],
        };
        let contours = solid.planar_face_contours(face.index, frame).unwrap();
        assert_eq!(contours.loops.len(), 1);
        near(contours.loops[0].signed_area(), face.area_mm2);
        transitions += 1;
    }
    assert_eq!(transitions, 12);
    assert!(
        !solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap()
            .mesh
            .indices
            .is_empty()
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("carry-handle.step");
    solid.write_step(&path).unwrap();
    let imported = Solid::read_step(&path, compiler.policy).unwrap();
    assert_eq!(imported.solid_count().unwrap(), 1);
    near(imported.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
    near(area(&imported.faces().unwrap()), area(&native_faces));
    let imported_bounds = imported.bounds().unwrap();
    for i in 0..3 {
        near(imported_bounds.min[i], bounds.min[i]);
        near(imported_bounds.max[i], bounds.max[i]);
    }
}
