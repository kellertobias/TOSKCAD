//! Placing FDM aids on the finished part: the exact tab contact query on a
//! curved and a flat side, its refusals, and the aid inspections a full
//! evaluation records with placement findings kept apart from allowance
//! findings.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, AidDefinition, AidRequest, AidShape, BodyOutput, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, ProcessDefinition, angle, length,
    },
};
use multicad_model::{Compiler, Model};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn add(s: &mut Session, request: &PrimitiveRequest) {
    let p = model::prepare_primitive(s, s.revision(), actor(), request).unwrap();
    s.commit(p.prepared).unwrap();
}
/// The knob from the native review: a radius-8 cylinder at (100, 50, 1),
/// and a 20 × 10 × 5 mm box at (60, 45, 0) 12 mm to its -X side.
fn parts() -> (Session, EntityId, EntityId) {
    let mut s = Session::new(Document::new("FDM contact").unwrap()).unwrap();
    let knob = PrimitiveRequest::new(
        "Knob",
        PrimitiveParameters::Cylinder {
            radius: length(8.),
            height: length(6.),
            frame: PrimitiveFrame {
                origin: [length(100.), length(50.), length(1.)],
                ..Default::default()
            },
        },
    );
    add(&mut s, &knob);
    let block = PrimitiveRequest::new(
        "Block",
        PrimitiveParameters::Box {
            size: [length(20.), length(10.), length(5.)],
            frame: PrimitiveFrame {
                origin: [length(60.), length(45.), length(0.)],
                ..Default::default()
            },
        },
    );
    add(&mut s, &block);
    (s, knob.body, block.body)
}
fn tab(target: EntityId, origin: [f64; 3], direction: f64) -> AidRequest {
    AidRequest::new(
        "Tab",
        AidDefinition {
            output: BodyOutput {
                body: EntityId::nil(),
            },
            target,
            shape: AidShape::Tab {
                origin: origin.map(length),
                angle: angle(direction),
                length: length(8.),
                width: length(6.),
                thickness: length(0.4),
                overlap: length(2.),
            },
            process: None,
        },
    )
}

#[test]
fn a_tab_contact_is_found_on_the_exact_curved_and_flat_sides() {
    let (s, knob, block) = parts();
    let m = compile(&s);
    // A display hit on a chord lies slightly inside the true cylinder; the
    // outward hint moves the probe off it and the exact outline is found.
    let c = m
        .fdm_tab_contact(knob, [107.98, 50.3, 4.], Some([1., 0.03]))
        .unwrap();
    let r = (c.point_mm[0] - 100.).hypot(c.point_mm[1] - 50.);
    near(r, 8., 1e-6);
    near(c.point_mm[2], 1., 1e-9);
    // The outward direction is the radial normal through the contact point.
    let radial = (c.point_mm[1] - 50.)
        .atan2(c.point_mm[0] - 100.)
        .to_degrees();
    near(c.angle_deg, radial, 1e-3);
    near(c.outward[0].hypot(c.outward[1]), 1., 1e-12);
    // The review's hand-computed contact for a +X tab.
    let c = m.fdm_tab_contact(knob, [112., 50., 9.], None).unwrap();
    // The probe is a 0.1 µm cube, so the direction holds to 0.001°.
    near(c.point_mm[0], 108., 1e-6);
    near(c.point_mm[1], 50., 1e-4);
    near(c.angle_deg, 0., 1e-3);
    near(c.probe_distance_mm, 4., 1e-3);
    // A flat side: the -Y face of the block.
    let c = m
        .fdm_tab_contact(block, [67., 45., 2.], Some([0., -1.]))
        .unwrap();
    near(c.point_mm[0], 67., 1e-4);
    near(c.point_mm[1], 45., 1e-6);
    near(c.point_mm[2], 0., 1e-9);
    near(c.angle_deg, -90., 1e-6);
    // Inside the outline (a top-face pick without a hint) is refused.
    let e = m.fdm_tab_contact(block, [67., 50., 5.], None).unwrap_err();
    assert!(e.message.contains("inside the part's outline"), "{e}");
    // A Part without a solid is refused.
    assert!(m.fdm_tab_contact(EntityId::new(), [0.; 3], None).is_err());
}

#[test]
fn a_full_evaluation_records_aid_inspections_with_placement_findings() {
    let (mut s, knob, block) = parts();
    assert!(compile(&s).snapshot().fdm_aids.as_ref().unwrap().is_empty());
    let (process, prepared) = model::prepare_process(
        &s,
        s.revision(),
        actor(),
        None,
        "PLA",
        &ProcessDefinition::default(),
    )
    .unwrap();
    s.commit(prepared).unwrap();
    // A tab placed at the exact contact touches the knob.
    let m = compile(&s);
    let c = m.fdm_tab_contact(knob, [112., 50., 3.], None).unwrap();
    let good = tab(knob, c.point_mm, c.angle_deg);
    let p = model::prepare_aid(&s, s.revision(), actor(), &good).unwrap();
    s.commit(p.prepared).unwrap();
    // A tab 3 mm off the knob misses it; a tab from the knob's -X side
    // runs into the block.
    let mut missing = tab(knob, [111., 50., 1.], 0.);
    missing.name = "Missing".into();
    let p = model::prepare_aid(&s, s.revision(), actor(), &missing).unwrap();
    s.commit(p.prepared).unwrap();
    let mut crowding = tab(knob, [92., 50., 1.], 180.);
    crowding.name = "Crowding".into();
    if let AidShape::Tab { length: l, .. } = &mut crowding.aid.shape {
        *l = length(13.);
    }
    crowding.aid.process = Some(process);
    let p = model::prepare_aid(&s, s.revision(), actor(), &crowding).unwrap();
    s.commit(p.prepared).unwrap();

    let m = compile(&s);
    let list = m.snapshot().fdm_aids.as_ref().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list, &m.fdm_aids(s.document()).unwrap());
    let find = |name: &str| list.iter().find(|a| a.name == name).unwrap();
    let ok = find("Tab");
    assert!(ok.contact_issues.is_empty(), "{:?}", ok.contact_issues);
    // A tab's inspection offers the exact contact near its own.
    let snap = ok.exact_contact.unwrap();
    assert!((snap.point_mm[0] - c.point_mm[0]).abs() < 1e-4);
    let miss = find("Missing");
    let snap = miss.exact_contact.unwrap();
    assert!((snap.point_mm[0] - 108.).abs() < 1e-4, "{snap:?}");
    assert!(snap.angle_deg.abs() < 1e-3);
    assert_eq!(miss.contact_issues.len(), 1);
    assert!(miss.contact_issues[0].contains("does not touch"));
    let crowd = find("Crowding");
    assert_eq!(crowd.crowded.len(), 1);
    assert_eq!(crowd.crowded[0].0, block);
    assert!(
        crowd
            .contact_issues
            .iter()
            .any(|i| i.contains("fit clearance"))
    );
    // Allowance findings are advisory: every placement finding is listed
    // in `issues`, not the other way round.
    for a in list {
        assert!(a.contact_issues.iter().all(|i| a.issues.contains(i)));
    }
}
