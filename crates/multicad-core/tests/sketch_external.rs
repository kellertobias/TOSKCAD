//! Pure analytic and source-contract tests. Actual B-rep propagation belongs to
//! the multicad-model native fixture, not these synthetic topology descriptors.
use multicad_core::{
    constraint_sketch::{Constraint, ConstraintKind, source::Source},
    document::{Definition, Document, Entity, EntityData, EntityId, Mutation},
    references::{
        self, AnalyticGeometry, GeometryIntent, Origin, TopologyElement, TopologyKey, TopologyKind,
        TopologySnapshot,
    },
    sketch::Datum,
    sketch_external::{self, Behavior, CaptureInput, CaptureRequest, LocalGeometry, Method},
};
use std::collections::{BTreeMap, BTreeSet};
fn fixture() -> (
    Document,
    EntityId,
    EntityId,
    BTreeMap<EntityId, TopologySnapshot>,
    GeometryIntent,
) {
    let (sketch, edits) = Source::create(Datum::Xy, "Reference sketch").unwrap();
    let mut document = Document::new("Exact analytic contracts")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let source = Entity::new(
        "Pure analytic source fixture",
        EntityData::Feature {
            definition: Definition {
                type_id: "test.analytic".into(),
                schema_version: 1,
                data: serde_json::json!({"length":30}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    let owner = source.id;
    document.insert(source).unwrap();
    let element = TopologyElement {
        key: TopologyKey {
            kind: TopologyKind::Edge,
            ordinal: 0,
        },
        origins: BTreeSet::from([Origin {
            source: owner,
            role: "test.edge".into(),
        }]),
        boundary: BTreeSet::new(),
        frames: vec![],
        geometry: Some(AnalyticGeometry::Line {
            a_mm: [0., 0., 0.],
            b_mm: [30., 0., 0.],
        }),
    };
    let snapshot = TopologySnapshot {
        owner,
        revision: 7,
        elements: vec![element],
    };
    let intent = GeometryIntent::capture(&snapshot, snapshot.elements[0].key).unwrap();
    (
        document,
        sketch,
        owner,
        BTreeMap::from([(owner, snapshot)]),
        intent,
    )
}
fn captured() -> (
    Document,
    EntityId,
    EntityId,
    BTreeMap<EntityId, TopologySnapshot>,
) {
    let (document, sketch, owner, map, intent) = fixture();
    let request = CaptureRequest::new(
        &document,
        7,
        CaptureInput {
            sketch,
            source: owner,
            intent,
            method: Method::Coplanar,
            behavior: Behavior::ReferenceOnly,
        },
        &map,
    )
    .unwrap();
    let candidate = document
        .edited(&sketch_external::capture_edits(&document, 7, &request, &map).unwrap())
        .unwrap();
    (candidate, sketch, request.reference, map)
}
#[test]
fn exact_projection_and_finite_intersection_cover_supported_families_and_reject_guesses() {
    let frame = Datum::Xy.frame();
    assert_eq!(
        sketch_external::project(
            AnalyticGeometry::Line {
                a_mm: [1., 2., -10.],
                b_mm: [1., 2., 10.]
            },
            frame,
            Method::Intersect
        )
        .unwrap(),
        LocalGeometry::Point([1., 2.])
    );
    for geometry in [
        AnalyticGeometry::Line {
            a_mm: [0., 0., 1.],
            b_mm: [10., 0., 2.],
        },
        AnalyticGeometry::Line {
            a_mm: [0., 0., 0.],
            b_mm: [10., 0., 0.],
        },
    ] {
        assert!(sketch_external::project(geometry, frame, Method::Intersect).is_err());
    }
    assert!(
        sketch_external::project(
            AnalyticGeometry::Point {
                position_mm: [0., 0., 2.]
            },
            frame,
            Method::Coplanar
        )
        .is_err()
    );
    let reference = references::ReferenceFrame {
        origin_mm: [2., 3., 0.],
        u: [1., 0., 0.],
        v: [0., -1., 0.],
    };
    assert_eq!(
        sketch_external::project(
            AnalyticGeometry::CircularArc {
                frame: reference,
                radius_mm: 5.,
                start_rad: 0.,
                sweep_rad: std::f64::consts::TAU
            },
            frame,
            Method::Coplanar
        )
        .unwrap(),
        LocalGeometry::Circle {
            center: [2., 3.],
            radius: 5.
        }
    );
    let arc = sketch_external::project(
        AnalyticGeometry::CircularArc {
            frame: reference,
            radius_mm: 5.,
            start_rad: 0.,
            sweep_rad: std::f64::consts::FRAC_PI_2,
        },
        frame,
        Method::Coplanar,
    )
    .unwrap();
    assert!(matches!(
        arc,
        LocalGeometry::Arc {
            clockwise: true,
            ..
        }
    ));
    assert!(
        sketch_external::project(
            AnalyticGeometry::Plane { frame: reference },
            frame,
            Method::Intersect
        )
        .is_err()
    );
}
#[test]
fn dependency_receipt_rejects_same_revision_upstream_edits_but_allows_consumer_constraint_edits() {
    let (document, sketch, reference, map) = captured();
    let report = references::inspect(&document, 7, &map);
    let binding = sketch_external::read(&document, reference).unwrap();
    assert!(
        sketch_external::resolved_geometry(&document, 7, &binding, Datum::Xy.frame(), &report)
            .is_ok()
    );
    let mut source = Source::from_definition(match &document.get(sketch).unwrap().data {
        EntityData::Sketch { definition, .. } => definition,
        _ => panic!(),
    })
    .unwrap();
    source.add_point([40., 20.]);
    let consumer = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    assert!(
        sketch_external::resolved_geometry(&consumer, 7, &binding, Datum::Xy.frame(), &report)
            .is_ok()
    );
    let mut changed = document.get(binding.source).unwrap().clone();
    let EntityData::Feature { definition, .. } = &mut changed.data else {
        panic!()
    };
    definition.data = serde_json::json!({"length":40});
    let upstream = document.edited(&[Mutation::Replace(changed)]).unwrap();
    assert!(
        sketch_external::resolved_geometry(&upstream, 7, &binding, Datum::Xy.frame(), &report)
            .unwrap_err()
            .to_string()
            .contains("source changed")
    );
    let mut legacy = report.clone();
    legacy.entries[0].dependency_hash = None;
    assert!(
        sketch_external::resolved_geometry(&document, 7, &binding, Datum::Xy.frame(), &legacy)
            .is_err()
    );
}
#[test]
fn protected_outputs_locks_and_ownership_cannot_be_removed_or_rewritten_by_generic_source_edits() {
    let (document, sketch, reference, _) = captured();
    let binding = sketch_external::read(&document, reference).unwrap();
    let EntityData::Sketch { definition, .. } = &document.get(sketch).unwrap().data else {
        panic!()
    };
    assert_eq!(definition.schema_version, 4);
    assert!(Source::read(&document, sketch).is_err());
    let source = Source::from_definition(definition).unwrap();
    let mut unlinked = document.get(sketch).unwrap().clone();
    let EntityData::Sketch { inputs, .. } = &mut unlinked.data else {
        panic!()
    };
    inputs.retain(|link| link.target != reference);
    assert!(
        document.edited(&[Mutation::Replace(unlinked)]).is_err(),
        "Removing only the owner link must not silently bake its guesses"
    );
    let point = binding.definition.output.points()[0];
    let mut moved = source.clone();
    moved
        .sketch
        .points
        .iter_mut()
        .find(|p| p.id == point)
        .unwrap()
        .position_mm[0] += 2.;
    assert!(moved.replace(&document, sketch).is_err());
    let mut locked = source.clone();
    locked.sketch.constraints.push(Constraint {
        id: binding.definition.output.locks()[0],
        reference: false,
        kind: ConstraintKind::Fix { point },
    });
    let mut entity = document.get(sketch).unwrap().clone();
    let EntityData::Sketch { definition, .. } = &mut entity.data else {
        panic!()
    };
    definition.data = serde_json::to_value(locked).unwrap();
    assert!(document.edited(&[Mutation::Replace(entity)]).is_err());
    let removed = document
        .edited(&sketch_external::remove_edits(&document, sketch, reference).unwrap())
        .unwrap();
    assert!(removed.get(reference).is_none());
    let EntityData::Sketch { definition, .. } = &removed.get(sketch).unwrap().data else {
        panic!()
    };
    assert_eq!(definition.schema_version, 4);
    assert!(
        Source::from_definition(definition)
            .unwrap()
            .sketch
            .points
            .is_empty()
    );
    let missing = document
        .edited(&[Mutation::Remove(binding.source)])
        .unwrap();
    assert!(missing.validate().is_ok());
    assert!(!missing.reference_diagnostics().is_empty());
}
#[test]
fn external_evaluation_rejects_an_unreported_attached_plane_and_replaces_arbitrary_solver_seeds() {
    let (document, sketch, reference, map) = captured();
    let report = references::inspect(&document, 7, &map);
    let binding = sketch_external::read(&document, reference).unwrap();
    let EntityData::Sketch {
        definition, plane, ..
    } = &document.get(sketch).unwrap().data
    else {
        panic!()
    };
    let plane = *plane;
    let mut numerical = Source::from_definition(definition).unwrap().sketch;
    for point in &mut numerical.points {
        point.position_mm = [999., 999.];
    }
    let locks = sketch_external::apply_to_solver(
        &document,
        7,
        sketch,
        Datum::Xy.frame(),
        Some(&report),
        &mut numerical,
    )
    .unwrap();
    assert_eq!(locks.len(), 2);
    assert!(
        numerical
            .points
            .iter()
            .all(|point| point.position_mm[1] == 0.)
    );
    let mut plane_entity = document.get(plane).unwrap().clone();
    let EntityData::Plane { anchor, .. } = &mut plane_entity.data else {
        panic!()
    };
    let mut input = multicad_core::document::Link::new(binding.source, "attachment");
    input.geometry = Some(binding.intent.clone());
    *anchor = Some(input);
    let changed = document.edited(&[Mutation::Replace(plane_entity)]).unwrap();
    assert!(
        sketch_external::apply_to_solver(
            &changed,
            7,
            sketch,
            Datum::Xy.frame(),
            Some(&report),
            &mut numerical
        )
        .is_err()
    );
}
