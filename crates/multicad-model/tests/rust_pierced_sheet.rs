use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::LengthUnit,
    model::{self, Cutout, CutoutFace, CutoutShape, length},
    project_file,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{
    Compiler, GeometryBackend,
    export::sheet_dxf::{SheetDxfLayers, encode_developed},
};
use std::io::Cursor;

fn front_panel() -> (Document, EntityId, EntityId) {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/front-panel.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let part = document
        .entities()
        .find(|entity| entity.name == "Front panel sheet")
        .unwrap()
        .id;
    let feature = model::sheet_feature(&document, part).unwrap();
    (document, part, feature)
}

fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-7 * b.abs().max(1.), "{a} != {b}");
}

#[test]
fn saved_front_panel_keeps_all_folded_openings_and_develops_them_into_dxf() {
    let (document, part, feature) = front_panel();
    let compiler = compiler();
    let evaluate = |backend| {
        compiler
            .compile_sources_with_backend(&document, 0, &[part], backend, None)
            .unwrap()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(evaluate);
    let [solid, reference] = [&rust, &native].map(|m| m.solid(feature).unwrap());
    assert!(solid.uses_rust_geometry());
    assert_eq!(solid.solid_count().unwrap(), 1);
    near(solid.volume_mm3().unwrap(), reference.volume_mm3().unwrap());
    let surface_area = |s: &multicad_kernel::Solid| {
        s.faces()
            .unwrap()
            .into_iter()
            .map(|face| face.area_mm2)
            .sum()
    };
    near(surface_area(solid), surface_area(reference));
    let receipt = solid
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    assert!(!receipt.mesh.indices.is_empty());
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("pierced-front-panel.step");
    solid.write_step(&path).unwrap();
    let policy = GeometryPolicy::default();
    for imported in [
        Solid::read_step(&path, policy).unwrap(),
        Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), feature, policy).unwrap(),
    ] {
        assert_eq!(
            imported.faces().unwrap().len(),
            solid.faces().unwrap().len()
        );
        near(imported.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
    }

    let [developed, expected] = [&rust, &native].map(|m| m.unfold(&document, part).unwrap());
    assert_eq!(developed.bends, expected.bends);
    assert_eq!(developed.drills, expected.drills);
    assert_eq!(developed.cut.loops.len(), 16);
    near(developed.area_mm2, expected.area_mm2);
    let loop_areas = |pattern: &multicad_model::DevelopedPattern| {
        let mut areas: Vec<_> = pattern
            .cut
            .loops
            .iter()
            .map(|wire| wire.signed_area())
            .collect();
        areas.sort_by(f64::total_cmp);
        areas
    };
    for (a, b) in loop_areas(&developed)
        .into_iter()
        .zip(loop_areas(&expected))
    {
        near(a, b);
    }
    let bytes = encode_developed(
        &developed,
        LengthUnit::Millimeter,
        &SheetDxfLayers::default(),
        compiler.policy.linear_tolerance_mm,
    )
    .unwrap();
    let dxf = ::dxf::Drawing::load(&mut Cursor::new(bytes)).unwrap();
    assert_eq!(
        dxf.header.default_drawing_units,
        ::dxf::enums::Units::Millimeters
    );
    for (layer, count) in [
        ("CUT", 1),
        ("CUT_INNER", 15),
        ("BEND_DOWN", 2),
        ("DRILL", 14),
    ] {
        assert_eq!(
            dxf.entities().filter(|e| e.common.layer == layer).count(),
            count,
            "{layer}"
        );
    }
}

#[test]
fn formed_sheet_refuses_overlapping_openings_and_a_cut_crossing_a_bend() {
    let (document, part, feature) = front_panel();
    let session = Session::new(document).unwrap();
    let source = model::read_sheet(session.document(), feature).unwrap();
    let duplicate = source.cutouts[0].clone();
    let across_bend = Cutout {
        face: CutoutFace::Flange { index: 0 },
        shape: CutoutShape::Circle {
            center: [length(50.), length(0.5)],
            diameter: length(4.),
        },
    };
    for opening in [duplicate, across_bend] {
        let mut edited = source.clone();
        edited.cutouts.push(opening);
        let prepared =
            model::prepare_sheet_edit(&session, 0, Actor::local(Channel::Ui), feature, &edited)
                .unwrap();
        let error = compiler()
            .compile_sources_with_backend(
                prepared.prepared.document(),
                1,
                &[part],
                GeometryBackend::Rust,
                None,
            )
            .err()
            .expect("the opening must stay in disjoint material inside one face");
        assert_eq!(error.entity, Some(feature), "{error}");
    }
}
