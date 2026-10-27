//! A native exact solid follows checked dimension expressions. This proves the
//! evaluation/profile boundary, not a completed production modeling compiler.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{
        ConstraintKind, Curve, CurveKind, Dimension,
        binding::{DimensionInput, DimensionRequest},
        source::Source,
    },
    document::*,
    project_file::{self, ProjectFile},
    references::Origin,
    sketch::Datum,
    sketch_regions::{ExactCurve, Region},
};
use multicad_kernel::{PlaneFrame, Profile, ProfileEdge, Solid};
use multicad_solver::{evaluate_document_sketch, prepare_dimension};
use std::{collections::BTreeMap, f64::consts::PI};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn parameter(name: &str, value: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: value.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    )
}
fn edge(e: &ExactCurve) -> ProfileEdge {
    match *e {
        ExactCurve::Line { a, b } => ProfileEdge::Line { a, b },
        ExactCurve::Arc {
            center,
            a,
            b,
            clockwise,
        } => ProfileEdge::Arc {
            center,
            a,
            b,
            clockwise,
        },
        ExactCurve::Circle { center, radius_mm } => ProfileEdge::Circle { center, radius_mm },
        ExactCurve::BSpline { .. } => panic!(
            "This fixture tests the analytic profile adapter; spline_sketch covers exact spline profiles"
        ),
    }
}
fn solid(region: &Region, sketch: EntityId) -> Solid {
    let profile = Profile {
        frame: PlaneFrame::default(),
        outer: region.outer.edges.iter().map(|e| edge(&e.curve)).collect(),
        holes: region
            .holes
            .iter()
            .map(|b| b.edges.iter().map(|e| edge(&e.curve)).collect())
            .collect(),
    };
    let origins: Vec<_> = std::iter::once(&region.outer)
        .chain(&region.holes)
        .flat_map(|b| &b.edges)
        .map(|e| Origin {
            source: sketch,
            role: format!("curve/{}", e.authored),
        })
        .collect();
    Solid::extrude_profile_named(&profile, 18., EntityId::new(), &origins, Default::default())
        .unwrap()
}
#[test]
fn parameter_edits_change_genuine_holed_volume_without_rewriting_authored_sketch_source() {
    let width = parameter("width", "80");
    let radius = parameter("hole_radius", "3");
    let (id, mut edits) = Source::create(Datum::Xy, "Panel").unwrap();
    edits.extend([
        Mutation::Insert(width.clone()),
        Mutation::Insert(radius.clone()),
    ]);
    let doc = Document::new("Parametric native profile")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut source, _, _) = Source::read(&doc, id).unwrap();
    let line = source.add_rectangle([0., 0.], [40., 20.]).unwrap();
    let corner = source.sketch.points[0].id;
    source.add_constraint(ConstraintKind::Fix { point: corner });
    let width_dim = source.add_constraint(ConstraintKind::Length {
        line,
        dimension: Dimension::Literal {
            value: 40.,
            unit: "mm".into(),
        },
    });
    source.add_constraint(ConstraintKind::Length {
        line: source.sketch.curves[1].id,
        dimension: Dimension::Literal {
            value: 20.,
            unit: "mm".into(),
        },
    });
    let center = source.add_point([10., 10.]);
    source.add_constraint(ConstraintKind::Fix { point: center });
    let circle = EntityId::new();
    source.sketch.curves.push(Curve {
        id: circle,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 2.,
        },
    });
    let radius_dim = source.add_constraint(ConstraintKind::Radius {
        curve: circle,
        dimension: Dimension::Literal {
            value: 2.,
            unit: "mm".into(),
        },
    });
    let mut session =
        Session::new(doc.edited(&source.replace(&doc, id).unwrap()).unwrap()).unwrap();
    for (constraint, text) in [(width_dim, "=$width"), (radius_dim, "=$hole_radius")] {
        let edit = prepare_dimension(
            &session,
            session.revision(),
            actor(),
            id,
            None,
            &DimensionRequest {
                constraint,
                input: DimensionInput::Expression {
                    text: text.into(),
                    unit: "mm".into(),
                },
                rebind: false,
            },
        )
        .unwrap();
        session.commit(edit.preview.unwrap()).unwrap();
    }
    let canonical = serde_json::to_vec(session.document().get(id).unwrap()).unwrap();
    let mut intent = None;
    for (w, r) in [(80., 3.), (120., 5.)] {
        if w != 80. {
            let edits = [(width.id, w), (radius.id, r)]
                .into_iter()
                .map(|(id, v)| {
                    let mut e = session.document().get(id).unwrap().clone();
                    let EntityData::Parameter { expression, .. } = &mut e.data else {
                        panic!("parameter")
                    };
                    *expression = v.to_string();
                    Mutation::Replace(e)
                })
                .collect();
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Change panel dimensions".into(),
                    edits,
                })
                .unwrap();
        }
        assert_eq!(
            serde_json::to_vec(session.document().get(id).unwrap()).unwrap(),
            canonical
        );
        let eval =
            evaluate_document_sketch(session.document(), session.revision(), id, None).unwrap();
        let extraction = eval.regions().unwrap();
        let region = extraction
            .regions
            .iter()
            .find(|r| !r.holes.is_empty())
            .unwrap();
        if let Some(previous) = &intent {
            assert_eq!(&region.intent(), previous);
        } else {
            intent = Some(region.intent());
        }
        let solid = solid(region, id);
        assert!((solid.volume_mm3().unwrap() - (w * 20. - PI * r * r) * 18.).abs() < 1e-6);
    }
    let project = ProjectFile::new(session.document().clone()).unwrap();
    let mut encoded = vec![];
    project_file::write(&project, &mut encoded, Default::default()).unwrap();
    let opened = project_file::read(
        &mut encoded.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, *session.document());
    let mut older = project_file::supported_capabilities();
    older.remove(multicad_core::constraint_sketch::source::V2_CAPABILITY);
    assert!(project_file::read(&mut encoded.as_slice(), Default::default(), &older).is_err());
    let reopened = evaluate_document_sketch(&opened.project.document, 0, id, None).unwrap();
    let extraction = reopened.regions().unwrap();
    let region = extraction
        .regions
        .iter()
        .find(|r| !r.holes.is_empty())
        .unwrap();
    assert!((solid(region, id).volume_mm3().unwrap() - (2400. - 25. * PI) * 18.).abs() < 1e-6);
    // An optional evaluated result never becomes a critical payload or changes
    // canonical source while opening/rebuilding the native project.
    assert!(opened.project.resources.payloads.is_empty());
    assert_eq!(opened.project.resources.metadata, BTreeMap::new());
}
