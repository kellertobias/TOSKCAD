//! FDM aids as their own Parts: a support whose height follows the target's
//! real solid, a breakaway tab and a bridge with measured contact, process
//! allowance findings, a fit-clearance finding against a neighbouring Part,
//! Undo/Redo, save/reopen, and export recipes that include or exclude the
//! aids by identity. Every size is measured from the native solids or read
//! back from the exported files.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    exchange::*,
    model::{
        self, AidDefinition, AidKind, AidRequest, AidShape, BodyOutput, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, ProcessDefinition, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
use multicad_kernel::{MeshOptions, SolidPairRelation};
use multicad_model::{AidInspection, Compiler, GeometryBackend, Model, export::Exporter};
use std::path::Path;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-6, "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
/// Evaluate the candidate natively, then commit it.
fn checked(s: &mut Session, prepared: Prepared) {
    Compiler::default()
        .compile(prepared.document(), s.revision() + 1)
        .unwrap();
    s.commit(prepared).unwrap();
}
fn xyz(v: [f64; 3]) -> [Dimension; 3] {
    v.map(length)
}
fn inspection<'a>(list: &'a [AidInspection], name: &str) -> &'a AidInspection {
    list.iter()
        .find(|a| a.name == name)
        .unwrap_or_else(|| panic!("no aid {name}"))
}
fn part_bounds(model: &Model, part: EntityId) -> [[f64; 3]; 2] {
    let head = model.snapshot().bodies[&part];
    let b = model.solid(head).unwrap().bounds().unwrap();
    [b.min, b.max]
}

struct Fixture {
    s: Session,
    /// 40 × 20 × 10 mm block whose underside is at `lift` mm.
    block: PrimitiveRequest,
    /// A 10 mm cube resting on the plate beside the block.
    post: PrimitiveRequest,
    lift: EntityId,
    process: EntityId,
}
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("FDM aids").unwrap()).unwrap();
    let lift = Entity::new(
        "lift",
        EntityData::Parameter {
            expression: "15".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let lift_id = lift.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Parameter".into(),
        edits: vec![Mutation::Insert(lift)],
    })
    .unwrap();
    let block = PrimitiveRequest::new(
        "Block",
        PrimitiveParameters::Box {
            size: [length(40.), length(20.), length(10.)],
            frame: PrimitiveFrame {
                origin: [
                    length(0.),
                    length(0.),
                    Dimension::Expression {
                        expression: BoundExpression::bind(s.document(), "=$lift", "mm", None)
                            .unwrap(),
                    },
                ],
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(&s, s.revision(), actor(), &block).unwrap();
    checked(&mut s, p.prepared);
    let post = PrimitiveRequest::new(
        "Post",
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(10.)],
            frame: PrimitiveFrame {
                origin: xyz([60., 0., 0.]),
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(&s, s.revision(), actor(), &post).unwrap();
    checked(&mut s, p.prepared);
    let (process, prepared) = model::prepare_process(
        &s,
        s.revision(),
        actor(),
        None,
        "PLA 0.4 mm",
        &ProcessDefinition::default(),
    )
    .unwrap();
    s.commit(prepared).unwrap();
    Fixture {
        s,
        block,
        post,
        lift: lift_id,
        process,
    }
}
fn aid(process: EntityId, target: EntityId, shape: AidShape) -> AidDefinition {
    AidDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        target,
        shape,
        process: Some(process),
    }
}
fn create(s: &mut Session, name: &str, definition: AidDefinition) -> (EntityId, EntityId) {
    let request = AidRequest::new(name, definition);
    let prepared = model::prepare_aid(s, s.revision(), actor(), &request).unwrap();
    checked(s, prepared.prepared);
    (request.feature, request.body)
}
fn support(origin: [f64; 2], size: [f64; 2], gap: f64) -> AidShape {
    AidShape::Support {
        origin: origin.map(length),
        angle: angle(0.),
        length: length(size[0]),
        width: length(size[1]),
        base: length(0.),
        gap: length(gap),
    }
}
fn tab(origin: [f64; 3], deg: f64, thickness: f64) -> AidShape {
    AidShape::Tab {
        origin: xyz(origin),
        angle: angle(deg),
        length: length(8.),
        width: length(6.),
        thickness: length(thickness),
        overlap: length(2.),
    }
}
fn set_lift(s: &mut Session, id: EntityId, expression: &str) {
    let mut entity = s.document().get(id).unwrap().clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        unreachable!()
    };
    *e = expression.into();
    let prepared = s
        .prepare(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Edit parameter".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
    checked(s, prepared);
}

#[test]
fn support_height_follows_the_target_solid_and_the_target_stays_unchanged() {
    let mut f = fixture();
    let before = compile(&f.s);
    let block_before = part_bounds(&before, f.block.body);
    let block_volume = before
        .solid(before.snapshot().bodies[&f.block.body])
        .unwrap()
        .volume_mm3()
        .unwrap();
    let (feature, body) = create(
        &mut f.s,
        "Block support",
        aid(f.process, f.block.body, support([5., 5.], [30., 10.], 0.2)),
    );
    assert!(model::aid_parts(f.s.document()).contains(&body));
    assert!(!model::aid_parts(f.s.document()).contains(&f.block.body));
    let m = compile(&f.s);
    let b = part_bounds(&m, body);
    near(b[0][2], 0.);
    near(b[1][2], 14.8);
    near(b[1][0] - b[0][0], 30.);
    // The finished Part is not changed by its aid.
    assert_eq!(part_bounds(&m, f.block.body), block_before);
    let head = m.snapshot().bodies[&f.block.body];
    near(m.solid(head).unwrap().volume_mm3().unwrap(), block_volume);
    let list = m.fdm_aids(f.s.document()).unwrap();
    let s = inspection(&list, "Block support");
    assert_eq!(s.kind, AidKind::Support);
    assert_eq!(s.feature, feature);
    assert_eq!(s.target.relation, SolidPairRelation::Separated);
    assert!(
        (s.target.distance_mm - 0.2).abs() < 1e-4,
        "{}",
        s.target.distance_mm
    );
    assert!(s.issues.is_empty(), "{:?}", s.issues);

    // Raising the block raises the derived support top.
    set_lift(&mut f.s, f.lift, "25");
    let m = compile(&f.s);
    near(part_bounds(&m, body)[1][2], 24.8);
    // Undo and Redo restore both the source and the evaluated height.
    f.s.undo(f.s.revision(), actor()).unwrap();
    near(part_bounds(&compile(&f.s), body)[1][2], 14.8);
    f.s.redo(f.s.revision(), actor()).unwrap();
    near(part_bounds(&compile(&f.s), body)[1][2], 24.8);

    // A gap below the process support gap is reported, not hidden.
    let mut edited = model::read_aid(f.s.document(), feature).unwrap();
    edited.shape = support([5., 5.], [30., 10.], 0.05);
    let p = model::prepare_aid_edit(&f.s, f.s.revision(), actor(), feature, &edited).unwrap();
    assert_eq!(p.outputs[0].body, body);
    checked(&mut f.s, p.prepared);
    let list = compile(&f.s).fdm_aids(f.s.document()).unwrap();
    let issues = &inspection(&list, "Block support").issues;
    assert!(
        issues.iter().any(|i| i.contains("process support gap")),
        "{issues:?}"
    );

    // Lowering the block onto the plate leaves no room; the rebuild fails
    // with a reason instead of building an overlapping support.
    let mut entity = f.s.document().get(f.lift).unwrap().clone();
    if let EntityData::Parameter { expression, .. } = &mut entity.data {
        *expression = "0".into();
    }
    let prepared =
        f.s.prepare(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Lower".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
    let error = Compiler::default()
        .compile(prepared.document(), f.s.revision() + 1)
        .err()
        .unwrap();
    assert_eq!(error.entity, Some(feature));
    assert!(error.message.contains("No room"), "{}", error.message);

    // A footprint beside the block has no material above it.
    let mut beside = model::read_aid(f.s.document(), feature).unwrap();
    beside.shape = support([100., 100.], [5., 5.], 0.2);
    let p = model::prepare_aid_edit(&f.s, f.s.revision(), actor(), feature, &beside).unwrap();
    let error = Compiler::default()
        .compile(p.prepared.document(), f.s.revision() + 1)
        .err()
        .unwrap();
    assert!(error.message.contains("not under"), "{}", error.message);
}

#[test]
fn tabs_and_bridges_report_measured_contact_thickness_and_fit_clearance() {
    let mut f = fixture();
    set_lift(&mut f.s, f.lift, "0");
    // A tab on the block's +X face (x = 40) runs outward and 2 mm under it.
    let (_, tab_body) = create(
        &mut f.s,
        "Tab right",
        aid(f.process, f.block.body, tab([40., 10., 0.], 0., 0.4)),
    );
    // A tab placed 5 mm off the block's −Y face does not touch it, and its
    // 0.3 mm thickness is neither two layers nor a whole number of layers.
    create(
        &mut f.s,
        "Tab loose",
        aid(f.process, f.block.body, tab([20., -7., 0.], -90., 0.3)),
    );
    // A bridge from the block to the post, 2 mm wide and 0.6 mm thick.
    let mut bridge = aid(
        f.process,
        f.block.body,
        AidShape::Bridge {
            start: xyz([39., 5., 5.]),
            end: xyz([61., 5., 5.]),
            width: length(2.),
            thickness: length(0.6),
            other: Some(f.post.body),
        },
    );
    bridge.process = Some(f.process);
    let (bridge_feature, bridge_body) = create(&mut f.s, "Bridge", bridge);
    let m = compile(&f.s);
    let b = part_bounds(&m, tab_body);
    near(b[0][0], 38.);
    near(b[1][0], 48.);
    near(b[1][1] - b[0][1], 6.);
    near(b[1][2], 0.4);
    let b = part_bounds(&m, bridge_body);
    near(b[0][0], 39.);
    near(b[1][0], 61.);
    near(b[1][2] - b[0][2], 0.6);
    near(b[1][1] - b[0][1], 2.);

    let list = m.fdm_aids(f.s.document()).unwrap();
    let right = inspection(&list, "Tab right");
    assert_eq!(right.target.relation, SolidPairRelation::Overlapping);
    near(right.target.common_volume_mm3, 2. * 6. * 0.4);
    // The tab ends at x = 48, 12 mm from the post: no fit finding.
    assert!(right.issues.is_empty(), "{:?}", right.issues);

    let loose = inspection(&list, "Tab loose");
    assert_eq!(loose.target.relation, SolidPairRelation::Separated);
    assert!((loose.target.distance_mm - 5.).abs() < 1e-6);
    let text = loose.issues.join("\n");
    assert!(text.contains("does not touch Block"), "{text}");
    assert!(text.contains("below 2 layers"), "{text}");
    assert!(text.contains("whole number"), "{text}");

    let bridge = inspection(&list, "Bridge");
    assert_eq!(bridge.target.relation, SolidPairRelation::Overlapping);
    let other = bridge.other.as_ref().unwrap();
    assert_eq!(other.part, f.post.body);
    assert_eq!(other.relation, SolidPairRelation::Overlapping);
    assert!(bridge.issues.is_empty(), "{:?}", bridge.issues);

    // Shortened, the bridge no longer reaches the post.
    let mut short = model::read_aid(f.s.document(), bridge_feature).unwrap();
    if let AidShape::Bridge { end, .. } = &mut short.shape {
        *end = xyz([55., 5., 5.]);
    }
    let p = model::prepare_aid_edit(&f.s, f.s.revision(), actor(), bridge_feature, &short).unwrap();
    checked(&mut f.s, p.prepared);
    let list = compile(&f.s).fdm_aids(f.s.document()).unwrap();
    let text = inspection(&list, "Bridge").issues.join("\n");
    assert!(text.contains("does not reach Post"), "{text}");
    assert!(!text.contains("fit clearance"), "{text}");

    // A tab ending 0.1 mm short of the post crowds it.
    let mut crowding = aid(f.process, f.block.body, tab([40., 5., 0.], 0., 0.4));
    if let AidShape::Tab { length: l, .. } = &mut crowding.shape {
        *l = length(19.9);
    }
    create(&mut f.s, "Tab crowding", crowding);
    let list = compile(&f.s).fdm_aids(f.s.document()).unwrap();
    let crowding = inspection(&list, "Tab crowding");
    assert_eq!(crowding.crowded.len(), 1);
    assert_eq!(crowding.crowded[0].0, f.post.body);
    assert!((crowding.crowded[0].1 - 0.1).abs() < 1e-6);
}

#[test]
fn rust_aids_match_native_contacts_and_use_the_lowest_material_over_the_footprint() {
    let mut f = fixture();
    let slope = std::f64::consts::FRAC_1_SQRT_2;
    let inclined = PrimitiveRequest::new(
        "Inclined stock",
        PrimitiveParameters::Box {
            size: xyz([20., 20., 2.]),
            frame: PrimitiveFrame {
                origin: xyz([100., 0., 10.]),
                u: [slope, 0., slope],
                v: [0., 1., 0.],
            },
        },
    );
    let p = model::prepare_primitive(&f.s, f.s.revision(), actor(), &inclined).unwrap();
    checked(&mut f.s, p.prepared);
    let mut footprint = support([104., 5.], [3., 3.], 0.2);
    if let AidShape::Support { angle: a, .. } = &mut footprint {
        *a = angle(45.);
    }
    let (support_id, _) = create(
        &mut f.s,
        "Inclined support",
        aid(f.process, inclined.body, footprint),
    );
    create(
        &mut f.s,
        "Flat support",
        aid(f.process, f.block.body, support([5., 5.], [30., 10.], 0.2)),
    );
    let (tab_id, _) = create(
        &mut f.s,
        "Angled tab",
        aid(f.process, f.block.body, tab([40., 10., 15.], 30., 0.4)),
    );
    let (bridge_id, _) = create(
        &mut f.s,
        "Sloping bridge",
        aid(
            f.process,
            f.block.body,
            AidShape::Bridge {
                start: xyz([39., 5., 18.]),
                end: xyz([61., 5., 5.]),
                width: length(2.),
                thickness: length(0.6),
                other: Some(f.post.body),
            },
        ),
    );
    let native = compile(&f.s);
    let rust = Compiler::default()
        .compile_with_backend(f.s.document(), f.s.revision(), GeometryBackend::Rust, None)
        .unwrap();
    let actual = rust.snapshot().fdm_aids.as_ref().unwrap();
    let expected = native.snapshot().fdm_aids.as_ref().unwrap();
    assert_eq!(actual, &rust.fdm_aids(f.s.document()).unwrap());
    assert_eq!(actual.len(), 4);
    for (feature, role) in [
        (support_id, "support.start"),
        (tab_id, "tab.contact"),
        (bridge_id, "bridge.start"),
    ] {
        let solid = rust.solid(feature).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert!(
            !solid
                .manufacturing_mesh(MeshOptions::default())
                .unwrap()
                .indices
                .is_empty()
        );
        assert!(
            rust.snapshot().shapes[&feature]
                .topology
                .elements
                .iter()
                .any(|element| element
                    .origins
                    .iter()
                    .any(|origin| origin.source == feature && origin.role == role))
        );
        let a = actual.iter().find(|aid| aid.feature == feature).unwrap();
        let e = inspection(expected, &a.name);
        assert_eq!(a.part, e.part);
        assert_eq!(a.process, e.process);
        assert_eq!(a.target.relation, e.target.relation);
        near(a.target.distance_mm, e.target.distance_mm);
        near(a.target.common_volume_mm3, e.target.common_volume_mm3);
        if let Some(other) = &a.other {
            let native_other = e.other.as_ref().unwrap();
            assert_eq!(other.part, native_other.part);
            assert_eq!(other.relation, SolidPairRelation::Overlapping);
            near(other.common_volume_mm3, native_other.common_volume_mm3);
        }
        near(a.volume_mm3, e.volume_mm3);
        for (a, e) in a
            .bounds_mm
            .iter()
            .flatten()
            .zip(e.bounds_mm.iter().flatten())
        {
            near(*a, *e);
        }
    }
    let tab = inspection(actual, "Angled tab");
    near(tab.target.common_volume_mm3, 4.8);
    let contact = tab.exact_contact.unwrap();
    near(contact.point_mm[0], 40.);
    near(contact.point_mm[2], 15.);
    near(contact.angle_deg, 0.);
    let inclined = inspection(actual, "Inclined support");
    near(inclined.bounds_mm[1][2], 14. - 3. * slope - 0.2);
    assert_eq!(inclined.target.relation, SolidPairRelation::Separated);
    let flat = inspection(actual, "Flat support");
    near(flat.bounds_mm[1][2], 14.8);
    near(flat.target.distance_mm, 0.2);
    assert!(flat.contact_issues.is_empty(), "{:?}", flat.contact_issues);
}

#[test]
fn rust_aids_report_missing_footprint_and_measure_curved_tab_contact() {
    let mut f = fixture();
    let compiler = Compiler::default();
    let missing = AidRequest::new(
        "Beside stock",
        aid(
            f.process,
            f.block.body,
            support([100., 100.], [5., 5.], 0.2),
        ),
    );
    let preview = model::prepare_aid(&f.s, f.s.revision(), actor(), &missing).unwrap();
    let missing_error = compiler
        .compile_with_backend(
            preview.prepared.document(),
            f.s.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(missing_error.entity, Some(missing.feature));
    assert!(
        missing_error.message.contains("not under"),
        "{missing_error}"
    );
    assert!(f.s.document().get(missing.body).is_none());

    let cylinder = PrimitiveRequest::new(
        "Cylinder",
        PrimitiveParameters::Cylinder {
            radius: length(8.),
            height: length(6.),
            frame: PrimitiveFrame {
                origin: xyz([100., 100., 15.]),
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(&f.s, f.s.revision(), actor(), &cylinder).unwrap();
    checked(&mut f.s, p.prepared);
    let support = AidRequest::new(
        "Below a curved body",
        aid(f.process, cylinder.body, support([99., 99.], [2., 2.], 0.2)),
    );
    let preview = model::prepare_aid(&f.s, f.s.revision(), actor(), &support).unwrap();
    let curved_error = compiler
        .compile_with_backend(
            preview.prepared.document(),
            f.s.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(curved_error.entity, Some(support.feature));
    assert!(curved_error.message.contains("planar"), "{curved_error}");
    assert!(f.s.document().get(support.body).is_none());

    create(
        &mut f.s,
        "Tab on cylinder",
        aid(f.process, cylinder.body, tab([108., 100., 15.], 0., 0.4)),
    );
    let rust = compiler
        .compile_with_backend(f.s.document(), f.s.revision(), GeometryBackend::Rust, None)
        .unwrap();
    let report = rust.snapshot().fdm_aids.as_ref().unwrap();
    let tab = inspection(report, "Tab on cylinder");
    assert_eq!(tab.target.relation, SolidPairRelation::Overlapping);
    let expected = (3. * 55_f64.sqrt() + 64. * (3_f64 / 8.).asin() - 36.) * 0.4;
    near(tab.target.common_volume_mm3, expected);
    assert!(tab.contact_issues.is_empty());
    let contact = tab.exact_contact.unwrap();
    near(contact.point_mm[0], 108.);
    assert!((contact.point_mm[1] - 100.).abs() < 1e-4);
    near(contact.point_mm[2], 15.);
    assert!(contact.angle_deg.abs() < 1e-3);
}

#[test]
fn invalid_aids_are_refused_before_any_geometry_is_built() {
    let f = fixture();
    let s = &f.s;
    let try_create = |definition: AidDefinition| {
        model::prepare_aid(
            s,
            s.revision(),
            actor(),
            &AidRequest::new("Bad", definition),
        )
        .map(|_| ())
        .unwrap_err()
        .message
    };
    // Zero thickness.
    assert!(
        try_create(aid(f.process, f.block.body, tab([0., 0., 0.], 0., 0.))).contains("thickness")
    );
    // A vertical bridge.
    let vertical = AidShape::Bridge {
        start: xyz([0., 0., 0.]),
        end: xyz([0., 0., 10.]),
        width: length(1.),
        thickness: length(1.),
        other: None,
    };
    assert!(try_create(aid(f.process, f.block.body, vertical)).contains("vertically"));
    // A target that is not a Part.
    assert!(
        try_create(aid(f.process, f.process, support([0., 0.], [1., 1.], 0.2)))
            .contains("must be a Part")
    );
    // A bridge tied to its own target.
    let own = AidShape::Bridge {
        start: xyz([0., 0., 0.]),
        end: xyz([10., 0., 0.]),
        width: length(1.),
        thickness: length(1.),
        other: Some(f.block.body),
    };
    assert!(try_create(aid(f.process, f.block.body, own)).contains("must differ"));
    // A process with impossible allowances.
    let bad = ProcessDefinition {
        layer_height: length(0.),
        ..Default::default()
    };
    assert!(
        model::prepare_process(s, s.revision(), actor(), None, "Bad", &bad)
            .map(|_| ())
            .unwrap_err()
            .message
            .contains("layer height")
    );
}

#[test]
fn aids_save_reopen_and_are_removed_without_touching_the_target() {
    let mut f = fixture();
    let (feature, body) = create(
        &mut f.s,
        "Block support",
        aid(f.process, f.block.body, support([5., 5.], [30., 10.], 0.2)),
    );
    let list = compile(&f.s).fdm_aids(f.s.document()).unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(f.s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert!(opened.capabilities().contains(model::AID_CAPABILITY));
    assert!(opened.capabilities().contains(model::PROCESS_CAPABILITY));
    assert_eq!(&opened.document, f.s.document());
    let reopened = Compiler::default()
        .compile(&opened.document, 0)
        .unwrap()
        .fdm_aids(&opened.document)
        .unwrap();
    assert_eq!(reopened, list);

    let prepared = model::prepare_aid_remove(&f.s, f.s.revision(), actor(), feature).unwrap();
    checked(&mut f.s, prepared);
    assert!(f.s.document().get(body).is_none());
    assert!(f.s.document().get(feature).is_none());
    assert!(f.s.document().get(f.block.body).is_some());
    assert!(compile(&f.s).fdm_aids(f.s.document()).unwrap().is_empty());
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert!(model::aid_parts(f.s.document()).contains(&body));
}

/// A printed Part used as a reusable Assembly component (a drawer on its
/// slide) has no world body; its aids are inspected against its definition
/// solid, the frame they are built in, instead of reporting it missing.
#[test]
fn aids_on_an_assembly_component_part_are_inspected_against_its_definition() {
    let mut f = fixture();
    set_lift(&mut f.s, f.lift, "0");
    let (_, tab_body) = create(
        &mut f.s,
        "Tab right",
        aid(f.process, f.block.body, tab([40., 10., 0.], 0., 0.4)),
    );
    let member = model::ComponentPlacementRequest::new(f.block.body, Default::default());
    let p = model::prepare_place_component(&f.s, f.s.revision(), actor(), &member).unwrap();
    checked(&mut f.s, p.prepared);
    let assembly = model::AssemblyRequest::new("Drawer", vec![member.instance], vec![f.block.body]);
    let p = model::prepare_assembly(&f.s, f.s.revision(), actor(), &assembly).unwrap();
    checked(&mut f.s, p.prepared);
    let root = model::ComponentPlacementRequest::new(
        assembly.assembly,
        model::NumericTransform {
            translation: xyz([100., 0., 0.]),
            ..Default::default()
        },
    );
    let p = model::prepare_place_component(&f.s, f.s.revision(), actor(), &root).unwrap();
    checked(&mut f.s, p.prepared);
    let m = compile(&f.s);
    assert!(!m.snapshot().bodies.contains_key(&f.block.body));
    let list = m.fdm_aids(f.s.document()).unwrap();
    let right = inspection(&list, "Tab right");
    assert_eq!(right.part, tab_body);
    assert_eq!(right.target.relation, SolidPairRelation::Overlapping);
    near(right.target.common_volume_mm3, 2. * 6. * 0.4);
    assert!(right.issues.is_empty(), "{:?}", right.issues);
}

/// Export the whole project with one recipe and return the preview, the
/// receipt and the written files by name.
fn export(
    f: &mut Fixture,
    output: RecipeOutput,
) -> (ExportPreview, Vec<(String, Vec<u8>)>, tempfile::TempDir) {
    let set = EntityId::new();
    let p = prepare_set(
        &f.s,
        f.s.revision(),
        actor(),
        set,
        "Print",
        &SetDefinition {
            recipes: vec![Recipe {
                id: EntityId::new(),
                name: "Recipe".into(),
                output,
            }],
            ..Default::default()
        },
    )
    .unwrap();
    f.s.commit(p).unwrap();
    let exporter = Exporter::default();
    let preview = exporter
        .preview(f.s.document(), f.s.revision(), set, &actor(), None)
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let receipt = exporter
        .run(
            f.s.document(),
            f.s.revision(),
            &preview.key,
            root.path(),
            &actor(),
            None,
        )
        .unwrap();
    let files = receipt
        .files
        .iter()
        .map(|file| {
            let name = file.artifact.output.filename.clone();
            let bytes = std::fs::read(Path::new(&receipt.directory).join(&name)).unwrap();
            (name, bytes)
        })
        .collect();
    (preview, files, root)
}
fn obj_objects(bytes: &[u8]) -> Vec<String> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .filter_map(|l| l.strip_prefix("o ").map(str::to_owned))
        .collect()
}
fn mesh(aids: AidPolicy, hierarchy: HierarchyMode, format: MeshFormat) -> RecipeOutput {
    RecipeOutput::Mesh {
        format,
        units: LengthUnit::Millimeter,
        hierarchy,
        names: NameHandling::ProductNames,
        aids,
        quality: MeshQuality::default(),
    }
}

#[test]
fn export_recipes_include_or_exclude_aids_by_identity() {
    let mut f = fixture();
    let (_, support_body) = create(
        &mut f.s,
        "Block support",
        aid(f.process, f.block.body, support([5., 5.], [30., 10.], 0.2)),
    );
    // A finished Part named like an aid is still finished geometry.
    let decoy = PrimitiveRequest::new(
        "Support decoy",
        PrimitiveParameters::Box {
            size: [length(5.), length(5.), length(5.)],
            frame: PrimitiveFrame {
                origin: xyz([0., 60., 0.]),
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(&f.s, f.s.revision(), actor(), &decoy).unwrap();
    checked(&mut f.s, p.prepared);

    // One assembly OBJ including everything holds four objects.
    let (preview, files, _root) = export(
        &mut f,
        mesh(
            AidPolicy::IncludeAll,
            HierarchyMode::PreserveAssembly,
            MeshFormat::Obj,
        ),
    );
    assert!(preview.recipes[0].excluded_aids.is_empty());
    let objects = obj_objects(&files[0].1);
    assert_eq!(objects.len(), 4, "{objects:?}");
    assert!(objects.iter().any(|o| o == "Block_support"), "{objects:?}");

    // Excluding aids leaves the support out and keeps the decoy.
    let (preview, files, _root) = export(
        &mut f,
        mesh(
            AidPolicy::ExcludeRemovable,
            HierarchyMode::PreserveAssembly,
            MeshFormat::Obj,
        ),
    );
    assert_eq!(preview.recipes[0].status, RecipeStatus::Ready);
    assert_eq!(preview.recipes[0].excluded_aids.len(), 1);
    assert_eq!(preview.recipes[0].excluded_aids[0].part, support_body);
    let objects = obj_objects(&files[0].1);
    assert_eq!(objects.len(), 3, "{objects:?}");
    assert!(!objects.iter().any(|o| o == "Block_support"));
    assert!(objects.iter().any(|o| o == "Support_decoy"));

    // Separate STEP files: one per finished Part, none for the aid.
    let (preview, files, _root) = export(
        &mut f,
        RecipeOutput::Step {
            units: LengthUnit::Millimeter,
            hierarchy: HierarchyMode::SeparateParts,
            names: NameHandling::FileNamesOnly,
            aids: AidPolicy::ExcludeRemovable,
        },
    );
    assert_eq!(files.len(), 3);
    assert_eq!(preview.recipes[0].files, 3);
    assert!(
        preview.artifacts.iter().all(|a| a.output.subject
            != OutputSubject::Part(preview.recipes[0].excluded_aids[0].clone()))
    );
    for (name, bytes) in &files {
        assert!(
            String::from_utf8_lossy(bytes).starts_with("ISO-10303-21"),
            "{name}"
        );
    }

    // A 3MF print package with the aid places it where it was modeled.
    let (_, files, _root) = export(
        &mut f,
        mesh(
            AidPolicy::IncludeAll,
            HierarchyMode::PreserveAssembly,
            MeshFormat::ThreeMf,
        ),
    );
    let model_xml = three_mf_model(&files[0].1);
    assert_eq!(model_xml.matches("<object ").count(), 4);
    assert!(model_xml.contains("Block support"));
}

/// The 3D model XML of a 3MF package, read with a minimal ZIP reader.
fn three_mf_model(bytes: &[u8]) -> String {
    let u16le = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32le = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let eocd = (0..bytes.len() - 21)
        .rev()
        .find(|&i| u32le(i) == 0x0605_4b50)
        .unwrap();
    let (count, mut at) = (u16le(eocd + 10), u32le(eocd + 16));
    for _ in 0..count {
        let (method, size, name_len) = (u16le(at + 10), u32le(at + 20), u16le(at + 28));
        let extra_comment = u16le(at + 30) + u16le(at + 32);
        let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec()).unwrap();
        let local = u32le(at + 42);
        let data_at = local + 30 + u16le(local + 26) + u16le(local + 28);
        if name.ends_with(".model") {
            assert_eq!(method, 0, "3MF model part is stored");
            return String::from_utf8(bytes[data_at..data_at + size].to_vec()).unwrap();
        }
        at += 46 + name_len + extra_comment;
    }
    panic!("no 3MF model part")
}
