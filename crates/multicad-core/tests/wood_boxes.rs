use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::{Grain, LibraryDefinition, Stock, StockKind},
    model::*,
    project_file::{self, ProjectFile},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (Session, [EntityId; 2]) {
    let mut edits = vec![];
    let mut stocks = vec![];
    for (name, thickness) in [("Birch 18", 18.), ("Birch 24", 24.)] {
        let material = Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: Some(650.),
                color_rgba: [210, 180, 140, 255],
                section_hatch: None,
            },
        );
        let stock = LibraryDefinition::Stock(Stock {
            material: material.id,
            specification: StockKind::Panel {
                thickness_mm: thickness,
                length_mm: 2500.,
                width_mm: 1250.,
                grain: Grain::None,
            },
        })
        .entity(EntityId::new(), name)
        .unwrap();
        stocks.push(stock.id);
        edits.extend([Mutation::Insert(material), Mutation::Insert(stock)]);
    }
    (
        Session::new(
            Document::new("Wood Box source")
                .unwrap()
                .edited(&edits)
                .unwrap(),
        )
        .unwrap(),
        stocks.try_into().unwrap(),
    )
}
fn request(stock: EntityId) -> WoodBoxRequest {
    WoodBoxRequest::new(
        "Box",
        WoodBoxDefinition::new(stock, length(600.), length(400.), length(300.)),
    )
}
fn apply(session: &mut Session, request: &WoodBoxRequest) -> WoodBoxPrepared {
    let prepared = prepare_wood_box(
        session,
        session.revision(),
        actor(),
        &WoodBoxChange::Set(Box::new(request.clone())),
    )
    .unwrap();
    session.commit(prepared.model.prepared.clone()).unwrap();
    prepared
}
#[test]
fn every_nonempty_face_combination_has_positive_panels_without_overlapping_corner_material() {
    let (session, stocks) = fixture();
    for mask in 1u32..64 {
        let mut request = request(stocks[0]);
        request.definition.faces = WoodBoxFace::ALL
            .into_iter()
            .enumerate()
            .filter_map(|(index, face)| (mask & (1 << index) != 0).then_some(face))
            .collect();
        let layout = resolve_wood_box(
            session.document(),
            session.parameters(),
            &request.definition,
        )
        .unwrap();
        assert_eq!(layout.panels.len(), mask.count_ones() as usize);
        let bounds: Vec<_> = layout
            .panels
            .iter()
            .map(|panel| {
                panel.corners().into_iter().fold(
                    [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]],
                    |mut bounds, corner| {
                        for axis in 0..3 {
                            bounds[0][axis] = bounds[0][axis].min(corner[axis]);
                            bounds[1][axis] = bounds[1][axis].max(corner[axis]);
                        }
                        bounds
                    },
                )
            })
            .collect();
        for (i, a) in bounds.iter().enumerate() {
            for b in bounds.iter().skip(i + 1) {
                let overlap: f64 = (0..3)
                    .map(|axis| (a[1][axis].min(b[1][axis]) - a[0][axis].max(b[0][axis])).max(0.))
                    .product();
                assert_eq!(overlap, 0., "Mask {mask} has overlapping panels");
            }
        }
        assert!(
            layout
                .panels
                .iter()
                .all(|panel| panel.panel.volume_mm3() > 0.)
        );
    }
    let mut empty = request(stocks[0]);
    empty.definition.faces.clear();
    assert!(resolve_wood_box(session.document(), session.parameters(), &empty.definition).is_err());
}
#[test]
fn face_presence_drives_clear_space_and_actual_bounds_separately_from_design_envelope() {
    let (session, stocks) = fixture();
    let mut request = request(stocks[0]);
    let closed = resolve_wood_box(
        session.document(),
        session.parameters(),
        &request.definition,
    )
    .unwrap();
    assert_eq!(closed.clear_mm, [564., 364., 264.]);
    assert_eq!(closed.actual_bounds, [[0., 0., 0.], [600., 400., 300.]]);
    request.definition.faces.remove(&WoodBoxFace::Top);
    request.definition.faces.remove(&WoodBoxFace::Left);
    let open = resolve_wood_box(
        session.document(),
        session.parameters(),
        &request.definition,
    )
    .unwrap();
    assert_eq!(open.clear_mm, [582., 364., 282.]);
    assert_eq!(open.panels.len(), 4);
    request.definition.faces = std::collections::BTreeSet::from([WoodBoxFace::Bottom]);
    let single = resolve_wood_box(
        session.document(),
        session.parameters(),
        &request.definition,
    )
    .unwrap();
    assert_eq!(single.outside_mm, [600., 400., 300.]);
    assert_eq!(single.actual_bounds, [[0., 0., 0.], [600., 400., 18.]]);
    request.definition.stock = stocks[1];
    request.definition.height = length(20.);
    assert!(
        resolve_wood_box(
            session.document(),
            session.parameters(),
            &request.definition
        )
        .is_err()
    );
}
#[test]
fn atomic_face_edits_restock_history_and_file_reopen_preserve_recipe_and_retained_identities() {
    let (mut session, stocks) = fixture();
    let before = session.document().clone();
    let mut request = request(stocks[0]);
    request.definition.faces.remove(&WoodBoxFace::Top);
    let prepared = prepare_wood_box(
        &session,
        session.revision(),
        actor(),
        &WoodBoxChange::Set(Box::new(request.clone())),
    )
    .unwrap();
    assert_eq!(prepared.panels.len(), 5);
    assert_eq!(session.document(), &before);
    session.commit(prepared.model.prepared).unwrap();
    let created = session.document().clone();
    let ids = request.definition.panels.clone();
    request.definition.width = length(700.);
    request.definition.stock = stocks[1];
    request.definition.faces.remove(&WoodBoxFace::Left);
    request.definition.faces.insert(WoodBoxFace::Top);
    let edited = apply(&mut session, &request);
    assert_eq!(edited.panels.len(), 5);
    assert_eq!(edited.removed_parts, vec![ids[&WoodBoxFace::Left].body]);
    assert!(
        session
            .document()
            .get(ids[&WoodBoxFace::Left].body)
            .is_none()
    );
    assert!(
        session
            .document()
            .get(ids[&WoodBoxFace::Top].body)
            .is_some()
    );
    for (face, output) in edited.panels {
        assert_eq!(output.body, ids[&face].body);
        assert_eq!(output.feature, ids[&face].feature);
        assert_eq!(
            resolve_panel(
                session.document(),
                session.parameters(),
                &read_panel(session.document(), output.feature).unwrap()
            )
            .unwrap()
            .stock,
            stocks[1]
        );
    }
    assert_eq!(
        read_wood_box(session.document(), request.id).unwrap(),
        request.definition
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &created);
    session.redo(session.revision(), actor()).unwrap();
    let file = ProjectFile::new(session.document().clone()).unwrap();
    assert!(file.capabilities().contains(WOOD_BOX_CAPABILITY));
    assert!(file.capabilities().contains(RECIPE_PANEL_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(
        read_wood_box(&reopened.document, request.id).unwrap(),
        request.definition
    );
}
#[test]
fn downstream_operations_block_face_omission_without_orphaning_other_panels() {
    let (mut session, stocks) = fixture();
    let mut request = request(stocks[0]);
    apply(&mut session, &request);
    let ids = request.definition.panels[&WoodBoxFace::Left].clone();
    let tool = PrimitiveRequest::new(
        "Tool",
        PrimitiveParameters::Box {
            size: [length(5.), length(5.), length(5.)],
            frame: Default::default(),
        },
    );
    let prepared = prepare_primitive(&session, session.revision(), actor(), &tool).unwrap();
    session.commit(prepared.prepared).unwrap();
    let cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: ids.body,
            producer: ids.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: tool.feature,
        }],
    );
    let prepared = prepare_boolean(&session, session.revision(), actor(), &cut).unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = session.document().clone();
    request.definition.faces.remove(&WoodBoxFace::Left);
    let error = prepare_wood_box(
        &session,
        session.revision(),
        actor(),
        &WoodBoxChange::Set(Box::new(request)),
    )
    .expect_err("Referenced face omission must require repair");
    assert!(error.to_string().contains("downstream"));
    assert_eq!(session.document(), &before);
}
