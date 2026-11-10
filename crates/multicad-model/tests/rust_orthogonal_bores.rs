use multicad_core::{
    document::{Document, Entity},
    model, project_file,
};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend};

fn compare_saved_cuts(
    mut project: &[u8],
    select: impl Fn(&Document, &Entity) -> bool,
    expected_count: usize,
) {
    let document = project_file::read(
        &mut project,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let features = document
        .entities()
        .filter(|entity| select(&document, entity))
        .map(|entity| entity.id)
        .collect::<Vec<_>>();
    assert_eq!(features.len(), expected_count);
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_sources_with_backend(&document, 0, &features, backend, None)
            .unwrap_or_else(|cause| panic!("{backend:?}: {cause}"))
    });
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum::<f64>()
    };
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    let temp = tempfile::tempdir().unwrap();
    for feature in features {
        let expected_bores = model::read_boolean(&document, feature).unwrap().tools.len();
        let a = rust.solid(feature).unwrap();
        let b = native.solid(feature).unwrap();
        assert!(a.uses_rust_geometry());
        assert_eq!(a.solid_count().unwrap(), 1);
        near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
        near(area(a), area(b));
        for solid in [a, b] {
            assert_eq!(
                solid
                    .faces()
                    .unwrap()
                    .iter()
                    .filter(|face| face.plane.is_none())
                    .count(),
                expected_bores
            );
        }
        let (a_bounds, b_bounds) = (a.bounds().unwrap(), b.bounds().unwrap());
        for axis in 0..3 {
            near(a_bounds.min[axis], b_bounds.min[axis]);
            near(a_bounds.max[axis], b_bounds.max[axis]);
        }
        let receipt = a
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        assert!(!receipt.mesh.indices.is_empty());
        let path = temp.path().join(format!("{feature}.step"));
        a.write_step(&path).unwrap();
        for imported in [
            Solid::read_step(&path, compiler.policy).unwrap(),
            Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                feature,
                compiler.policy,
            )
            .unwrap(),
        ] {
            near(imported.volume_mm3().unwrap(), a.volume_mm3().unwrap());
            near(area(&imported), area(a));
        }
    }
}

#[test]
fn front_panel_end_plates_rebuild_with_orthogonal_blind_and_through_bores() {
    compare_saved_cuts(
        include_bytes!("../../../examples/front-panel.tcd"),
        |_, entity| {
            matches!(
                entity.name.as_str(),
                "Right end part holes" | "Left end part holes"
            )
        },
        2,
    );
}

#[test]
fn toolcase_slide_spacers_rebuild_with_side_through_bores() {
    compare_saved_cuts(
        include_bytes!("../../../examples/toolcase.tcd"),
        |document, entity| {
            model::read_boolean(document, entity.id)
                .ok()
                .and_then(|source| document.get(source.target.producer))
                .is_some_and(|target| target.name.starts_with("Slide spacer "))
        },
        4,
    );
}
