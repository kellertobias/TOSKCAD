//! Source-only sheet cut history. Native folded/developed geometry is tested
//! by the model compiler.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    materials::{LibraryDefinition, Stock, StockKind, prepare_library},
    model::{
        self, BodyInput, BodyOutput, Cutout, CutoutFace, CutoutShape, PanelPlane, ProfileSource,
        SHEET_PROFILE_CUT_CAPABILITY, SheetBase, SheetDefinition, SheetProfileCutDefinition,
        SheetRequest, length,
    },
    project_file::{self, ProjectFile},
    references::{
        AnchoredFrame, Origin, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind,
        TopologySnapshot,
    },
    sketch::{self, CreationPlane},
    sketch_regions,
};
use std::{collections::BTreeSet, io::Cursor};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn fixture() -> (Session, SheetRequest, ProfileSource) {
    let mut session = Session::new(Document::new("Sheet cut source").unwrap()).unwrap();
    let material = Entity::new(
        "Aluminium",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [182, 188, 202, 255],
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
            k_factor: 0.4,
        },
    })
    .entity(EntityId::new(), "Aluminium 1.5 mm")
    .unwrap();
    let request = SheetRequest::new(
        "Sheet",
        SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stock.id,
            base: SheetBase::Rectangle {
                plane: PanelPlane::Xy,
                origin: [length(0.), length(0.), length(0.)],
                length: length(100.),
                width: length(80.),
            },
            reverse: false,
            inside_radius: None,
            k_factor: None,
            flanges: vec![],
            cutouts: vec![Cutout {
                face: CutoutFace::Base,
                shape: CutoutShape::Circle {
                    center: [length(60.), length(50.)],
                    diameter: length(4.),
                },
            }],
            fasteners: vec![],
        },
    );
    session
        .commit(prepare_library(&session, 0, actor(), &[material, stock]).unwrap())
        .unwrap();
    session
        .commit(
            model::prepare_sheet(&session, session.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let origin = Origin {
        source: request.feature,
        role: "cap.start".into(),
    };
    let face = TopologyKey {
        kind: TopologyKind::Face,
        ordinal: 0,
    };
    let mut snapshot = TopologySnapshot {
        owner: request.feature,
        revision: session.revision(),
        elements: vec![TopologyElement {
            key: face,
            origins: BTreeSet::from([origin.clone()]),
            boundary: BTreeSet::new(),
            frames: vec![AnchoredFrame {
                origin: origin.clone(),
                frame: ReferenceFrame {
                    origin_mm: [0.; 3],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
            }],
            geometry: None,
        }],
    };
    let new = sketch::prepare_new(
        &session,
        session.revision(),
        actor(),
        "Attached cutting sketch",
        CreationPlane::Face {
            snapshot: &snapshot,
            key: face,
            anchor: Some(&origin),
        },
        Default::default(),
    )
    .unwrap();
    let id = new.sketch;
    session.commit(new.prepared).unwrap();
    snapshot.revision = session.revision();
    let report = multicad_core::references::inspect(
        session.document(),
        session.revision(),
        &[(request.feature, snapshot)].into(),
    );
    let mut source = Source::read_evaluated(session.document(), id, session.revision(), &report)
        .unwrap()
        .0;
    source.add_rectangle([20., 20.], [40., 30.]).unwrap();
    let region = sketch_regions::extract(&source).regions[0].intent();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Draw cutting profile".into(),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
    (session, request, ProfileSource { sketch: id, region })
}

fn cut(sheet: &SheetRequest, profile: ProfileSource) -> SheetProfileCutDefinition {
    SheetProfileCutDefinition {
        output: BodyOutput { body: sheet.body },
        input: BodyInput {
            body: sheet.body,
            producer: sheet.feature,
        },
        profile,
        face: CutoutFace::Base,
    }
}

#[test]
fn attached_profile_stays_live_without_a_cycle_through_save_reopen_and_undo() {
    let (mut session, sheet, profile) = fixture();
    let original = session.document().clone();
    let definition = cut(&sheet, profile);
    let first = EntityId::new();
    let preview =
        model::prepare_sheet_profile_cut(&session, session.revision(), actor(), first, &definition)
            .unwrap();
    assert_eq!(preview.outputs[0].body, sheet.body);
    assert_eq!(session.document(), &original);
    let candidate = preview.prepared.document();
    let EntityData::Feature { inputs, .. } = &candidate.get(first).unwrap().data else {
        panic!("expected a profile cut feature");
    };
    assert_eq!(
        inputs,
        &[
            Link::new(sheet.feature, "source"),
            Link::new(definition.profile.sketch, "profile")
        ],
    );
    assert!(
        candidate
            .affected([definition.profile.sketch])
            .unwrap()
            .contains(&first)
    );
    assert!(
        !candidate
            .affected([definition.profile.sketch])
            .unwrap()
            .contains(&sheet.feature)
    );
    assert_eq!(
        model::read_sheet(candidate, sheet.feature).unwrap(),
        sheet.sheet
    );
    assert_eq!(
        model::read_sheet_profile_cut(candidate, first).unwrap(),
        definition
    );
    assert_eq!(
        model::sheet_profile_head(candidate, sheet.body),
        Some(first)
    );

    session.commit(preview.prepared).unwrap();
    let second = EntityId::new();
    assert!(
        model::prepare_sheet_profile_cut(
            &session,
            session.revision(),
            actor(),
            second,
            &definition
        )
        .unwrap_err()
        .message
        .contains("current producer changed")
    );
    let mut continued = definition;
    continued.input.producer = first;
    session
        .commit(
            model::prepare_sheet_profile_cut(
                &session,
                session.revision(),
                actor(),
                second,
                &continued,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();

    let file = ProjectFile::new(session.document().clone()).unwrap();
    assert!(file.capabilities().contains(SHEET_PROFILE_CUT_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(
        model::read_sheet_profile_cut(&opened.project.document, second).unwrap(),
        continued
    );
    assert_eq!(
        model::sheet_profile_head(&opened.project.document, sheet.body),
        Some(second)
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(
        model::sheet_profile_head(session.document(), sheet.body),
        Some(first)
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
}

#[test]
fn malformed_sheet_cut_links_and_unavailable_flange_are_refused() {
    let (session, sheet, profile) = fixture();
    let feature = EntityId::new();
    let mut definition = cut(&sheet, profile);
    definition.face = CutoutFace::Flange { index: 0 };
    assert!(
        model::prepare_sheet_profile_cut(
            &session,
            session.revision(),
            actor(),
            feature,
            &definition
        )
        .unwrap_err()
        .message
        .contains("flange no longer exists")
    );
    definition.face = CutoutFace::Base;
    let preview = model::prepare_sheet_profile_cut(
        &session,
        session.revision(),
        actor(),
        feature,
        &definition,
    )
    .unwrap();
    let document = preview.prepared.document();
    let mut changed = document.get(feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut changed.data else {
        panic!("expected a profile cut feature");
    };
    inputs.push(Link::new(sheet.body, "wrong target"));
    assert!(document.edited(&[Mutation::Replace(changed)]).is_err());

    let mut changed = document.get(sheet.body).unwrap().clone();
    let EntityData::Part { features, .. } = &mut changed.data else {
        panic!("expected a sheet Part");
    };
    features.swap(0, 1);
    assert!(
        document
            .edited(&[Mutation::Replace(changed)])
            .unwrap_err()
            .to_string()
            .contains("captured sheet producer")
    );
}

#[test]
fn developed_chart_remains_linked_to_the_forming_feature_through_cut_and_reopen() {
    let (mut session, sheet, folded_profile) = fixture();
    assert_eq!(
        model::developed_sheet_sketch(session.document(), folded_profile.sketch).unwrap(),
        None
    );
    let folded_cut = EntityId::new();
    let before_chart = cut(&sheet, folded_profile);
    session
        .commit(
            model::prepare_sheet_profile_cut(
                &session,
                session.revision(),
                actor(),
                folded_cut,
                &before_chart,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();

    let new = sketch::prepare_new(
        &session,
        session.revision(),
        actor(),
        "Developed cutting sketch",
        CreationPlane::DevelopedSheet {
            part: sheet.body,
            face: CutoutFace::Base,
        },
        Default::default(),
    )
    .unwrap();
    let sketch = new.sketch;
    let plane = new.plane;
    assert_eq!(new.frame, sketch::Datum::Xy.frame());
    session.commit(new.prepared).unwrap();

    let expected_chart = Some(model::DevelopedSheetSketch {
        part: sheet.body,
        forming: sheet.feature,
        face: CutoutFace::Base,
    });
    assert_eq!(
        model::developed_sheet_sketch(session.document(), sketch).unwrap(),
        expected_chart
    );
    let EntityData::Sketch { inputs, .. } = &session.document().get(sketch).unwrap().data else {
        panic!("expected a developed sketch");
    };
    assert_eq!(inputs, &[Link::new(sheet.feature, "sheet/developed/base")]);
    let mut plane = session.document().get(plane).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut plane.data else {
        panic!("expected a plane");
    };
    origin_mm[2] = 1.;
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(plane)])
            .unwrap_err()
            .to_string()
            .contains("unrotated XY plane")
    );

    let mut source = Source::read(session.document(), sketch).unwrap().0;
    source.add_rectangle([50., 15.], [65., 25.]).unwrap();
    let region = sketch_regions::extract(&source).regions[0].intent();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Draw on the developed pattern".into(),
            edits: source.replace(session.document(), sketch).unwrap(),
        })
        .unwrap();
    assert_eq!(
        model::developed_sheet_sketch(session.document(), sketch).unwrap(),
        expected_chart
    );

    let continued_cut = EntityId::new();
    let mut definition = cut(&sheet, ProfileSource { sketch, region });
    definition.input.producer = folded_cut;
    definition.face = CutoutFace::Flange { index: 0 };
    assert!(
        model::prepare_sheet_profile_cut(
            &session,
            session.revision(),
            actor(),
            continued_cut,
            &definition,
        )
        .unwrap_err()
        .message
        .contains("different sheet Part or face")
    );
    definition.face = CutoutFace::Base;
    session
        .commit(
            model::prepare_sheet_profile_cut(
                &session,
                session.revision(),
                actor(),
                continued_cut,
                &definition,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    assert!(
        session
            .document()
            .affected([sketch])
            .unwrap()
            .contains(&continued_cut)
    );
    assert!(
        !session
            .document()
            .affected([sketch])
            .unwrap()
            .contains(&sheet.feature)
    );
    let file = ProjectFile::new(session.document().clone()).unwrap();
    assert!(
        file.capabilities()
            .contains(model::DEVELOPED_SHEET_SKETCH_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(
        model::developed_sheet_sketch(&opened.project.document, sketch).unwrap(),
        expected_chart
    );
    assert_eq!(
        model::read_sheet_profile_cut(&opened.project.document, continued_cut).unwrap(),
        definition
    );
}

#[test]
fn developed_chart_rejects_unavailable_flange_and_transformed_plane() {
    let (session, sheet, _) = fixture();
    let creation = |face, options| {
        sketch::prepare_new(
            &session,
            session.revision(),
            actor(),
            "Developed chart",
            CreationPlane::DevelopedSheet {
                part: sheet.body,
                face,
            },
            options,
        )
    };
    let Err(invalid_face) = creation(CutoutFace::Flange { index: 0 }, Default::default()) else {
        panic!("a missing flange cannot own a developed chart");
    };
    assert!(invalid_face.message.contains("flange no longer exists"));
    let Err(invalid_plane) = creation(
        CutoutFace::Base,
        sketch::PlaneOptions {
            normal_offset_mm: 1.,
            angle_rad: 0.,
        },
    ) else {
        panic!("a developed sketch must remain in its fixed XY plane");
    };
    assert!(invalid_plane.message.contains("fixed XY plane"));
}
