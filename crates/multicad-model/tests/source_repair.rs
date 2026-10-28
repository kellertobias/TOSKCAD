use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{ConstraintKind, CurveKind, source::Source},
    document::{Document, EntityData, Mutation},
    model::{self, ExtrudeRequest, PrimitiveParameters, PrimitiveRequest},
    references::{AnalyticGeometry, GeometryIntent},
    sketch::Datum,
    sketch_external::{Behavior, CaptureInput, CaptureRequest, Method, OutputIds},
};
use multicad_model::{Compiler, DrawingProviders, EvaluationScope};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn parameters(width: f64, height: f64) -> PrimitiveParameters {
    PrimitiveParameters::Box {
        size: [
            model::length(width),
            model::length(20.),
            model::length(height),
        ],
        frame: Default::default(),
    }
}
#[test]
fn fresh_source_scope_repairs_a_consumed_sketch_then_full_model_rebuilds_atomically() {
    let (sketch, mut edits) =
        Source::create(Datum::Xy, "Broken consumed wall intersection").unwrap();
    let plane = edits
        .iter_mut()
        .find_map(|edit| {
            if let Mutation::Insert(entity) = edit
                && let EntityData::Plane { origin_mm, .. } = &mut entity.data
            {
                origin_mm[2] = 5.;
                Some(entity.id)
            } else {
                None
            }
        })
        .unwrap();
    let mut session = Session::new(
        Document::new("Checked partial-source repair")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let original = PrimitiveRequest::new("Original wall", parameters(30., 10.));
    let replacement = PrimitiveRequest::new("Replacement wall", parameters(40., 10.));
    for request in [&original, &replacement] {
        let edit =
            model::prepare_primitive(&session, session.revision(), actor(), request).unwrap();
        session.commit(edit.prepared).unwrap();
    }
    let compiler = Compiler::default();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let map = native
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| (*id, s.topology.clone()))
        .collect();
    let topo = &native.snapshot().shapes[&original.feature].topology;
    let key=topo.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm:a,b_mm:b}) if (a[0]-30.).abs()<1e-7&&(b[0]-30.).abs()<1e-7&&a[1].abs()<1e-7&&b[1].abs()<1e-7&&(a[2]-b[2]).abs()>9.)).unwrap().key;
    let capture = CaptureRequest::new(
        session.document(),
        session.revision(),
        CaptureInput {
            sketch,
            source: original.feature,
            intent: GeometryIntent::capture(topo, key).unwrap(),
            method: Method::Intersect,
            behavior: Behavior::ReferenceOnly,
        },
        &map,
    )
    .unwrap();
    let reference = multicad_solver::prepare_external_reference(
        &session,
        session.revision(),
        actor(),
        &capture,
        &map,
    )
    .unwrap();
    session.commit(reference.edit.preview.unwrap()).unwrap();
    let EntityData::Sketch { definition, .. } = &session.document().get(sketch).unwrap().data
    else {
        panic!()
    };
    let mut own = Source::from_definition(definition).unwrap();
    let OutputIds::Point { point, .. } = capture.output else {
        panic!()
    };
    let line = own.add_rectangle([30., 0.], [35., 7.]).unwrap();
    let CurveKind::Line { start, .. } = own
        .sketch
        .curves
        .iter()
        .find(|c| c.id == line)
        .unwrap()
        .kind
    else {
        panic!()
    };
    let side = own.sketch.curves[1].id;
    for kind in [
        ConstraintKind::Coincident { a: point, b: start },
        ConstraintKind::Length {
            line,
            dimension: model::length(5.),
        },
        ConstraintKind::Length {
            line: side,
            dimension: model::length(7.),
        },
    ] {
        own.add_constraint(kind);
    }
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let edit = multicad_solver::prepare_canonical_edit(
        &session,
        session.revision(),
        actor(),
        sketch,
        own,
        Some(&native.snapshot().references),
    )
    .unwrap();
    let region = edit.semantic.as_ref().unwrap().regions().unwrap().regions[0].intent();
    session.commit(edit.preview.unwrap()).unwrap();
    let extrusion = ExtrudeRequest::new(
        sketch,
        vec![region],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: model::length(2.),
                },
            },
            ..Default::default()
        },
    );
    let edit = model::prepare_extrude(&session, session.revision(), actor(), &extrusion).unwrap();
    session.commit(edit.prepared).unwrap();
    let edit = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        original.feature,
        &parameters(30., 2.),
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let broken = session.document().clone();
    assert!(compiler.compile(&broken, session.revision()).is_err());
    let roots = [replacement.feature, plane];
    let partial = compiler
        .compile_sources(&broken, session.revision(), &roots)
        .unwrap();
    assert!(matches!(
        partial.snapshot().scope,
        EvaluationScope::Sources(_)
    ));
    assert_eq!(
        partial.snapshot().stamp.source_hash,
        multicad_core::project_file::ContentHash::document(&broken).unwrap()
    );
    assert!(partial.snapshot().shapes.contains_key(&replacement.feature));
    assert!(
        !partial
            .snapshot()
            .shapes
            .contains_key(&extrusion.outputs[0].feature)
    );
    assert!(
        partial
            .drawings(&broken, session.revision(), &DrawingProviders::default())
            .unwrap_err()
            .to_string()
            .contains("source-only")
    );
    assert!(
        partial
            .cut_targets(
                &broken,
                session.revision(),
                &model::CutTargetQuery {
                    tools: vec![replacement.feature],
                    component: None,
                    include: vec![],
                    exclude: vec![]
                },
                None
            )
            .unwrap_err()
            .to_string()
            .contains("source-only")
    );
    let topo = &partial.snapshot().shapes[&replacement.feature].topology;
    let key=topo.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm:a,b_mm:b}) if (a[0]-40.).abs()<1e-7&&(b[0]-40.).abs()<1e-7&&a[1].abs()<1e-7&&b[1].abs()<1e-7&&(a[2]-b[2]).abs()>9.)).unwrap().key;
    let map = partial
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| (*id, s.topology.clone()))
        .collect();
    // Missing geometry-source links remain editable source. The recovery query
    // must not evaluate the removed producer or reuse its previous topology.
    let mut missing = Session::new(
        broken
            .edited(&[
                Mutation::Remove(original.body),
                Mutation::Remove(original.feature),
            ])
            .unwrap(),
    )
    .unwrap();
    assert!(
        compiler
            .compile(missing.document(), missing.revision())
            .is_err()
    );
    let fresh = compiler
        .compile_sources(missing.document(), missing.revision(), &roots)
        .unwrap();
    let fresh_maps = fresh
        .snapshot()
        .shapes
        .iter()
        .map(|(id, shape)| (*id, shape.topology.clone()))
        .collect();
    let fixed = multicad_solver::prepare_external_repair(
        &missing,
        missing.revision(),
        actor(),
        &multicad_solver::ExternalRepairRequest {
            sketch,
            reference: capture.reference,
            source: replacement.feature,
            selected: key,
        },
        &fresh_maps,
    )
    .unwrap();
    let checked = fixed.preview.unwrap();
    let rebuilt = compiler
        .compile(checked.document(), missing.revision())
        .unwrap();
    assert!(
        (rebuilt.snapshot().shapes[&extrusion.outputs[0].feature].volume_mm3 - 70.).abs() < 1e-6
    );
    missing.commit(checked).unwrap();
    assert_eq!(
        multicad_core::sketch_external::read(missing.document(), capture.reference)
            .unwrap()
            .source,
        replacement.feature
    );
    let edit = multicad_solver::prepare_external_repair(
        &session,
        session.revision(),
        actor(),
        &multicad_solver::ExternalRepairRequest {
            sketch,
            reference: capture.reference,
            source: replacement.feature,
            selected: key,
        },
        &map,
    )
    .unwrap();
    let checked = edit.preview.unwrap();
    let full = compiler
        .compile(checked.document(), session.revision())
        .unwrap();
    assert_eq!(full.snapshot().scope, EvaluationScope::Full);
    let shape = &full.snapshot().shapes[&extrusion.outputs[0].feature];
    assert!((shape.volume_mm3 - 70.).abs() < 1e-6);
    assert!((shape.bounds.min[0] - 40.).abs() < 1e-6);
    assert_eq!(session.document(), &broken);
    session.commit(checked).unwrap();
    assert_eq!(
        multicad_core::sketch_external::read(session.document(), capture.reference)
            .unwrap()
            .source,
        replacement.feature
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &broken);
    assert!(
        compiler
            .compile(session.document(), session.revision())
            .is_err()
    );
    session.redo(session.revision(), actor()).unwrap();
    assert!(
        compiler
            .compile(session.document(), session.revision())
            .is_ok()
    );
    assert!(
        compiler
            .compile_sources(session.document(), session.revision(), &[])
            .is_err()
    );
    assert!(
        compiler
            .compile_sources(session.document(), session.revision(), &[plane, plane])
            .is_err()
    );
    assert!(
        compiler
            .compile_sources(session.document(), session.revision(), &[sketch])
            .is_err()
    );
    assert!(
        compiler
            .compile_sources_interruptible(
                session.document(),
                session.revision(),
                &roots,
                Some(&|| true)
            )
            .is_err()
    );
}
