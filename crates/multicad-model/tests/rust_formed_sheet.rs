use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{self, Cutout, CutoutFace, CutoutShape, SheetDefinition, SheetRequest, length},
    project_file,
    references::TopologyKind,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, GeometryBackend};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-7 * b.abs().max(1.), "{a} != {b}");
}

fn assert_same_solid(actual: &Solid, expected: &Solid) {
    let surface_area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .into_iter()
            .map(|face| face.area_mm2)
            .sum()
    };
    assert_eq!(actual.solid_count().unwrap(), 1);
    near(actual.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
    near(surface_area(actual), surface_area(expected));
    let actual = actual.bounds().unwrap();
    let expected = expected.bounds().unwrap();
    for axis in 0..3 {
        near(actual.min[axis], expected.min[axis]);
        near(actual.max[axis], expected.max[axis]);
    }
}

fn front_panel_sheet() -> (Document, SheetDefinition) {
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
    let sheet = model::read_sheet(&document, feature).unwrap();
    (document, sheet)
}

#[test]
fn rust_builds_and_develops_the_saved_front_panel_channel_before_piercing() {
    let (document, mut sheet) = front_panel_sheet();
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
        let rust = evaluate(GeometryBackend::Rust);
        let solid = rust.solid(request.feature).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
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
        let imported = Solid::import_rust_step(
            &std::fs::read_to_string(&step).unwrap(),
            request.feature,
            policy,
        )
        .unwrap();
        assert_eq!(imported.faces().unwrap().len(), 14);
        assert_same_solid(&imported, solid);
        let developed = rust.unfold(session.document(), request.body).unwrap();
        assert_eq!(developed.bends.len(), 2);
        assert_eq!(developed.cut.loops.len(), 1);
        if cfg!(feature = "occt") {
            let native = evaluate(GeometryBackend::OpenCascade);
            let reference = native.solid(request.feature).unwrap();
            assert_same_solid(solid, reference);
            let readback = Solid::read_step(&step, policy).unwrap();
            assert_eq!(readback.faces().unwrap().len(), 14);
            assert_same_solid(&readback, reference);
            let expected = native.unfold(session.document(), request.body).unwrap();
            assert_eq!(developed.bends, expected.bends);
            near(developed.area_mm2, expected.area_mm2);
            for (a, b) in developed.size_mm.into_iter().zip(expected.size_mm) {
                near(a, b);
            }
        }
    }
}

#[test]
fn rust_builds_and_develops_a_single_full_length_bend_with_a_web_opening() {
    let (document, sheet) = front_panel_sheet();
    let mut session = Session::new(document).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let temporary = tempfile::tempdir().unwrap();
    for (reverse, index) in [(false, 0), (true, 1)] {
        let flange = sheet.flanges[index].clone();
        let opposite = (flange.edge + 2) % 4;
        let request = SheetRequest::new(
            "One-sided front panel",
            SheetDefinition {
                reverse,
                flanges: vec![flange],
                fasteners: vec![],
                cutouts: vec![Cutout {
                    face: CutoutFace::Base,
                    shape: CutoutShape::Circle {
                        center: [length(20.), length(20.)],
                        diameter: length(8.),
                    },
                }],
                ..sheet.clone()
            },
        );
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
                .unwrap_or_else(|error| panic!("{backend:?}: {error}"))
        };
        let rust = evaluate(GeometryBackend::Rust);
        let solid = rust.solid(request.feature).unwrap();
        assert!(solid.uses_rust_geometry());
        let role = format!("sheet.base.edge.{opposite}");
        let topology = solid
            .topology_snapshot(request.feature, session.revision())
            .unwrap();
        assert!(topology.elements.iter().any(|element| {
            element.key.kind == TopologyKind::Face
                && element
                    .origins
                    .iter()
                    .any(|origin| origin.role.ends_with(&role))
        }));
        assert!(
            !solid
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap()
                .mesh
                .indices
                .is_empty()
        );
        let step = temporary.path().join(format!("single-{reverse}.step"));
        solid.write_step(&step).unwrap();
        let rust_readback = Solid::import_rust_step(
            &std::fs::read_to_string(&step).unwrap(),
            request.feature,
            compiler.policy,
        )
        .unwrap();
        assert_same_solid(&rust_readback, solid);
        let flat = rust.unfold(session.document(), request.body).unwrap();
        assert_eq!(flat.bends.len(), 1);
        assert_eq!(flat.cut.loops.len(), 2);
        assert_eq!(flat.drills.len(), 1);
        if cfg!(feature = "occt") {
            let native = evaluate(GeometryBackend::OpenCascade);
            let expected = native.solid(request.feature).unwrap();
            assert_same_solid(solid, expected);
            assert_same_solid(&Solid::read_step(&step, compiler.policy).unwrap(), expected);
            let expected_flat = native.unfold(session.document(), request.body).unwrap();
            assert_eq!(flat.bends, expected_flat.bends);
            assert_eq!(flat.drills, expected_flat.drills);
            near(flat.area_mm2, expected_flat.area_mm2);
            for (a, b) in flat.size_mm.into_iter().zip(expected_flat.size_mm) {
                near(a, b);
            }
        }
    }
}

#[test]
fn control_desk_reliefs_form_one_solid_and_develop_exactly() {
    use multicad_core::{
        document::{Entity, EntityData, EntityId},
        materials::{LibraryDefinition, Stock, StockKind, prepare_library},
        model::{
            BendDirection, BodyOutput, Flange, FlangeKind, PanelPlane, Relief, SheetBase, angle,
        },
    };
    let actor = Actor::local(Channel::Ui);
    let mut session = Session::new(Document::new("Control Desk inset shell").unwrap()).unwrap();
    let material = Entity::new(
        "AlMg3",
        EntityData::Material {
            density_kg_m3: Some(2660.),
            color_rgba: [180, 184, 190, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: material.id,
        specification: StockKind::Sheet {
            thickness_mm: 1.5,
            length_mm: 1000.,
            width_mm: 500.,
            inside_bend_radius_mm: 1.5,
            k_factor: 0.44,
        },
    })
    .entity(EntityId::new(), "AlMg3 1.5 sheet")
    .unwrap();
    let stock_id = stock.id;
    session
        .commit(prepare_library(&session, 0, actor.clone(), &[material, stock]).unwrap())
        .unwrap();
    let flange = |edge, leg| Flange {
        edge,
        kind: FlangeKind::Bend { angle: angle(90.) },
        direction: BendDirection::Down,
        length: length(leg),
        inside_radius: None,
        start_inset: Some(length(14.)),
        end_inset: Some(length(14.)),
        relief: Some(Relief {
            width: length(2.),
            depth: length(4.),
        }),
    };
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    for reverse in [false, true] {
        let sheet = SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stock_id,
            base: SheetBase::Rectangle {
                plane: PanelPlane::Xy,
                origin: [length(0.), length(0.), length(0.)],
                length: length(240.),
                width: length(160.),
            },
            reverse,
            inside_radius: None,
            k_factor: None,
            flanges: vec![flange(0, 50.), flange(2, 30.)],
            cutouts: vec![
                Cutout {
                    face: CutoutFace::Base,
                    shape: CutoutShape::Rectangle {
                        center: [length(120.), length(70.)],
                        size: [length(40.), length(22.)],
                    },
                },
                Cutout {
                    face: CutoutFace::Base,
                    shape: CutoutShape::Circle {
                        center: [length(23.), length(90.)],
                        diameter: length(8.),
                    },
                },
                Cutout {
                    face: CutoutFace::Flange { index: 1 },
                    shape: CutoutShape::Circle {
                        center: [length(30.), length(15.)],
                        diameter: length(4.2),
                    },
                },
                Cutout {
                    face: CutoutFace::Flange { index: 1 },
                    shape: CutoutShape::Circle {
                        center: [length(182.), length(15.)],
                        diameter: length(4.2),
                    },
                },
            ],
            fasteners: vec![],
        };
        let request = SheetRequest::new("Relieved Control Desk panel", sheet);
        let prepared =
            model::prepare_sheet(&session, session.revision(), actor.clone(), &request).unwrap();
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
                .unwrap_or_else(|error| panic!("{backend:?}: {error}"))
        };
        let rust = evaluate(GeometryBackend::Rust);
        let solid = rust.solid(request.feature).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        let pi = std::f64::consts::PI;
        let web_area = 240. * 160. - 4. * 2. * 4. - 40. * 22. - 16. * pi;
        let flange_span = 240. - 2. * 14.;
        let flange_holes = 2. * pi * 2.1_f64.powi(2);
        near(
            solid.volume_mm3().unwrap(),
            1.5 * (web_area - flange_holes)
                + flange_span * (1.5 * 80. + pi * 0.5 * (3_f64.powi(2) - 1.5_f64.powi(2))),
        );
        let topology = solid
            .topology_snapshot(request.feature, session.revision())
            .unwrap();
        for role in [
            "sheet.web/cap.start",
            "sheet.web/cap.end",
            "sheet.flange.0.bend.0.a",
            "sheet.flange.1.leg.b",
        ] {
            assert_eq!(
                topology
                    .elements
                    .iter()
                    .filter(|face| face.key.kind == TopologyKind::Face
                        && face.origins.iter().any(|origin| origin.role == role))
                    .count(),
                1,
                "{role}"
            );
        }
        let receipt = solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        assert!(!receipt.mesh.indices.is_empty());
        let developed = rust.unfold(session.document(), request.body).unwrap();
        assert_eq!(developed.cut.loops.len(), 5);
        assert_eq!(developed.cut.loops[0].edges.len(), 24);
        near(
            developed.area_mm2,
            web_area - flange_holes + flange_span * (80. + pi * (1.5 + 0.44 * 1.5)),
        );

        #[cfg(feature = "occt")]
        {
            let native = evaluate(GeometryBackend::OpenCascade);
            let expected = native.solid(request.feature).unwrap();
            let bounds = |solid: &Solid| {
                let b = solid.bounds().unwrap();
                b.min.into_iter().chain(b.max)
            };
            let surface = |solid: &Solid| solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            let step = tempfile::tempdir().unwrap();
            let path = step.path().join("relieved.step");
            solid.write_step(&path).unwrap();
            let imported = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            for measured in [solid, &imported] {
                assert_eq!(measured.solid_count().unwrap(), 1);
                near(
                    measured.volume_mm3().unwrap(),
                    expected.volume_mm3().unwrap(),
                );
                near(surface(measured), surface(expected));
                for (a, b) in bounds(measured).zip(bounds(expected)) {
                    near(a, b);
                }
            }
            let flat = native.unfold(session.document(), request.body).unwrap();
            near(flat.area_mm2, developed.area_mm2);
            for (a, b) in flat.size_mm.into_iter().zip(developed.size_mm) {
                near(a, b);
            }
        }
    }
}
