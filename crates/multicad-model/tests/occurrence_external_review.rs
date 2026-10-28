#[path = "../../multicad-app/examples/occurrence_reference_review.rs"]
mod review;
use multicad_core::{constraint_sketch::CurveKind, project_file::ContentHash, sketch_external::*};
use multicad_model::Compiler;
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn endpoints(geometry: &ExternalGeometry) -> [[f64; 2]; 2] {
    let ExternalGeometry::Line { a_mm, b_mm } = geometry else {
        panic!("Expected native line")
    };
    let mut p = [*a_mm, *b_mm];
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    p
}
#[test]
fn nested_occurrences_cancel_common_pose_and_world_projection_distinguishes_two_roots() {
    let mut f = review::seed_nested().unwrap();
    let before = review::native(&f.session).unwrap();
    assert_eq!(before.snapshot().leaf_occurrences.len(), 4);
    assert!(before.solid(f.source.body).is_some());
    assert_ne!(f.member.instance, f.nested.instance);
    let compiler = Compiler::default();
    let c = compiler
        .query_external_catalog(f.session.document(), f.session.revision(), &f.query, None)
        .unwrap();
    assert_eq!(endpoints(&c.results[0].geometry), [[20., 20.], [100., 20.]]);
    let expected = before
        .leaf_snapshot(&f.query.sketch_context.as_ref().unwrap().path)
        .unwrap()
        .pose;
    for (a, b) in c
        .sketch_world_pose
        .translation_mm
        .into_iter()
        .zip(expected.translation_mm)
    {
        near(a, b);
    }
    let mut worlds = vec![];
    for root in &f.roots {
        let mut q = f.query.clone();
        q.reference = multicad_core::document::EntityId::new();
        q.sketch = f.world_sketch;
        q.source.path.root = root.instance;
        q.sketch_context = None;
        q.method = QueryMethod::Project;
        let c = compiler
            .query_external_catalog(f.session.document(), f.session.revision(), &q, None)
            .unwrap();
        assert_eq!(c.sketch_world_pose, Default::default());
        let p = endpoints(&c.results[0].geometry);
        let pose = before.leaf_snapshot(&q.source.path).unwrap().pose;
        let mut expected =
            [pose.point([0., 0., 0.]), pose.point([80., 0., 0.])].map(|p| [p[0], p[1]]);
        expected.sort_by(|a, b| a[0].total_cmp(&b[0]));
        for (a, b) in p.into_iter().zip(expected) {
            near(a[0], b[0]);
            near(a[1], b[1]);
        }
        worlds.push(p);
    }
    assert_ne!(worlds[0], worlds[1]);
    let request = review::capture(&mut f.session, &f.query, 0, Behavior::ReferenceOnly).unwrap();
    let canonical = f.session.document().get(f.sketch).unwrap().clone();
    let base = review::native(&f.session).unwrap();
    let old_report = base.snapshot().references.clone();
    review::parameter(&mut f.session, f.offset, "35").unwrap();
    let current = review::native(&f.session).unwrap();
    assert_eq!(f.session.document().get(f.sketch), Some(&canonical));
    assert_eq!(
        endpoints(
            &current.snapshot().references.external_results[&request.query.reference].geometry
        ),
        [[35., 20.], [115., 20.]]
    );
    near(
        current.snapshot().shapes[&f.receiver.feature].volume_mm3,
        57600.,
    );
    assert!(
        multicad_solver::evaluate_document_sketch(
            f.session.document(),
            f.session.revision(),
            f.sketch,
            Some(&old_report)
        )
        .is_err()
    );
    let reopened = review::reopen(&review::archive(f.session.document()).unwrap()).unwrap();
    let m = review::native(&reopened).unwrap();
    assert_eq!(
        endpoints(&m.snapshot().references.external_results[&request.query.reference].geometry),
        [[35., 20.], [115., 20.]]
    );
    assert_eq!(
        read_occurrence(reopened.document(), request.query.reference)
            .unwrap()
            .query
            .source
            .path,
        f.query.source.path
    );
}
#[test]
fn finite_curved_face_section_drives_a_real_closed_solid_and_empty_section_cannot_reuse_guesses() {
    let mut f = review::seed_section().unwrap();
    let c = Compiler::default()
        .query_external_catalog(f.session.document(), f.session.revision(), &f.query, None)
        .unwrap();
    assert_eq!(c.results.len(), 1);
    assert!(
        matches!(c.results[0].geometry,ExternalGeometry::Circle{radius_mm,..}if(radius_mm-10.).abs()<1e-7)
    );
    review::capture_section(&mut f).unwrap();
    let output = f.output.as_ref().unwrap();
    let m = review::native(&f.session).unwrap();
    near(
        m.snapshot().shapes[&output.feature].volume_mm3,
        std::f64::consts::PI * 100. * 5.,
    );
    let source =
        multicad_core::model::read_primitive(f.session.document(), f.source.feature).unwrap();
    let mut parameters = source.parameters;
    let multicad_core::model::PrimitiveParameters::Cylinder { ref mut height, .. } = parameters
    else {
        panic!()
    };
    *height = multicad_core::model::length(5.);
    let p = multicad_core::model::prepare_primitive_edit(
        &f.session,
        f.session.revision(),
        review::actor(),
        f.source.feature,
        &parameters,
    )
    .unwrap();
    f.session.commit(p.prepared).unwrap();
    let saved = ContentHash::document(f.session.document()).unwrap();
    assert!(review::native(&f.session).is_err());
    let c = Compiler::default()
        .query_external_catalog(f.session.document(), f.session.revision(), &f.query, None)
        .unwrap();
    assert!(c.results.is_empty());
    assert_eq!(ContentHash::document(f.session.document()).unwrap(), saved);
}
#[test]
fn explicit_point_branch_tracks_source_radius_and_broken_dependent_solid_repairs_without_new_ids() {
    let mut f = review::seed_circle().unwrap();
    let c = Compiler::default()
        .query_external_catalog(f.session.document(), f.session.revision(), &f.query, None)
        .unwrap();
    assert_eq!(c.results.len(), 2);
    assert!(c.results.iter().all(|c| c.selector.branch.is_some()));
    review::capture_circle(&mut f).unwrap();
    let request = f.request.clone().unwrap();
    let output = f.output.clone().unwrap();
    near(
        review::native(&f.session).unwrap().snapshot().shapes[&output.feature].volume_mm3,
        1000.,
    );
    let canonical = f.session.document().get(f.sketch).unwrap().clone();
    review::parameter(&mut f.session, f.radius, "14").unwrap();
    near(
        review::native(&f.session).unwrap().snapshot().shapes[&output.feature].volume_mm3,
        1400.,
    );
    assert_eq!(f.session.document().get(f.sketch), Some(&canonical));
    review::break_circle(&mut f).unwrap();
    let broken = f.session.document().clone();
    assert_eq!(
        read_occurrence(&broken, request.query.reference)
            .unwrap()
            .output,
        request.output
    );
    let reopened = review::reopen(&review::archive(&broken).unwrap()).unwrap();
    assert_eq!(reopened.document(), &broken);
    assert!(review::native(&reopened).is_err());
    review::repair_circle(&mut f).unwrap();
    let repaired = f.session.document().clone();
    let r = f.request.as_ref().unwrap();
    assert_eq!(r.output, request.output);
    assert_eq!(r.query.reference, request.query.reference);
    assert_eq!(r.query.source.component, f.replacement.body);
    near(
        review::native(&f.session).unwrap().snapshot().shapes[&output.feature].volume_mm3,
        3800.,
    );
    f.session
        .undo(f.session.revision(), review::actor())
        .unwrap();
    assert_eq!(f.session.document(), &broken);
    assert!(review::native(&f.session).is_err());
    f.session
        .redo(f.session.revision(), review::actor())
        .unwrap();
    assert_eq!(f.session.document(), &repaired);
    near(
        review::native(&f.session).unwrap().snapshot().shapes[&output.feature].volume_mm3,
        3800.,
    );
}
#[test]
fn current_native_spline_basis_regenerates_without_rewriting_authored_output_or_pole_ids() {
    let mut f = review::seed_spline().unwrap();
    let request = review::capture(&mut f.session, &f.query, 0, Behavior::Derived).unwrap();
    f.request = Some(request.clone());
    let mut receiver = review::read_source(&f.session, f.sketch).unwrap();
    receiver.add_line(
        *request.output.points.last().unwrap(),
        request.output.points[0],
    );
    let edits = receiver.replace(f.session.document(), f.sketch).unwrap();
    review::apply(&mut f.session, "Close native referenced spline", edits).unwrap();
    let output = review::extrude(&mut f.session, f.sketch, 3.).unwrap();
    let original = f.session.document().get(f.sketch).unwrap().clone();
    let before = review::native(&f.session).unwrap();
    near(
        before.snapshot().shapes[&output.feature].volume_mm3,
        before.snapshot().shapes[&f.source.feature].volume_mm3 / 8. * 3.,
    );
    let mut source = review::read_source(&f.session, f.source_sketch).unwrap();
    let CurveKind::BSpline { ref mut basis, .. } = source
        .sketch
        .curves
        .iter_mut()
        .find(|c| matches!(c.kind, CurveKind::BSpline { .. }))
        .unwrap()
        .kind
    else {
        panic!()
    };
    basis.weights = Some(vec![1., 0.8, 1.]);
    let edits = source
        .replace(f.session.document(), f.source_sketch)
        .unwrap();
    review::apply(&mut f.session, "Edit rational source weights", edits).unwrap();
    let after = review::native(&f.session).unwrap();
    assert_eq!(f.session.document().get(f.sketch), Some(&original));
    near(
        after.snapshot().shapes[&output.feature].volume_mm3,
        after.snapshot().shapes[&f.source.feature].volume_mm3 / 8. * 3.,
    );
    assert!(
        (after.snapshot().shapes[&output.feature].volume_mm3
            - before.snapshot().shapes[&output.feature].volume_mm3)
            .abs()
            > 1e-3
    );
    let solved = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        f.sketch,
        Some(&after.snapshot().references),
    )
    .unwrap();
    let CurveKind::BSpline {
        ref poles,
        ref basis,
    } = solved
        .solved()
        .unwrap()
        .curves
        .iter()
        .find(|c| Some(c.id) == request.output.curve)
        .unwrap()
        .kind
    else {
        panic!()
    };
    assert_eq!(*poles, request.output.points);
    let weights = basis.weights.as_ref().unwrap();
    near(weights[1] / weights[0], 0.8);
    let reopened = review::reopen(&review::archive(f.session.document()).unwrap()).unwrap();
    let again = review::native(&reopened).unwrap();
    near(
        again.snapshot().shapes[&output.feature].volume_mm3,
        after.snapshot().shapes[&output.feature].volume_mm3,
    );
}
#[test]
fn editable_native_review_generator_roundtrips_twelve_projects_and_refuses_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("occurrence-review");
    let manifest = review::generate(&dir).unwrap();
    assert_eq!(manifest["projects"].as_array().unwrap().len(), 12);
    let broken = manifest["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["expected_broken"] == true)
        .count();
    assert_eq!(broken, 1);
    let bytes = std::fs::read(dir.join("manifest.json")).unwrap();
    assert!(review::generate(&dir).is_err());
    assert_eq!(std::fs::read(dir.join("manifest.json")).unwrap(), bytes);
}
