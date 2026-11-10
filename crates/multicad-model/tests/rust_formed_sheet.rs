use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, EntityData, EntityId},
    model::{self, Cutout, CutoutFace, CutoutShape, SheetDefinition, SheetRequest, length},
    project_file,
    references::TopologyKind,
    sketch::{self, CreationPlane, PlaneOptions},
    sketch_regions,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, GeometryBackend, Model};

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

fn check_saved_step_import(path: &std::path::Path, expected: &Solid) {
    let input =
        multicad_model::imports::read_step_file_with_backend(path, GeometryBackend::Rust).unwrap();
    let mut session = Session::new(Document::new("Imported folded sheet").unwrap()).unwrap();
    let prepared = model::imports::prepare_step_import(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &input,
    )
    .unwrap();
    let [output] = prepared.model.outputs.as_slice() else {
        panic!("Expected one folded solid");
    };
    let part = output.body;
    session.commit(prepared.model.prepared).unwrap();
    let mut bytes = Vec::new();
    project_file::write(
        &project_file::ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    std::fs::remove_file(path).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(&reopened.document, 0, GeometryBackend::Rust, None)
    .unwrap();
    assert_same_solid(model.solid(part).unwrap(), expected);
    check_inset_sections(model.solid(part).unwrap());
}

fn check_inset_sections(solid: &Solid) {
    use multicad_core::{
        drawings::{VectorCurve, VectorRole},
        materials::HatchPattern,
    };
    use multicad_kernel::{
        DrawingHatch, DrawingProjection, DrawingSolid, PlaneFrame, project_drawing,
    };

    let bend_area = std::f64::consts::FRAC_PI_2 * (3_f64.powi(2) - 1.5_f64.powi(2));
    let round_chord = 2. * (2.1_f64.powi(2) - 0.5_f64.powi(2)).sqrt();
    for (x, expected_area, bends) in [
        (7., 160. * 1.5, false),
        (120., (160. + 50. + 30. - 22. - 8.) * 1.5 + bend_area, true),
        (
            44.5,
            (160. + 50. + 30. - round_chord) * 1.5 + bend_area,
            true,
        ),
    ] {
        let vectors = project_drawing(
            &[DrawingSolid {
                solid,
                hatch: Some(DrawingHatch {
                    angle_rad: 0.,
                    spacing_mm: 2.,
                    line_width_mm: 0.2,
                    pattern: HatchPattern::Solid,
                }),
            }],
            DrawingProjection {
                frame: PlaneFrame {
                    origin: [0.; 3],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                },
                section_only: true,
                section_offset_mm: Some(x),
                chord_tolerance_mm: 0.0001,
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| panic!("Section at X={x}: {error}"));
        let mut radii = Vec::new();
        let mut hatch_area = 0.;
        for vector in vectors {
            match (vector.role, vector.curve) {
                (VectorRole::Section, VectorCurve::Line { .. }) => {}
                (
                    VectorRole::Section,
                    VectorCurve::Ellipse {
                        major_mm, minor_mm, ..
                    },
                ) => {
                    near(major_mm, minor_mm);
                    radii.push(major_mm);
                }
                (VectorRole::Hatch, VectorCurve::HatchTriangle { points: [a, b, c] }) => {
                    hatch_area +=
                        0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs();
                }
                other => panic!("Unexpected inset section output: {other:?}"),
            }
        }
        radii.sort_by(f64::total_cmp);
        if bends {
            assert_eq!(radii.len(), 4);
            for (actual, expected) in radii.into_iter().zip([1.5, 1.5, 3., 3.]) {
                near(actual, expected);
            }
        } else {
            assert!(radii.is_empty());
        }
        assert!(
            (hatch_area - expected_area).abs() < 0.02,
            "Section X={x}: hatch area {hatch_area}, expected {expected_area}"
        );
    }
}

fn prepare_sheet_rectangle(
    session: &mut Session,
    model: &Model,
    part: EntityId,
    cutout: CutoutFace,
    developed: bool,
    [x, y]: [f64; 2],
    [width, height]: [f64; 2],
) -> (EntityId, model::ModelPrepared) {
    let actor = Actor::local(Channel::Ui);
    let producer = model.snapshot().body_sources[&part];
    let rectangle = [
        [x, y],
        [x + width, y],
        [x + width, y + height],
        [x, y + height],
    ];
    let new_sketch = |plane| {
        sketch::prepare_new(
            session,
            session.revision(),
            actor.clone(),
            "Sheet opening",
            plane,
            PlaneOptions::default(),
        )
        .unwrap()
    };
    let (created, points) = if developed {
        let CutoutFace::Flange { index } = cutout else {
            panic!("This fixture uses the developed chart to target a flange");
        };
        let flat = model.unfold(session.document(), part).unwrap();
        let leg = flat
            .chart
            .legs
            .iter()
            .find(|leg| leg.index == index)
            .unwrap();
        let [origin, end_u, _, end_v] = leg.quad;
        let unit = |end: [f64; 2]| {
            let delta = [end[0] - origin[0], end[1] - origin[1]];
            let length = delta[0].hypot(delta[1]);
            delta.map(|v| v / length)
        };
        let [u, v] = [unit(end_u), unit(end_v)];
        let points =
            rectangle.map(|[a, b]| std::array::from_fn(|i| origin[i] + a * u[i] + b * v[i]));
        (
            new_sketch(CreationPlane::DevelopedSheet { part, face: cutout }),
            points,
        )
    } else {
        let authored = model
            .sheet_authoring_faces(session.document())
            .into_iter()
            .find(|frame| frame.part == part && frame.face == cutout)
            .unwrap();
        let topology = &model.snapshot().shapes[&producer].topology;
        let role = match cutout {
            CutoutFace::Base => "sheet.web/cap.start".into(),
            CutoutFace::Flange { index } => format!("sheet.flange.{index}.leg.a"),
        };
        let selected = topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face
                    && element
                        .origins
                        .iter()
                        .any(|origin| origin.source == authored.feature && origin.role == role)
            })
            .unwrap();
        let created = new_sketch(CreationPlane::Face {
            snapshot: topology,
            key: selected.key,
            anchor: Some(&selected.frames[0].origin),
        });
        let points = rectangle.map(|[u, v]| {
            created.frame.coordinates(std::array::from_fn(|i| {
                authored.origin_mm[i] + u * authored.u[i] + v * authored.v[i]
            }))
        });
        (created, points)
    };
    let id = created.sketch;
    session.commit(created.prepared).unwrap();
    let EntityData::Sketch { definition, .. } = &session.document().get(id).unwrap().data else {
        panic!("Expected a sketch");
    };
    let mut source = Source::from_definition(definition).unwrap();
    let points = points.map(|point| source.add_point(point));
    for i in 0..4 {
        source.add_line(points[i], points[(i + 1) % 4]);
    }
    let region = sketch_regions::extract(&source).regions[0].intent();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor.clone(),
            label: "Draw the opening".into(),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
    let feature = EntityId::new();
    let prepared = model::prepare_sheet_profile_cut(
        session,
        session.revision(),
        actor,
        feature,
        &model::SheetProfileCutDefinition {
            input: model::BodyInput {
                body: part,
                producer,
            },
            output: model::BodyOutput { body: part },
            profile: model::ProfileSource { sketch: id, region },
            face: cutout,
        },
    )
    .unwrap();
    (feature, prepared)
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
        document::Entity,
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
    let stock_document = session.document().clone();
    for reverse in [false, true] {
        let mut session = Session::new(stock_document.clone()).unwrap();
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
                Cutout {
                    face: CutoutFace::Flange { index: 1 },
                    shape: CutoutShape::Rectangle {
                        center: [length(106.), length(15.)],
                        size: [length(42.), length(8.)],
                    },
                },
            ],
            fasteners: vec![],
        };
        let request = SheetRequest::new("Relieved Control Desk panel", sheet);
        #[cfg(feature = "occt")]
        let mut native_source = Session::new(session.document().clone()).unwrap();
        let prepared =
            model::prepare_sheet(&session, session.revision(), actor.clone(), &request).unwrap();
        session.commit(prepared.prepared).unwrap();
        let evaluate = |session: &Session, backend| {
            compiler
                .compile_with_backend(session.document(), session.revision(), backend, None)
                .unwrap_or_else(|error| panic!("{backend:?}: {error}"))
        };
        let rust = evaluate(&session, GeometryBackend::Rust);
        let solid = rust.solid(request.feature).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        let pi = std::f64::consts::PI;
        let web_area = 240. * 160. - 4. * 2. * 4. - 40. * 22. - 16. * pi;
        let flange_span = 240. - 2. * 14.;
        let flange_holes = 2. * pi * 2.1_f64.powi(2) + 42. * 8.;
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
        assert_eq!(developed.cut.loops.len(), 6);
        assert_eq!(developed.cut.loops[0].edges.len(), 24);
        near(
            developed.area_mm2,
            web_area - flange_holes + flange_span * (80. + pi * (1.5 + 0.44 * 1.5)),
        );

        #[cfg(feature = "occt")]
        {
            let native = evaluate(&session, GeometryBackend::OpenCascade);
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

        let (flange_cut, prepared) = prepare_sheet_rectangle(
            &mut session,
            &rust,
            request.body,
            CutoutFace::Flange { index: 0 },
            false,
            [63., 15.],
            [34., 10.],
        );
        session.commit(prepared.prepared).unwrap();
        let flanged = evaluate(&session, GeometryBackend::Rust);
        assert_eq!(flanged.snapshot().body_sources[&request.body], flange_cut);
        #[cfg(feature = "occt")]
        assert_same_solid(
            flanged.solid(flange_cut).unwrap(),
            evaluate(&session, GeometryBackend::OpenCascade)
                .solid(flange_cut)
                .unwrap(),
        );
        let (cut, prepared) = prepare_sheet_rectangle(
            &mut session,
            &flanged,
            request.body,
            CutoutFace::Base,
            false,
            [155., 105.],
            [22., 14.],
        );
        session.commit(prepared.prepared).unwrap();
        let removed_area = 34. * 10. + 22. * 14.;
        let mut bytes = vec![];
        project_file::write(
            &project_file::ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let restored = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let reopened = Session::new(restored.document).unwrap();
        let folded = evaluate(&reopened, GeometryBackend::Rust);
        assert_eq!(folded.snapshot().body_sources[&request.body], cut);
        let punched = folded.solid(cut).unwrap();
        check_inset_sections(punched);
        near(
            solid.volume_mm3().unwrap() - punched.volume_mm3().unwrap(),
            removed_area * 1.5,
        );
        assert_eq!(punched.solid_count().unwrap(), 1);
        assert_eq!(punched.bounds().unwrap(), solid.bounds().unwrap());
        assert!(
            punched
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap()
                .edge_pairs
                > 0
        );
        let flat = folded.unfold(reopened.document(), request.body).unwrap();
        near(developed.area_mm2 - flat.area_mm2, removed_area);
        assert_eq!(flat.cut.loops.len(), 8);
        use multicad_model::export::{
            Exporter,
            sheet_dxf::{SheetDxfLayers, SheetDxfOptions},
        };
        let dxf = Exporter::default()
            .developed_sheet_dxf_with_backend(
                reopened.document(),
                reopened.revision(),
                request.body,
                SheetDxfOptions {
                    backend: GeometryBackend::Rust,
                    units: multicad_core::exchange::LengthUnit::Millimeter,
                    layers: &SheetDxfLayers::default(),
                },
                None,
            )
            .unwrap();
        assert_eq!(dxf.pattern.cut, flat.cut);
        assert!(!dxf.bytes.is_empty());
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("rectangular-aperture.step");
        punched.write_step(&path).unwrap();
        let verified = multicad_model::export::verify_rust_step_shape(
            &path,
            punched,
            &folded.snapshot().shapes[&cut],
            compiler.policy.linear_tolerance_mm,
        )
        .unwrap();
        near(verified, punched.volume_mm3().unwrap());
        #[cfg(feature = "occt")]
        {
            // Independently cut the same two rectangles in OpenCascade.
            let mut native_request = SheetRequest::new(&request.name, request.sheet.clone());
            native_request.sheet.cutouts.extend([
                Cutout {
                    face: CutoutFace::Flange { index: 0 },
                    shape: CutoutShape::Rectangle {
                        center: [length(80.), length(20.)],
                        size: [length(34.), length(10.)],
                    },
                },
                Cutout {
                    face: CutoutFace::Base,
                    shape: CutoutShape::Rectangle {
                        center: [length(166.), length(112.)],
                        size: [length(22.), length(14.)],
                    },
                },
            ]);
            let prepared = model::prepare_sheet(
                &native_source,
                native_source.revision(),
                actor.clone(),
                &native_request,
            )
            .unwrap();
            native_source.commit(prepared.prepared).unwrap();
            let native = evaluate(&native_source, GeometryBackend::OpenCascade);
            let expected = native.solid(native_request.feature).unwrap();
            assert_same_solid(punched, expected);
            assert_same_solid(
                &Solid::read_step(&path, GeometryPolicy::default()).unwrap(),
                expected,
            );
            near(
                native
                    .unfold(native_source.document(), native_request.body)
                    .unwrap()
                    .area_mm2,
                flat.area_mm2,
            );
        }
        check_saved_step_import(&path, punched);

        let current = evaluate(&session, GeometryBackend::Rust);
        let (invalid, rejected) = prepare_sheet_rectangle(
            &mut session,
            &current,
            request.body,
            CutoutFace::Flange { index: 0 },
            true,
            [110., -2.],
            [20., 7.],
        );
        let error = compiler
            .compile_with_backend(
                rejected.prepared.document(),
                session.revision(),
                GeometryBackend::Rust,
                None,
            )
            .err()
            .expect("An opening cannot cross the bend tangent");
        assert_eq!(error.entity, Some(invalid));
        assert!(
            error.message.contains("clear of its edges, bends"),
            "{error}"
        );
        assert!(session.document().get(invalid).is_none());
    }
}
