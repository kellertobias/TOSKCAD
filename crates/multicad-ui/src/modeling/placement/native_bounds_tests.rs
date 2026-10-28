use super::*;
use crate::modeling::native_fixture;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{PrimitiveFrame, PrimitiveParameters, PrimitiveRequest},
    parameters::{self, Unit},
};
use multicad_model::Compiler;
use std::{collections::BTreeMap, sync::Arc};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, EntityId, EntityId) {
    let mut s = Session::new(Document::new("Native centered gizmo").unwrap()).unwrap();
    let angle = 0.37_f64;
    let p = PrimitiveRequest::new(
        "Far cylinder",
        PrimitiveParameters::Cylinder {
            radius: model::length(10.),
            height: model::length(12.),
            frame: PrimitiveFrame {
                origin: [
                    model::length(1000.),
                    model::length(2000.),
                    model::length(3000.),
                ],
                u: [angle.cos(), angle.sin(), 0.],
                v: [-angle.sin(), angle.cos(), 0.],
            },
        },
    );
    s.commit(
        model::prepare_primitive(&s, s.revision(), actor(), &p)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let copy = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: p.body,
                producer: p.feature,
            },
        },
        NumericTransform {
            translation: [
                model::length(400.),
                model::length(-100.),
                model::length(20.),
            ],
            axis: [0., 1., 0.],
            angle: model::angle(23.),
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
fn scene(s: &Session) -> Arc<Scene> {
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    native_fixture::scene(s.document(), native.snapshot(), 0).unwrap()
}
fn pivot(panel: &PlacementPanel, s: &Session) -> [f64; 3] {
    let p = parameters::evaluate(s.document()).unwrap();
    panel
        .parameters(s.document())
        .unwrap()
        .pivot
        .map(|d| match d {
            multicad_core::constraint_sketch::Dimension::Literal { value, unit } => {
                value * Unit::parse(&unit).unwrap().scale() / Unit::Millimeter.scale()
            }
            multicad_core::constraint_sketch::Dimension::Expression { expression } => expression
                .evaluate(&p)
                .unwrap()
                .in_unit(Unit::Millimeter)
                .unwrap(),
        })
}
fn close(a: [f64; 3], b: [f64; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        assert!((a - b).abs() < 1e-8, "{a} != {b}");
    }
}
#[test]
fn a_new_native_placement_uses_the_exact_selected_union_center_and_preserves_it_when_retargeted() {
    let (s, part, instance) = fixture();
    let current = scene(&s);
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let part_producer = native.snapshot().body_sources[&part];
    let part_bounds = native.snapshot().shapes[&part_producer].bounds;
    let instance_bounds = native.snapshot().occurrences[&instance].bounds;
    assert_eq!(
        current.native_instance_bounds(part),
        Some([part_bounds.min, part_bounds.max])
    );
    assert_eq!(
        current.native_instance_bounds(instance),
        Some([instance_bounds.min, instance_bounds.max])
    );
    // Curved tessellation does not contain the exact analytic extrema.
    assert_ne!(
        current.instance_bounds(part).unwrap(),
        current.native_instance_bounds(part).unwrap()
    );
    let mut panel = PlacementPanel::new(
        s.document(),
        s.revision(),
        &current,
        &BTreeSet::from([part]),
        PlacementMode::Move,
    )
    .unwrap();
    close(
        pivot(&panel, &s),
        std::array::from_fn(|i| {
            part_bounds.min[i] + (part_bounds.max[i] - part_bounds.min[i]) * 0.5
        }),
    );
    let before = pivot(&panel, &s);
    panel
        .retarget(
            s.document(),
            s.revision(),
            &current,
            &BTreeSet::from([part, instance]),
        )
        .unwrap();
    assert_eq!(pivot(&panel, &s), before);
    let panel = PlacementPanel::new(
        s.document(),
        s.revision(),
        &current,
        &BTreeSet::from([part, instance]),
        PlacementMode::Copy,
    )
    .unwrap();
    close(
        pivot(&panel, &s),
        std::array::from_fn(|i| {
            let min = part_bounds.min[i].min(instance_bounds.min[i]);
            let max = part_bounds.max[i].max(instance_bounds.max[i]);
            min + (max - min) * 0.5
        }),
    );
    let visible = current.visible_subset(&BTreeSet::from([instance])).unwrap();
    assert_eq!(
        visible.native_instance_bounds(part),
        current.native_instance_bounds(part)
    );
    assert_eq!(
        visible.native_instance_bounds(instance),
        current.native_instance_bounds(instance)
    );
}
#[test]
fn editing_a_saved_native_feature_retains_its_explicit_pivot_instead_of_recentering() {
    let (mut s, part, _) = fixture();
    let current = scene(&s);
    let panel = PlacementPanel::new(
        s.document(),
        s.revision(),
        &current,
        &BTreeSet::from([part]),
        PlacementMode::Move,
    )
    .unwrap();
    let control = NumericTransform {
        pivot: [model::length(7.), model::length(8.), model::length(9.)],
        angle: model::angle(13.),
        ..Default::default()
    };
    let command = panel
        .request(s.document(), true)
        .unwrap()
        .with_parameters(control.clone());
    let prepared = command.prepare(&s, actor()).unwrap();
    let feature = prepared.outputs[0].feature;
    s.commit(prepared.prepared).unwrap();
    scene(&s);
    let edit = PlacementPanel::existing(s.document(), feature).unwrap();
    assert_eq!(edit.parameters(s.document()).unwrap(), control);
    close(pivot(&edit, &s), [7., 8., 9.]);
}
#[test]
fn native_bounds_reject_wrong_hashes_missing_occurrences_nonfinite_reversed_and_unknown_extents() {
    let (s, _, instance) = fixture();
    let current = scene(&s);
    let source = current.model_evaluation().unwrap().source_hash;
    let make = || {
        Scene::new(
            current.stamp(),
            current.geometries().values().cloned().collect(),
            current.reference_instances().to_vec(),
        )
        .unwrap()
        .with_model_evaluation(current.model_evaluation().unwrap().clone())
        .unwrap()
    };
    let bounds = current
        .reference_instances()
        .iter()
        .map(|i| (i.id, current.native_instance_bounds(i.id).unwrap()))
        .collect::<BTreeMap<_, _>>();
    let wrong = ContentHash::document(&Document::new("Wrong source").unwrap()).unwrap();
    assert!(make().with_native_bounds(wrong, bounds.clone()).is_err());
    let mut missing = bounds.clone();
    missing.remove(&instance);
    assert!(make().with_native_bounds(source, missing).is_err());
    for bad in [
        [[f64::NAN, 0., 0.], [1.; 3]],
        [[0.; 3], [-1.; 3]],
        [[0.; 3], [f64::INFINITY, 1., 1.]],
    ] {
        let mut value = bounds.clone();
        value.insert(instance, bad);
        assert!(make().with_native_bounds(source, value).is_err());
    }
    let mut unknown = bounds.clone();
    unknown.insert(EntityId::new(), [[0.; 3], [1.; 3]]);
    assert!(make().with_native_bounds(source, unknown).is_err());
    let incomplete = make()
        .with_native_bounds(source, bounds)
        .unwrap()
        .with_source_scope(vec![instance])
        .unwrap();
    assert!(incomplete.native_instance_bounds(instance).is_none());
}
