use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::{Grain, LibraryDefinition, Stock, StockKind},
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, Model};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, WoodBoxRequest) {
    let material = Entity::new(
        "Birch",
        EntityData::Material {
            density_kg_m3: Some(650.),
            color_rgba: [210, 180, 140, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: material.id,
        specification: StockKind::Panel {
            thickness_mm: 18.,
            length_mm: 2500.,
            width_mm: 1250.,
            grain: Grain::None,
        },
    })
    .entity(EntityId::new(), "Birch 18")
    .unwrap();
    let request = WoodBoxRequest::new(
        "Editable box",
        WoodBoxDefinition::new(stock.id, length(600.), length(400.), length(300.)),
    );
    let session = Session::new(
        Document::new("Native six-face box")
            .unwrap()
            .edited(&[Mutation::Insert(material), Mutation::Insert(stock)])
            .unwrap(),
    )
    .unwrap();
    (session, request)
}
fn native(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn apply(session: &mut Session, request: &WoodBoxRequest) {
    let prepared = prepare_wood_box(
        session,
        session.revision(),
        actor(),
        &WoodBoxChange::Set(Box::new(request.clone())),
    )
    .unwrap();
    Compiler::default()
        .compile(prepared.model.prepared.document(), session.revision() + 1)
        .unwrap();
    session.commit(prepared.model.prepared).unwrap();
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}
#[test]
fn closed_open_top_omitted_side_and_multiple_omissions_produce_separate_nonoverlapping_native_panels()
 {
    for (index, absent) in [
        vec![],
        vec![WoodBoxFace::Top],
        vec![WoodBoxFace::Left],
        vec![WoodBoxFace::Left, WoodBoxFace::Top, WoodBoxFace::Front],
    ]
    .into_iter()
    .enumerate()
    {
        let (mut session, mut request) = fixture();
        if index == 3 {
            request.definition.origin = [length(100.), length(-250.), length(40.)];
            request.definition.rotation = angle(30.);
        }
        for face in absent {
            request.definition.faces.remove(&face);
        }
        apply(&mut session, &request);
        let native = native(&session);
        assert_eq!(
            native.snapshot().bodies.len(),
            request.definition.faces.len()
        );
        let layout = resolve_wood_box(
            session.document(),
            session.parameters(),
            &request.definition,
        )
        .unwrap();
        for panel in &layout.panels {
            let id = &request.definition.panels[&panel.face];
            let solid = native.solid(id.body).unwrap();
            assert_eq!(solid.solid_count().unwrap(), 1);
            near(solid.volume_mm3().unwrap(), panel.panel.volume_mm3());
            let expected = panel.corners().into_iter().fold(
                [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]],
                |mut b, p| {
                    for axis in 0..3 {
                        b[0][axis] = b[0][axis].min(p[axis]);
                        b[1][axis] = b[1][axis].max(p[axis]);
                    }
                    b
                },
            );
            let bounds = solid.bounds().unwrap();
            for (actual, expected) in bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip(expected.into_iter().flatten())
            {
                near(actual, expected);
            }
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("face.step");
            solid.write_step(&path).unwrap();
            let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            near(readback.volume_mm3().unwrap(), panel.panel.volume_mm3());
            assert_eq!(readback.solid_count().unwrap(), 1);
        }
        let ids: Vec<_> = request
            .definition
            .faces
            .iter()
            .map(|face| request.definition.panels[face].body)
            .collect();
        for (i, a) in ids.iter().enumerate() {
            for b in ids.iter().skip(i + 1) {
                let relation = native
                    .solid(*a)
                    .unwrap()
                    .inspect_pair(native.solid(*b).unwrap(), 1e-7, GeometryPolicy::default())
                    .unwrap();
                near(relation.common_volume_mm3, 0.);
            }
        }
        let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        for id in ids {
            let b = native.solid(id).unwrap().bounds().unwrap();
            for (value, coordinate) in bounds[0].iter_mut().zip(b.min) {
                *value = value.min(coordinate);
            }
            for (value, coordinate) in bounds[1].iter_mut().zip(b.max) {
                *value = value.max(coordinate);
            }
        }
        for (actual, expected) in bounds
            .into_iter()
            .flatten()
            .zip(layout.actual_bounds.into_iter().flatten())
        {
            near(actual, expected);
        }
    }
}
#[test]
fn downstream_inside_pocket_follows_recipe_dimensions_and_reopens_without_stale_panels() {
    let (mut session, mut request) = fixture();
    request.definition.faces.remove(&WoodBoxFace::Top);
    apply(&mut session, &request);
    let bottom = request.definition.panels[&WoodBoxFace::Bottom].clone();
    let op = MachiningDefinition {
        generated_by: None,
        kind: MachiningKind::Pocket {
            center: [length(100.), length(100.)],
            size: [length(40.), length(30.)],
            depth: length(5.),
        },
        face: MachiningFace::Inside,
        targets: vec![bottom.body],
        suppressed: false,
    };
    let prepared =
        prepare_machining(&session, session.revision(), actor(), "Inside pocket", &op).unwrap();
    Compiler::default()
        .compile(prepared.model.prepared.document(), session.revision() + 1)
        .unwrap();
    session.commit(prepared.model.prepared).unwrap();
    near(
        native(&session)
            .solid(bottom.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        564. * 364. * 18. - 40. * 30. * 5.,
    );
    request.definition.width = length(700.);
    apply(&mut session, &request);
    near(
        native(&session)
            .solid(bottom.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        664. * 364. * 18. - 40. * 30. * 5.,
    );
    assert_eq!(
        read_wood_box(session.document(), request.id).unwrap(),
        request.definition
    );
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let file = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let reopened = Session::new(file.document).unwrap();
    near(
        native(&reopened)
            .solid(bottom.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        664. * 364. * 18. - 40. * 30. * 5.,
    );
    let before = session.document().clone();
    let mut too_small = request.clone();
    too_small.definition.width = length(100.);
    let prepared = prepare_wood_box(
        &session,
        session.revision(),
        actor(),
        &WoodBoxChange::Set(Box::new(too_small)),
    )
    .unwrap();
    assert!(
        Compiler::default()
            .compile(prepared.model.prepared.document(), session.revision() + 1)
            .is_err(),
        "Resize must diagnose the pocket outside the smaller panel"
    );
    assert_eq!(session.document(), &before);
    request.definition.faces.remove(&WoodBoxFace::Bottom);
    assert!(
        prepare_wood_box(
            &session,
            session.revision(),
            actor(),
            &WoodBoxChange::Set(Box::new(request))
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
}
