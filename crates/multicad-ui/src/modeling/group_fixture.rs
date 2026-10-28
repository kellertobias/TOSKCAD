//! Test-only canonical fixtures and scripted named-mesh receipts. Native model
//! shape/volume correctness is checked separately in multicad-model.
use crate::viewport::{Geometry, Instance, Scene, SceneStamp, Transform};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::source::Source,
    document::{Document, EntityData, EntityId},
    model::{
        self, BodyInput, ExtrudeBooleanRequest, ExtrudeCapturedScope, ExtrudeGroupDraft,
        ExtrudeRequest, Operation, PrimitiveParameters, PrimitiveRequest,
    },
    project_file::ContentHash,
    sketch::Datum,
    sketch_regions,
};
use multicad_kernel::{GeometryPolicy, MeshOptions, Solid};
use std::{collections::BTreeSet, sync::Arc};
pub(super) struct Fixture {
    pub session: Session,
    pub request: ExtrudeBooleanRequest,
    pub group: EntityId,
    pub targets: Vec<BodyInput>,
}
pub(super) fn fixture() -> Fixture {
    let (sketch, edits) = Source::create(Datum::Xy, "Shared operation profiles").unwrap();
    let document = Document::new("Captured operation editor")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    source.add_rectangle([5., 5.], [15., 15.]).unwrap();
    source.add_rectangle([20., 5.], [30., 15.]).unwrap();
    let regions = sketch_regions::extract(&source)
        .regions
        .iter()
        .map(|region| region.intent())
        .collect();
    let mut session = Session::new(
        document
            .edited(&source.replace(&document, sketch).unwrap())
            .unwrap(),
    )
    .unwrap();
    let mut targets = vec![];
    for name in ["Cabinet left", "Cabinet right", "Extra target"] {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [model::length(80.), model::length(60.), model::length(20.)],
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &request,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        targets.push(BodyInput {
            body: request.body,
            producer: request.feature,
        });
    }
    let mut request = ExtrudeBooleanRequest::new(
        ExtrudeRequest::new(sketch, regions, Default::default()),
        Operation::Cut,
        targets[..2].to_vec(),
        false,
    );
    request.group = Some(ExtrudeGroupDraft::new(
        "Cabinet machining",
        Some(ExtrudeCapturedScope {
            component: None,
            include: vec![targets[1].body],
            exclude: vec![],
        }),
    ));
    let group = request.group.as_ref().unwrap().id;
    let prepared = model::prepare_extrude_boolean(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &request,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    Fixture {
        session,
        request,
        group,
        targets,
    }
}
pub(super) fn consumed(fixture: &Fixture) -> BTreeSet<EntityId> {
    fixture
        .request
        .extrusion
        .outputs
        .iter()
        .map(|output| output.body)
        .collect()
}
pub(super) fn scene(
    document: &Document,
    stamp: SceneStamp,
    consumed: BTreeSet<EntityId>,
    omit: Option<EntityId>,
) -> Arc<Scene> {
    let mut geometries = vec![];
    for entity in document
        .entities()
        .filter(|entity| matches!(entity.data, EntityData::Feature { .. }))
    {
        if Some(entity.id) == omit {
            continue;
        }
        let solid =
            Solid::box_named([30., 20., 10.], entity.id, GeometryPolicy::default()).unwrap();
        let mesh = solid.mesh(MeshOptions::default()).unwrap();
        geometries.push(Arc::new(
            Geometry::new(
                entity.id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                solid
                    .topology_snapshot(entity.id, stamp.revision)
                    .unwrap()
                    .elements,
            )
            .unwrap(),
        ));
    }
    let instances = document
        .entities()
        .filter_map(|entity| match &entity.data {
            EntityData::Part { features, .. } if !consumed.contains(&entity.id) => Some(Instance {
                id: entity.id,
                geometry: *features.last().unwrap(),
                transform: Transform::default(),
                color: [150, 180, 200, 255],
            }),
            _ => None,
        })
        .collect();
    Arc::new(
        Scene::new(stamp, geometries, instances)
            .unwrap()
            .with_model_evaluation(Arc::new(model::ModelEvaluationInfo {
                source_hash: ContentHash::document(document).unwrap(),
                consumed_bodies: consumed,
                cut_targets: vec![],
            }))
            .unwrap(),
    )
}
