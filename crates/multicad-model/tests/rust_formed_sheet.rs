use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{self, SheetRequest, length},
    project_file,
    references::TopologyKind,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, GeometryBackend};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-7 * b.abs().max(1.), "{a} != {b}");
}

#[test]
fn rust_builds_and_develops_the_saved_front_panel_channel_before_piercing() {
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
    let mut sheet = model::read_sheet(&document, feature).unwrap();
    sheet.cutouts.clear();
    sheet.fasteners.clear();
    let mut session = Session::new(document).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let temporary = tempfile::tempdir().unwrap();

    for reverse in [false, true] {
        sheet.reverse = reverse;
        let request = SheetRequest::new("Unpierced front panel channel", sheet.clone());
        let prepared = model::prepare_sheet(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &request,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        let evaluate = |backend| {
            compiler
                .compile_sources_with_backend(
                    session.document(),
                    session.revision(),
                    &[request.body],
                    backend,
                    None,
                )
                .unwrap()
        };
        let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(evaluate);
        let [solid, reference] = [&rust, &native].map(|m| m.solid(request.feature).unwrap());
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
        let [a, b] = [solid.bounds().unwrap(), reference.bounds().unwrap()];
        for (x, y) in a
            .min
            .into_iter()
            .chain(a.max)
            .zip(b.min.into_iter().chain(b.max))
        {
            near(x, y);
        }
        let topology = solid
            .topology_snapshot(request.feature, session.revision())
            .unwrap();
        let faces = solid.faces().unwrap();
        assert_eq!(faces.len(), 14);
        assert_eq!(faces.iter().filter(|face| face.plane.is_none()).count(), 4);
        for role in [
            "sheet.web/cap.start",
            "sheet.web/cap.end",
            "sheet.flange.1.leg.a",
        ] {
            assert!(
                topology.elements.iter().any(|face| {
                    face.key.kind == TopologyKind::Face
                        && face
                            .origins
                            .iter()
                            .any(|origin| origin.role.ends_with(role))
                }),
                "Missing folded sheet face role {role}"
            );
        }
        let receipt = solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        assert!(!receipt.mesh.indices.is_empty());
        let step = temporary.path().join(format!("folded-{reverse}.step"));
        solid.write_step(&step).unwrap();
        let policy = GeometryPolicy::default();
        for imported in [
            Solid::read_step(&step, policy).unwrap(),
            Solid::import_rust_step(
                &std::fs::read_to_string(&step).unwrap(),
                request.feature,
                policy,
            )
            .unwrap(),
        ] {
            assert_eq!(imported.faces().unwrap().len(), 14);
            near(
                imported.volume_mm3().unwrap(),
                reference.volume_mm3().unwrap(),
            );
        }
        let [developed, expected] =
            [&rust, &native].map(|m| m.unfold(session.document(), request.body).unwrap());
        assert_eq!(developed.bends, expected.bends);
        assert_eq!(developed.cut.loops.len(), 1);
        near(developed.area_mm2, expected.area_mm2);
        for (a, b) in developed.size_mm.into_iter().zip(expected.size_mm) {
            near(a, b);
        }
    }

    sheet.flanges[0].start_inset = Some(length(5.));
    let request = SheetRequest::new("Inset bend", sheet);
    let prepared = model::prepare_sheet(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &request,
    )
    .unwrap();
    let error = compiler
        .compile_sources_with_backend(
            prepared.prepared.document(),
            session.revision() + 1,
            &[request.body],
            GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("the inset channel needs a nonconstant section");
    assert!(
        error.message.contains("opposite full-length bends"),
        "{error}"
    );
}
