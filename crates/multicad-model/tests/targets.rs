use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityId, Mutation},
    hierarchy::{self, Action, ContainerKind},
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, length,
    },
};
use multicad_kernel::GeometryPolicy;
use multicad_model::{Compiler, CutTargetQuery, Model};
use std::{cell::Cell, collections::BTreeSet};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Target fixture edit".into(),
        edits,
    })
    .unwrap();
}
fn h(s: &mut Session, a: Action) -> Option<EntityId> {
    let p = hierarchy::prepare(s, s.revision(), actor(), a).unwrap();
    let id = p.created;
    s.commit(p.prepared).unwrap();
    id
}
fn box_part(s: &mut Session, name: &str, origin: [f64; 3], size: [f64; 3]) -> BodyInput {
    part(
        s,
        name,
        PrimitiveParameters::Box {
            size: size.map(length),
            frame: PrimitiveFrame {
                origin: origin.map(length),
                ..Default::default()
            },
        },
    )
}
fn part(s: &mut Session, name: &str, p: PrimitiveParameters) -> BodyInput {
    let r = PrimitiveRequest::new(name, p);
    let draft = model::prepare_primitive(s, s.revision(), actor(), &r).unwrap();
    s.commit(draft.prepared).unwrap();
    BodyInput {
        body: r.body,
        producer: r.feature,
    }
}
fn root() -> Session {
    Session::new(Document::new("Exact cut target selection").unwrap()).unwrap()
}
fn bodies(inputs: &[BodyInput]) -> BTreeSet<EntityId> {
    inputs.iter().map(|i| i.body).collect()
}
fn ids(values: impl IntoIterator<Item = EntityId>) -> BTreeSet<EntityId> {
    values.into_iter().collect()
}
struct Fixture {
    session: Session,
    b: Vec<BodyInput>,
    tool: BodyInput,
    component: EntityId,
    sibling: EntityId,
    nested: EntityId,
    group: EntityId,
}
impl Fixture {
    fn new() -> Self {
        let mut s = root();
        let component = h(
            &mut s,
            Action::Create {
                kind: ContainerKind::Component,
                name: "Active".into(),
                parent: None,
            },
        )
        .unwrap();
        let group = h(
            &mut s,
            Action::Create {
                kind: ContainerKind::Group,
                name: "Owned group".into(),
                parent: Some(component),
            },
        )
        .unwrap();
        let nested = h(
            &mut s,
            Action::Create {
                kind: ContainerKind::Component,
                name: "Nested".into(),
                parent: Some(component),
            },
        )
        .unwrap();
        let sibling = h(
            &mut s,
            Action::Create {
                kind: ContainerKind::Component,
                name: "Sibling".into(),
                parent: None,
            },
        )
        .unwrap();
        let mut b = vec![];
        for i in 0..6 {
            b.push(box_part(&mut s, &format!("Body {i}"), [0.; 3], [10.; 3]));
        }
        let tool = box_part(&mut s, "Tool", [4., 4., -1.], [2., 2., 12.]);
        for (input, parent) in [
            (&b[0], component),
            (&b[1], group),
            (&b[2], component),
            (&b[3], nested),
            (&b[4], sibling),
            (&tool, component),
        ] {
            h(
                &mut s,
                Action::Move {
                    entries: vec![input.body, input.producer],
                    parent: Some(parent),
                    before: None,
                },
            );
        }
        h(
            &mut s,
            Action::SetVisible {
                id: b[2].body,
                visible: false,
            },
        );
        h(
            &mut s,
            Action::SetActive {
                component: Some(component),
            },
        );
        Self {
            session: s,
            b,
            tool,
            component,
            sibling,
            nested,
            group,
        }
    }
    fn model(&self) -> Model {
        Compiler::default()
            .compile(self.session.document(), self.session.revision())
            .unwrap()
    }
    fn query(&self) -> CutTargetQuery {
        CutTargetQuery {
            tools: vec![self.tool.producer],
            component: Some(self.component),
            include: vec![],
            exclude: vec![],
        }
    }
}
#[test]
fn defaults_respect_visible_owned_groups_and_component_boundaries_with_exact_stable_heads() {
    let f = Fixture::new();
    let before = f.session.document().clone();
    let model = f.model();
    let q = f.query();
    let selection = model
        .cut_targets(&before, f.session.revision(), &q, None)
        .unwrap();
    assert_eq!(bodies(&selection.defaults), ids([f.b[0].body, f.b[1].body]));
    assert_eq!(selection.defaults, selection.targets);
    assert!(selection.not_intersecting.is_empty());
    for input in &selection.targets {
        assert_eq!(model.snapshot().bodies[&input.body], input.producer);
    }
    let mut root = q.clone();
    root.component = None;
    let root = model
        .cut_targets(&before, f.session.revision(), &root, None)
        .unwrap();
    assert_eq!(bodies(&root.targets), ids([f.b[5].body]));
    assert_eq!(f.session.document(), &before);
}
#[test]
fn explicit_hidden_body_foreign_component_and_exclusion_override_defaults_without_mutation() {
    let f = Fixture::new();
    let model = f.model();
    let before = f.session.document().clone();
    let native_before: Vec<_> = model
        .snapshot()
        .bodies
        .values()
        .map(|id| {
            (
                *id,
                model.solid(*id).unwrap().volume_mm3().unwrap(),
                model.solid(*id).unwrap().bounds().unwrap(),
            )
        })
        .collect();
    let mut q = f.query();
    q.include = vec![f.b[2].body, f.sibling];
    q.exclude = vec![f.b[0].body];
    let r = model
        .cut_targets(&before, f.session.revision(), &q, None)
        .unwrap();
    assert_eq!(bodies(&r.defaults), ids([f.b[0].body, f.b[1].body]));
    assert_eq!(
        bodies(&r.targets),
        ids([f.b[1].body, f.b[2].body, f.b[4].body])
    );
    assert!(r.not_intersecting.is_empty());
    assert_eq!(f.session.document(), &before);
    for (id, volume, bounds) in native_before {
        assert_eq!(model.solid(id).unwrap().volume_mm3().unwrap(), volume);
        assert_eq!(model.solid(id).unwrap().bounds().unwrap(), bounds);
    }
    q.include = vec![f.component, f.nested];
    q.exclude.clear();
    let r = model
        .cut_targets(&before, f.session.revision(), &q, None)
        .unwrap();
    assert_eq!(
        bodies(&r.targets),
        ids([f.b[0].body, f.b[1].body, f.b[3].body])
    );
    assert!(!bodies(&r.targets).contains(&f.tool.body));
    assert!(!bodies(&r.targets).contains(&f.b[2].body));
}
#[test]
fn exact_intersection_rejects_overlapping_boxes_without_volume_and_contact_faces_edges_points() {
    let mut s = root();
    let target = part(
        &mut s,
        "Cylinder",
        PrimitiveParameters::Cylinder {
            radius: length(1.),
            height: length(10.),
            frame: Default::default(),
        },
    );
    let tool = part(
        &mut s,
        "Diagonal separated cylinder",
        PrimitiveParameters::Cylinder {
            radius: length(1.),
            height: length(10.),
            frame: PrimitiveFrame {
                origin: [length(1.5), length(1.5), length(0.)],
                ..Default::default()
            },
        },
    );
    let model = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let a = model.snapshot().shapes[&target.producer].bounds;
    let b = model.snapshot().shapes[&tool.producer].bounds;
    assert!(a.max[0] > b.min[0] && a.max[1] > b.min[1]);
    let query = CutTargetQuery {
        tools: vec![tool.producer],
        component: None,
        include: vec![target.body],
        exclude: vec![],
    };
    let r = model
        .cut_targets(s.document(), s.revision(), &query, None)
        .unwrap();
    assert!(r.defaults.is_empty());
    assert!(r.targets.is_empty());
    assert_eq!(r.not_intersecting, vec![target.body]);
    for origin in [
        [10., 0., 0.],
        [10., 10., 0.],
        [10., 10., 10.],
        [100., 0., 0.],
    ] {
        let mut s = root();
        let target = box_part(&mut s, "Target", [0.; 3], [10.; 3]);
        let tool = box_part(&mut s, "Contact/disjoint tool", origin, [2.; 3]);
        let m = Compiler::default()
            .compile(s.document(), s.revision())
            .unwrap();
        let q = CutTargetQuery {
            tools: vec![tool.producer],
            component: None,
            include: vec![target.body],
            exclude: vec![],
        };
        let r = m.cut_targets(s.document(), s.revision(), &q, None).unwrap();
        assert!(
            r.targets.is_empty(),
            "selected non-volume contact at{origin:?}"
        );
        assert_eq!(r.not_intersecting, vec![target.body]);
    }
}
#[test]
fn stale_foreign_same_revision_hash_and_invalid_overrides_or_tool_identity_reject() {
    let f = Fixture::new();
    let model = f.model();
    let query = f.query();
    let revision = f.session.revision();
    assert!(
        model
            .cut_targets(f.session.document(), revision + 1, &query, None)
            .is_err()
    );
    assert!(
        model
            .cut_targets(&Document::new("Foreign").unwrap(), revision, &query, None)
            .is_err()
    );
    let changed = f
        .session
        .document()
        .edited(&[Mutation::RenameDocument("Hash changed".into())])
        .unwrap();
    assert!(
        model
            .cut_targets(&changed, revision, &query, None)
            .unwrap_err()
            .message
            .contains("current")
    );
    let mut invalid = vec![];
    let mut q = query.clone();
    q.tools.clear();
    invalid.push(q);
    let mut q = query.clone();
    q.tools.push(q.tools[0]);
    invalid.push(q);
    let mut q = query.clone();
    q.tools[0] = f.tool.body;
    invalid.push(q);
    let mut q = query.clone();
    q.tools[0] = EntityId::new();
    invalid.push(q);
    let mut q = query.clone();
    q.include = vec![f.tool.body];
    invalid.push(q);
    let mut q = query.clone();
    q.include = vec![EntityId::new()];
    invalid.push(q);
    let mut q = query.clone();
    q.include = vec![f.group];
    invalid.push(q);
    let mut q = query.clone();
    q.exclude = vec![f.component];
    invalid.push(q);
    let mut q = query.clone();
    q.exclude = vec![EntityId::new()];
    invalid.push(q);
    let mut q = query.clone();
    q.component = Some(f.group);
    invalid.push(q);
    let mut q = query.clone();
    q.include = vec![f.b[0].body, f.b[0].body];
    invalid.push(q);
    for q in invalid {
        assert!(
            model
                .cut_targets(f.session.document(), revision, &q, None)
                .is_err()
        );
    }
}
#[test]
fn consumed_or_obsolete_tool_versions_and_ambiguous_body_aliases_are_not_guessed() {
    let mut f = Fixture::new();
    let req = BooleanRequest::new(Operation::Cut, f.b[0].clone(), vec![f.tool.clone()]);
    let p = model::prepare_boolean(&f.session, f.session.revision(), actor(), &req).unwrap();
    f.session.commit(p.prepared).unwrap();
    let m = f.model();
    assert!(m.snapshot().consumed_bodies.contains(&f.tool.body));
    let q = f.query();
    assert!(
        m.cut_targets(f.session.document(), f.session.revision(), &q, None)
            .is_err()
    );
    let mut q = f.query();
    q.tools = vec![f.b[0].producer];
    assert!(
        m.cut_targets(f.session.document(), f.session.revision(), &q, None)
            .is_err()
    );
    let mut q = f.query();
    q.tools = vec![req.feature];
    q.include = vec![f.tool.body];
    assert!(
        m.cut_targets(f.session.document(), f.session.revision(), &q, None)
            .is_err()
    );
    let mut f = Fixture::new();
    let mut alias = f.session.document().get(f.tool.body).unwrap().clone();
    alias.id = EntityId::new();
    apply(&mut f.session, vec![Mutation::Insert(alias)]);
    let m = f.model();
    assert!(
        m.cut_targets(f.session.document(), f.session.revision(), &f.query(), None)
            .unwrap_err()
            .message
            .contains("uniquely")
    );
}
#[test]
fn cancellation_is_checked_before_and_between_native_work_and_preserves_source() {
    let f = Fixture::new();
    let model = f.model();
    let before = f.session.document().clone();
    assert!(
        model
            .cut_targets(&before, f.session.revision(), &f.query(), Some(&|| true))
            .unwrap_err()
            .message
            .contains("superseded")
    );
    let n = Cell::new(0);
    let interrupt = || {
        n.set(n.get() + 1);
        n.get() >= 5
    };
    assert!(
        model
            .cut_targets(&before, f.session.revision(), &f.query(), Some(&interrupt))
            .unwrap_err()
            .message
            .contains("superseded")
    );
    assert!(n.get() >= 5);
    assert_eq!(f.session.document(), &before);
}
#[test]
fn stored_custom_geometry_policy_multi_tool_hits_and_empty_components_are_explicit() {
    let mut f = Fixture::new();
    let extra = box_part(&mut f.session, "Other tool", [5., 5., 0.], [1., 1., 10.]);
    let empty = h(
        &mut f.session,
        Action::Create {
            kind: ContainerKind::Component,
            name: "Empty".into(),
            parent: None,
        },
    )
    .unwrap();
    let compiler = Compiler {
        policy: GeometryPolicy {
            minimum_feature_mm: 1e-4,
            maximum_coordinate_mm: 1e6,
            ..Default::default()
        },
        ..Default::default()
    };
    let m = compiler
        .compile(f.session.document(), f.session.revision())
        .unwrap();
    let mut q = f.query();
    q.tools.push(extra.producer);
    q.include = vec![empty];
    let r = m
        .cut_targets(f.session.document(), f.session.revision(), &q, None)
        .unwrap();
    assert_eq!(bodies(&r.targets), ids([f.b[0].body, f.b[1].body]));
    assert_eq!(r.not_intersecting, vec![empty]);
}
#[test]
fn query_input_pool_and_final_target_budgets_are_bounded_with_overrides_applied_last() {
    let mut s = root();
    let target = box_part(&mut s, "Shared source target", [0.; 3], [10.; 3]);
    let tool = box_part(&mut s, "Tool", [4., 4., -1.], [2., 2., 12.]);
    let original = s.document().get(target.body).unwrap().clone();
    let mut aliases = vec![];
    for _ in 0..128 {
        let mut copy = original.clone();
        copy.id = EntityId::new();
        aliases.push(copy);
    }
    apply(
        &mut s,
        aliases.iter().cloned().map(Mutation::Insert).collect(),
    );
    let m = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let mut q = CutTargetQuery {
        tools: vec![tool.producer],
        component: None,
        include: vec![],
        exclude: vec![],
    };
    assert!(
        m.cut_targets(s.document(), s.revision(), &q, None)
            .unwrap_err()
            .message
            .contains("128")
    );
    q.exclude.push(target.body);
    let r = m.cut_targets(s.document(), s.revision(), &q, None).unwrap();
    assert_eq!(r.defaults.len(), 129);
    assert_eq!(r.targets.len(), 128);
    q.include = (0..4097).map(|_| EntityId::new()).collect();
    assert!(
        m.cut_targets(s.document(), s.revision(), &q, None)
            .unwrap_err()
            .message
            .contains("budget")
    );
    q.include.clear();
    q.tools = (0..129).map(|_| EntityId::new()).collect();
    assert!(
        m.cut_targets(s.document(), s.revision(), &q, None)
            .unwrap_err()
            .message
            .contains("budget")
    );
    let mut doc = s.document().clone();
    let mut edits = vec![];
    for _ in 0..3968 {
        let mut copy = original.clone();
        copy.id = EntityId::new();
        edits.push(Mutation::Insert(copy));
    }
    doc = doc.edited(&edits).unwrap();
    let m = Compiler::default().compile(&doc, 0).unwrap();
    let q = CutTargetQuery {
        tools: vec![tool.producer],
        component: None,
        include: vec![],
        exclude: vec![],
    };
    assert!(
        m.cut_targets(&doc, 0, &q, None)
            .unwrap_err()
            .message
            .contains("4096")
    );
}
#[test]
fn expanded_component_overrides_report_nonhits_but_do_not_cross_hidden_or_nested_boundaries() {
    let mut f = Fixture::new();
    let far = box_part(&mut f.session, "Far sibling body", [100., 0., 0.], [10.; 3]);
    let hidden = box_part(&mut f.session, "Hidden sibling body", [0.; 3], [10.; 3]);
    for input in [&far, &hidden] {
        h(
            &mut f.session,
            Action::Move {
                entries: vec![input.body, input.producer],
                parent: Some(f.sibling),
                before: None,
            },
        );
    }
    h(
        &mut f.session,
        Action::SetVisible {
            id: hidden.body,
            visible: false,
        },
    );
    let nested = h(
        &mut f.session,
        Action::Create {
            kind: ContainerKind::Component,
            name: "Nested sibling".into(),
            parent: Some(f.sibling),
        },
    )
    .unwrap();
    let deep = box_part(&mut f.session, "Nested sibling body", [0.; 3], [10.; 3]);
    h(
        &mut f.session,
        Action::Move {
            entries: vec![deep.body, deep.producer],
            parent: Some(nested),
            before: None,
        },
    );
    let model = f.model();
    let mut q = f.query();
    q.include = vec![f.sibling];
    let result = model
        .cut_targets(f.session.document(), f.session.revision(), &q, None)
        .unwrap();
    assert_eq!(
        bodies(&result.targets),
        ids([f.b[0].body, f.b[1].body, f.b[4].body])
    );
    assert_eq!(result.not_intersecting, vec![far.body]);
}
#[test]
fn tangent_native_cylinders_with_overlapping_bounds_are_zero_volume_contacts() {
    let mut s = root();
    let a = part(
        &mut s,
        "Target",
        PrimitiveParameters::Cylinder {
            radius: length(1.),
            height: length(10.),
            frame: Default::default(),
        },
    );
    let b = part(
        &mut s,
        "Tangent tool",
        PrimitiveParameters::Cylinder {
            radius: length(1.),
            height: length(10.),
            frame: PrimitiveFrame {
                origin: [length(2_f64.sqrt()), length(2_f64.sqrt()), length(0.)],
                ..Default::default()
            },
        },
    );
    let model = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let query = CutTargetQuery {
        tools: vec![b.producer],
        component: None,
        include: vec![a.body],
        exclude: vec![],
    };
    let result = model
        .cut_targets(s.document(), s.revision(), &query, None)
        .unwrap();
    assert!(result.defaults.is_empty());
    assert!(result.targets.is_empty());
    assert_eq!(result.not_intersecting, vec![a.body]);
}

#[test]
fn positive_native_intersections_below_absolute_volume_scales_remain_cut_targets() {
    let mut session = root();
    let edge = GeometryPolicy::default().minimum_feature_mm;
    let target = box_part(&mut session, "Small target", [0.; 3], [edge; 3]);
    let tool = box_part(
        &mut session,
        "Small intersecting tool",
        [edge / 2., 0., 0.],
        [edge; 3],
    );
    let source = session.document().clone();
    let model = Compiler::default()
        .compile(&source, session.revision())
        .unwrap();
    let query = CutTargetQuery {
        tools: vec![tool.producer],
        component: None,
        include: vec![],
        exclude: vec![],
    };
    let selection = model
        .cut_targets(&source, session.revision(), &query, None)
        .unwrap();
    assert_eq!(selection.targets, vec![target.clone()]);
    let common = model
        .solid(target.producer)
        .unwrap()
        .boolean(
            model.solid(tool.producer).unwrap(),
            multicad_kernel::BooleanOperation::Intersect,
            GeometryPolicy::default(),
        )
        .unwrap()
        .unwrap();
    let expected = edge.powi(3) / 2.;
    let actual = common.volume_mm3().unwrap();
    assert!(expected < 1e-12 && actual > 0.);
    assert!(
        (actual / expected - 1.).abs() < 1e-6,
        "{actual} != {expected}"
    );
    assert_eq!(session.document(), &source);
}
