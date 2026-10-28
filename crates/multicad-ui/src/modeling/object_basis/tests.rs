use super::*;
use crate::{
    modeling::native_fixture,
    viewport::{SceneStamp, Transform},
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::source::Source,
    document::Mutation,
    model::{NumericTransform, PrimitiveFrame, PrimitiveRequest},
    sketch::Datum,
    sketch_regions,
};
use multicad_model::Compiler;
use std::sync::Arc;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn scene(s: &Session) -> Arc<Scene> {
    let model = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    native_fixture::scene(s.document(), model.snapshot(), 0).unwrap()
}
fn close(actual: GizmoBasis, expected: [[f64; 3]; 3]) {
    for (a, b) in actual
        .axes
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        assert!((a - b).abs() < 1e-9, "{actual:?} != {expected:?}");
    }
}
fn fixture() -> (Session, EntityId, EntityId) {
    let mut s = Session::new(Document::new("Authored object coordinates").unwrap()).unwrap();
    let p = PrimitiveRequest::new(
        "Oriented stock",
        PrimitiveParameters::Box {
            size: [model::length(2.), model::length(3.), model::length(4.)],
            frame: PrimitiveFrame {
                u: [0., 1., 0.],
                v: [0., 0., 1.],
                ..Default::default()
            },
        },
    );
    s.commit(
        model::prepare_primitive(&s, s.revision(), actor(), &p)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let transform = model::TransformRequest::new(
        model::BodyInput {
            body: p.body,
            producer: p.feature,
        },
        NumericTransform {
            angle: model::angle(90.),
            ..Default::default()
        },
    );
    s.commit(
        model::prepare_transform(&s, s.revision(), actor(), &transform)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let copy = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: model::BodyInput {
                body: p.body,
                producer: transform.feature,
            },
        },
        NumericTransform {
            axis: [1., 0., 0.],
            angle: model::angle(90.),
            translation: [model::length(80.), model::length(0.), model::length(0.)],
            ..Default::default()
        },
    );
    s.commit(
        model::prepare_instance_copies(&s, s.revision(), actor(), std::slice::from_ref(&copy))
            .unwrap()
            .prepared,
    )
    .unwrap();
    (s, p.body, copy.instance)
}
#[test]
fn native_part_uses_its_authored_frame_and_rigid_history_and_instance_uses_its_explicit_pose() {
    let (s, part, instance) = fixture();
    let scene = scene(&s);
    let basis =
        object_gizmo_basis(s.document(), s.revision(), &scene, &BTreeSet::from([part])).unwrap();
    assert_eq!(basis.primary, part);
    close(basis.basis, [[-1., 0., 0.], [0., 0., 1.], [0., 1., 0.]]);
    let basis = object_gizmo_basis(
        s.document(),
        s.revision(),
        &scene,
        &BTreeSet::from([instance]),
    )
    .unwrap();
    assert_eq!(basis.primary, instance);
    close(basis.basis, [[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]]);
    let basis = object_gizmo_basis(
        s.document(),
        s.revision(),
        &scene,
        &BTreeSet::from([instance, part]),
    )
    .unwrap();
    assert_eq!(basis.primary, part.min(instance));
    let expected = object_gizmo_basis(
        s.document(),
        s.revision(),
        &scene,
        &BTreeSet::from([part.min(instance)]),
    )
    .unwrap();
    assert_eq!(basis, expected);
}
#[test]
fn native_extrusion_uses_the_authored_sketch_plane_instead_of_a_face_or_mesh_axis() {
    let (sketch, edits) = Source::create(Datum::Yz, "YZ profile").unwrap();
    let d = Document::new("Extrusion object frame")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&d, sketch).unwrap().0;
    source.add_rectangle([0., 0.], [10., 8.]).unwrap();
    let regions = sketch_regions::extract(&source);
    let d = d.edited(&source.replace(&d, sketch).unwrap()).unwrap();
    let mut s = Session::new(d).unwrap();
    let request = model::ExtrudeRequest::new(
        sketch,
        vec![regions.regions[0].intent()],
        Default::default(),
    );
    let prepared = model::prepare_extrude(&s, s.revision(), actor(), &request).unwrap();
    let part = prepared.outputs[0].body;
    s.commit(prepared.prepared).unwrap();
    let scene = scene(&s);
    close(
        object_gizmo_basis(s.document(), s.revision(), &scene, &BTreeSet::from([part]))
            .unwrap()
            .basis,
        [[0., 1., 0.], [0., 0., 1.], [1., 0., 0.]],
    );
}
#[test]
fn source_stamp_hash_and_published_instance_pose_are_verified_before_using_object_axes() {
    let (s, part, instance) = fixture();
    let current = scene(&s);
    let selected = BTreeSet::from([instance]);
    assert!(object_gizmo_basis(s.document(), s.revision() + 1, &current, &selected).is_err());
    let other = s
        .document()
        .edited(&[Mutation::RenameDocument("Changed source".into())])
        .unwrap();
    assert!(
        object_gizmo_basis(&other, s.revision(), &current, &selected)
            .unwrap_err()
            .contains("source-bound")
    );
    let mut instances = current.reference_instances().to_vec();
    instances
        .iter_mut()
        .find(|i| i.id == instance)
        .unwrap()
        .transform = Transform::default();
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let forged = Scene::new(
        current.stamp(),
        current.geometries().values().cloned().collect(),
        instances,
    )
    .unwrap()
    .with_reference_aliases(native.snapshot().body_sources.clone())
    .unwrap()
    .with_model_evaluation(current.model_evaluation().unwrap().clone())
    .unwrap();
    assert!(object_gizmo_basis(s.document(), s.revision(), &forged, &selected).is_err());
    let no_info = Scene::new(
        SceneStamp {
            generation: 99,
            ..current.stamp()
        },
        current.geometries().values().cloned().collect(),
        current.reference_instances().to_vec(),
    )
    .unwrap();
    assert!(
        object_gizmo_basis(
            s.document(),
            s.revision(),
            &no_info,
            &BTreeSet::from([part])
        )
        .is_err()
    );
}
