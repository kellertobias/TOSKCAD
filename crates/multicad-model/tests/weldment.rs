//! Weldment members: a sketched frame path producing separate tube members,
//! path edits that keep member identities, Undo, save/reopen, section
//! orientation, anchors and hollow walls, and explicit diagnostics. Every
//! size is measured from the native solids.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::*,
    model::{
        self, AxisAnchor, BodyOutput, ConnectionDefinition, ConnectionKind, MemberDefinition,
        MemberPath, MemberRequest, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    sketch::Datum,
};
use multicad_model::{Compiler, MemberRecord, Model};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-6, "{a} != {b}");
}
fn rel(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-7 * b.abs().max(1.), "{a} != {b}");
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
fn insert(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fixture".into(),
        edits,
    })
    .unwrap();
}
fn record<'a>(list: &'a [MemberRecord], name: &str) -> &'a MemberRecord {
    list.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no member record {name}"))
}

pub struct Fixture {
    pub s: Session,
    pub steel: EntityId,
    pub shs: EntityId,
    pub rhs: EntityId,
    pub chs: EntityId,
    pub angle: EntityId,
    pub flat: EntityId,
    pub ply: EntityId,
}
fn profile(
    material: EntityId,
    name: &str,
    shape: ProfileShape,
    w: f64,
    h: f64,
    wall: Option<f64>,
) -> Entity {
    LibraryDefinition::Stock(Stock {
        material,
        specification: StockKind::Profile {
            designation: name.into(),
            length_mm: 6000.,
            width_mm: w,
            height_mm: h,
            wall_mm: wall,
            shape,
        },
    })
    .entity(EntityId::new(), name)
    .unwrap()
}
pub fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Weldment").unwrap()).unwrap();
    let steel = Entity::new(
        "S235 steel",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [120, 124, 130, 255],
            section_hatch: None,
        },
    );
    let birch = Entity::new(
        "Birch",
        EntityData::Material {
            density_kg_m3: Some(650.),
            color_rgba: [214, 186, 140, 255],
            section_hatch: None,
        },
    );
    let shs = profile(
        steel.id,
        "SHS 40×40×2",
        ProfileShape::Rectangular,
        40.,
        40.,
        Some(2.),
    );
    let rhs = profile(
        steel.id,
        "RHS 60×40×3",
        ProfileShape::Rectangular,
        60.,
        40.,
        Some(3.),
    );
    let chs = profile(
        steel.id,
        "CHS 33.7×2.6",
        ProfileShape::Round,
        33.7,
        33.7,
        Some(2.6),
    );
    let angle = profile(
        steel.id,
        "L 40×30×4",
        ProfileShape::Angle,
        40.,
        30.,
        Some(4.),
    );
    let flat = profile(
        steel.id,
        "Flat 40×8",
        ProfileShape::Rectangular,
        40.,
        8.,
        None,
    );
    let ply = LibraryDefinition::Stock(Stock {
        material: birch.id,
        specification: StockKind::Panel {
            thickness_mm: 18.,
            length_mm: 2500.,
            width_mm: 1250.,
            grain: Grain::AlongLength,
        },
    })
    .entity(EntityId::new(), "Birch ply 18")
    .unwrap();
    let ids = [steel.id, shs.id, rhs.id, chs.id, angle.id, flat.id, ply.id];
    s.commit(prepare_library(&s, 0, actor(), &[steel, birch]).unwrap())
        .unwrap();
    s.commit(
        prepare_library(
            &s,
            s.revision(),
            actor(),
            &[shs, rhs, chs, angle, flat, ply],
        )
        .unwrap(),
    )
    .unwrap();
    Fixture {
        s,
        steel: ids[0],
        shs: ids[1],
        rhs: ids[2],
        chs: ids[3],
        angle: ids[4],
        flat: ids[5],
        ply: ids[6],
    }
}
pub fn template(stock: EntityId) -> MemberDefinition {
    MemberDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        stock,
        path: MemberPath::Points {
            start: [length(0.), length(0.), length(0.)],
            end: [length(1.), length(0.), length(0.)],
        },
        anchor: [AxisAnchor::Center; 2],
        rotation: None,
        extend_start: None,
        extend_end: None,
    }
}
pub fn points(stock: EntityId, a: [f64; 3], b: [f64; 3]) -> MemberDefinition {
    MemberDefinition {
        path: MemberPath::Points {
            start: a.map(length),
            end: b.map(length),
        },
        ..template(stock)
    }
}
/// A sketch of open polyline/loop segments on `datum` with shared corners;
/// returns the sketch, its point IDs and its line IDs.
pub fn path_sketch(
    s: &mut Session,
    datum: Datum,
    corners: &[[f64; 2]],
    closed: bool,
) -> (EntityId, Vec<EntityId>, Vec<EntityId>) {
    let (sketch, edits) = Source::create(datum, "Frame path").unwrap();
    insert(s, edits);
    let mut source = Source::read(s.document(), sketch).unwrap().0;
    let points: Vec<_> = corners.iter().map(|c| source.add_point(*c)).collect();
    let n = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let lines = (0..n)
        .map(|i| source.add_line(points[i], points[(i + 1) % points.len()]))
        .collect();
    let edits = source.replace(s.document(), sketch).unwrap();
    insert(s, edits);
    (sketch, points, lines)
}
/// Move sketch points to new positions in one checked transaction.
pub fn move_points(s: &mut Session, sketch: EntityId, moves: &[(EntityId, [f64; 2])]) {
    let mut source = Source::read(s.document(), sketch).unwrap().0;
    for (id, at) in moves {
        source
            .sketch
            .points
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap()
            .position_mm = *at;
    }
    let edits = source.replace(s.document(), sketch).unwrap();
    let prepared = s
        .prepare(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Edit path".into(),
            edits,
        })
        .unwrap();
    checked(s, prepared);
}
fn bounds(m: &Model, part: EntityId) -> multicad_kernel::Bounds {
    m.solid(m.snapshot().bodies[&part])
        .unwrap()
        .bounds()
        .unwrap()
}
const SHS_AREA: f64 = 40. * 40. - 36. * 36.;

#[test]
fn sketched_frame_path_produces_separate_members_that_follow_path_edits() {
    let mut f = fixture();
    let (sketch, pts, lines) = path_sketch(
        &mut f.s,
        Datum::Xy,
        &[[0., 0.], [1000., 0.], [1000., 600.], [0., 600.]],
        true,
    );
    let requests =
        model::frame_member_requests(f.s.document(), sketch, "Top rail", &template(f.shs)).unwrap();
    assert_eq!(requests.len(), 4);
    let p = model::prepare_members(
        &f.s,
        f.s.revision(),
        actor(),
        &requests,
        Some("Table frame"),
    )
    .unwrap();
    assert_eq!(p.outputs.len(), 4);
    checked(&mut f.s, p.prepared);
    let m = compile(&f.s);
    let list = m.member_cut_list(f.s.document()).unwrap();
    assert_eq!(list.len(), 4);
    // Four independent Parts, one per sketch line, each with the steel
    // material and the SHS stock assignment.
    let parts: std::collections::BTreeSet<_> = list.iter().map(|r| r.part).collect();
    assert_eq!(parts.len(), 4);
    for (i, line) in lines.iter().enumerate() {
        let r = record(&list, &format!("Top rail {}", i + 1));
        assert!(r.issues.is_empty(), "{}: {:?}", r.name, r.issues);
        assert_eq!(r.material, Some(f.steel));
        assert_eq!(r.stock, f.shs);
        assert_eq!(r.designation, "SHS 40×40×2");
        assert!(matches!(r.path, MemberPath::SketchLine { line: l, .. } if l == *line));
        assert!(r.fits_stock);
        near(r.measured_section_mm[0], 40.);
        near(r.measured_section_mm[1], 40.);
    }
    let rail1 = record(&list, "Top rail 1");
    let rail2 = record(&list, "Top rail 2");
    near(rail1.length_mm, 1000.);
    near(rail2.length_mm, 600.);
    // The hollow wall is real: volume is the tube area times its length.
    rel(rail1.volume_mm3, SHS_AREA * 1000.);
    // The section is centered on the path, its height along the sketch normal.
    let b = bounds(&m, rail1.part);
    near(b.min[1], -20.);
    near(b.max[1], 20.);
    near(b.min[2], -20.);
    near(b.max[2], 20.);
    near(b.min[0], 0.);
    near(b.max[0], 1000.);
    let group =
        f.s.document()
            .hierarchy()
            .members
            .iter()
            .find_map(|(g, m)| {
                (f.s.document()
                    .get(*g)
                    .is_some_and(|e| e.name == "Table frame"))
                .then_some(m)
            });
    assert!(group.unwrap().contains(&rail1.part));

    // Editing the path lengthens two members in place; identities stay.
    let before = f.s.document().clone();
    move_points(
        &mut f.s,
        sketch,
        &[(pts[1], [1200., 0.]), (pts[2], [1200., 600.])],
    );
    let m = compile(&f.s);
    let edited = m.member_cut_list(f.s.document()).unwrap();
    assert_eq!(record(&edited, "Top rail 1").part, rail1.part);
    assert_eq!(record(&edited, "Top rail 1").feature, rail1.feature);
    near(record(&edited, "Top rail 1").length_mm, 1200.);
    near(record(&edited, "Top rail 3").length_mm, 1200.);
    near(record(&edited, "Top rail 2").length_mm, 600.);
    near(bounds(&m, record(&edited, "Top rail 2").part).min[0], 1180.);
    // Undo restores the source exactly.
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &before);

    // Save and reopen keep editable members and identical records.
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
    assert!(opened.capabilities().contains(model::MEMBER_CAPABILITY));
    assert_eq!(&opened.document, f.s.document());
    let reopened = Compiler::default()
        .compile(&opened.document, 0)
        .unwrap()
        .member_cut_list(&opened.document)
        .unwrap();
    assert_eq!(reopened, list);
    // An older reader without the capability refuses the file.
    let mut older = project_file::supported_capabilities();
    older.remove(model::MEMBER_CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &older).is_err());
}

#[test]
fn section_orientation_anchor_and_walls_are_modeled_and_recorded() {
    let mut f = fixture();
    let leg = MemberRequest::new("Leg", points(f.rhs, [0., 0., 0.], [0., 0., 500.]));
    let mut turned = points(f.rhs, [200., 0., 0.], [200., 0., 500.]);
    turned.rotation = Some(angle(90.));
    let turned = MemberRequest::new("Turned leg", turned);
    let mut corner = points(f.rhs, [400., 0., 0.], [400., 0., 500.]);
    corner.anchor = [AxisAnchor::Min, AxisAnchor::Max];
    let corner = MemberRequest::new("Corner leg", corner);
    let brace = MemberRequest::new("Brace", points(f.chs, [0., 300., 0.], [800., 300., 0.]));
    let mut ledge = points(f.angle, [0., 600., 0.], [800., 600., 0.]);
    ledge.anchor = [AxisAnchor::Min, AxisAnchor::Min];
    ledge.extend_start = Some(length(10.));
    ledge.extend_end = Some(length(-5.));
    let ledge = MemberRequest::new("Ledge", ledge);
    let bar = MemberRequest::new("Bar", points(f.flat, [0., 900., 0.], [0., 900., 250.]));
    let requests = [leg, turned, corner, brace, ledge, bar];
    let p = model::prepare_members(&f.s, f.s.revision(), actor(), &requests, None).unwrap();
    checked(&mut f.s, p.prepared);
    let m = compile(&f.s);
    let list = m.member_cut_list(f.s.document()).unwrap();

    // A vertical member: height along +Y, width along +X, centered.
    let b = bounds(&m, requests[0].body);
    near(b.min[0], -30.);
    near(b.max[0], 30.);
    near(b.min[1], -20.);
    near(b.max[1], 20.);
    near(b.max[2], 500.);
    let leg = record(&list, "Leg");
    rel(leg.volume_mm3, (60. * 40. - 54. * 34.) * 500.);
    assert_eq!(leg.frame.x, [1., 0., 0.]);
    // A quarter turn puts the 60 mm width along +Y; the record says so.
    let b = bounds(&m, requests[1].body);
    near(b.max[0] - b.min[0], 40.);
    near(b.max[1] - b.min[1], 60.);
    let turned = record(&list, "Turned leg");
    near(turned.rotation_deg, 90.);
    near(turned.frame.x[1], 1.);
    near(turned.measured_section_mm[0], 60.);
    // Min/Max anchor: the path runs along the section's minimum-x,
    // maximum-y edge.
    let b = bounds(&m, requests[2].body);
    near(b.min[0], 400.);
    near(b.max[0], 460.);
    near(b.min[1], -40.);
    near(b.max[1], 0.);
    // Round tube keeps its bore.
    let brace = record(&list, "Brace");
    let (r, t) = (33.7 / 2., 2.6);
    rel(
        brace.volume_mm3,
        std::f64::consts::PI * (r * r - (r - t) * (r - t)) * 800.,
    );
    assert_eq!(brace.shape, ProfileShape::Round);
    // Angle: heel on the path, legs along +Y (width) and +Z (height);
    // extensions lengthen the start and trim the end.
    let ledge = record(&list, "Ledge");
    near(ledge.length_mm, 805.);
    near(ledge.nominal_length_mm, 805.);
    rel(ledge.volume_mm3, (40. * 4. + 26. * 4.) * 805.);
    let b = bounds(&m, requests[4].body);
    near(b.min[0], -10.);
    near(b.max[0], 795.);
    near(b.min[1], 600.);
    near(b.max[1], 640.);
    near(b.min[2], 0.);
    near(b.max[2], 30.);
    // A solid flat bar has no bore.
    rel(record(&list, "Bar").volume_mm3, 40. * 8. * 250.);

    // A stock change moves the material assignment; a parameter drives the
    // path length.
    let mut width = Entity::new(
        "leg_height",
        EntityData::Parameter {
            expression: "720".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    width.description = "Leg height".into();
    let parameter = width.id;
    insert(&mut f.s, vec![Mutation::Insert(width)]);
    let mut edit = model::read_member(f.s.document(), requests[0].feature).unwrap();
    edit.stock = f.shs;
    edit.path = MemberPath::Points {
        start: [length(0.), length(0.), length(0.)],
        end: [
            length(0.),
            length(0.),
            Dimension::Expression {
                expression: BoundExpression::bind(f.s.document(), "=$leg_height", "mm", None)
                    .unwrap(),
            },
        ],
    };
    let p = model::prepare_member_edit(&f.s, f.s.revision(), actor(), requests[0].feature, &edit)
        .unwrap();
    checked(&mut f.s, p.prepared);
    let list = compile(&f.s).member_cut_list(f.s.document()).unwrap();
    let leg = record(&list, "Leg");
    assert_eq!(leg.stock, f.shs);
    assert!(leg.issues.is_empty(), "{:?}", leg.issues);
    near(leg.length_mm, 720.);
    let props = part_properties(f.s.document(), requests[0].body)
        .unwrap()
        .unwrap()
        .1;
    assert_eq!(props.stock.unwrap().id, f.shs);
    let mut p = f.s.document().get(parameter).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut p.data else {
        unreachable!()
    };
    *expression = "900".into();
    let prepared =
        f.s.prepare(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Edit parameter".into(),
            edits: vec![Mutation::Replace(p)],
        })
        .unwrap();
    checked(&mut f.s, prepared);
    let list = compile(&f.s).member_cut_list(f.s.document()).unwrap();
    near(record(&list, "Leg").length_mm, 900.);
    assert!(record(&list, "Leg").fits_stock);
}

#[test]
fn invalid_paths_and_stock_are_diagnosed() {
    let mut f = fixture();
    let rev = f.s.revision();
    let e = |r: Result<model::ModelPrepared, multicad_core::commands::CommandError>| {
        r.unwrap_err().message
    };
    let one = |m: MemberDefinition| vec![MemberRequest::new("M", m)];
    let msg = e(model::prepare_members(
        &f.s,
        rev,
        actor(),
        &one(points(f.shs, [5., 5., 5.], [5., 5., 5.])),
        None,
    ));
    assert!(msg.contains("zero length"), "{msg}");
    let msg = e(model::prepare_members(
        &f.s,
        rev,
        actor(),
        &one(points(f.ply, [0., 0., 0.], [100., 0., 0.])),
        None,
    ));
    assert!(msg.contains("not profile stock"), "{msg}");
    let mut trimmed = points(f.shs, [0., 0., 0.], [100., 0., 0.]);
    trimmed.extend_end = Some(length(-100.));
    let msg = e(model::prepare_members(
        &f.s,
        rev,
        actor(),
        &one(trimmed),
        None,
    ));
    assert!(msg.contains("trim away"), "{msg}");
    // Round stock must be round.
    let oval = LibraryDefinition::Stock(Stock {
        material: f.steel,
        specification: StockKind::Profile {
            designation: "Oval".into(),
            length_mm: 6000.,
            width_mm: 40.,
            height_mm: 30.,
            wall_mm: Some(2.),
            shape: ProfileShape::Round,
        },
    });
    assert!(oval.entity(EntityId::new(), "Oval").is_err());

    // Sketch paths: construction lines, arcs, duplicates and deleted lines.
    let (sketch, _, lines) = path_sketch(
        &mut f.s,
        Datum::Xz,
        &[[0., 0.], [500., 0.], [500., 400.]],
        false,
    );
    let mut source = Source::read(f.s.document(), sketch).unwrap().0;
    source
        .sketch
        .curves
        .iter_mut()
        .find(|c| c.id == lines[1])
        .unwrap()
        .construction = true;
    let edits = source.replace(f.s.document(), sketch).unwrap();
    insert(&mut f.s, edits);
    let on = |line| MemberDefinition {
        path: MemberPath::SketchLine { sketch, line },
        ..template(f.shs)
    };
    let msg = e(model::prepare_members(
        &f.s,
        f.s.revision(),
        actor(),
        &one(on(lines[1])),
        None,
    ));
    assert!(msg.contains("construction line"), "{msg}");
    assert_eq!(
        model::sketch_path_lines(f.s.document(), sketch).unwrap(),
        vec![lines[0]]
    );
    let twice = [
        MemberRequest::new("A", on(lines[0])),
        MemberRequest::new("B", on(lines[0])),
    ];
    let msg = e(model::prepare_members(
        &f.s,
        f.s.revision(),
        actor(),
        &twice,
        None,
    ));
    assert!(msg.contains("more than one"), "{msg}");
    let p =
        model::prepare_members(&f.s, f.s.revision(), actor(), &one(on(lines[0])), None).unwrap();
    checked(&mut f.s, p.prepared);
    // The sketch on XZ: the member's height follows the sketch normal (±Y).
    let list = compile(&f.s).member_cut_list(f.s.document()).unwrap();
    near(list[0].frame.y[1].abs(), 1.);
    // Deleting the path line is reported by the native evaluator.
    let mut source = Source::read(f.s.document(), sketch).unwrap().0;
    source.sketch.curves.retain(|c| c.id != lines[0]);
    let edits = source.replace(f.s.document(), sketch).unwrap();
    let prepared =
        f.s.prepare(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Delete path".into(),
            edits,
        })
        .unwrap();
    let err = Compiler::default()
        .compile(prepared.document(), f.s.revision() + 1)
        .err()
        .unwrap();
    assert!(err.message.contains("no longer exists"), "{}", err.message);
}

// ---- Connections -----------------------------------------------------------

fn create(f: &mut Fixture, requests: &[MemberRequest]) {
    let p = model::prepare_members(&f.s, f.s.revision(), actor(), requests, None).unwrap();
    checked(&mut f.s, p.prepared);
}
fn connect(
    f: &mut Fixture,
    name: &str,
    kind: ConnectionKind,
    members: [EntityId; 2],
    gap: f64,
) -> EntityId {
    let (id, p) = model::prepare_connection(
        &f.s,
        f.s.revision(),
        actor(),
        name,
        &ConnectionDefinition {
            kind,
            members,
            gap: (gap != 0.).then(|| length(gap)),
        },
    )
    .unwrap();
    checked(&mut f.s, p);
    id
}
/// Stage a connection and return the native evaluator's refusal.
fn refused(f: &Fixture, kind: ConnectionKind, members: [EntityId; 2]) -> String {
    let (_, p) = model::prepare_connection(
        &f.s,
        f.s.revision(),
        actor(),
        "Refused",
        &ConnectionDefinition {
            kind,
            members,
            gap: None,
        },
    )
    .unwrap();
    Compiler::default()
        .compile(p.document(), f.s.revision() + 1)
        .err()
        .expect("the connection should be refused")
        .message
}

#[test]
fn paired_miter_follows_the_meeting_angle_and_records_both_end_angles() {
    let mut f = fixture();
    let (sketch, pts, _) = path_sketch(
        &mut f.s,
        Datum::Xy,
        &[[0., 0.], [800., 0.], [800., 500.]],
        false,
    );
    let requests =
        model::frame_member_requests(f.s.document(), sketch, "Rail", &template(f.shs)).unwrap();
    create(&mut f, &requests);
    let (a, b) = (requests[0].body, requests[1].body);
    let before = f.s.document().clone();
    let miter = connect(&mut f, "Corner", ConnectionKind::Miter, [a, b], 0.);
    let m = compile(&f.s);
    let list = m.member_cut_list(f.s.document()).unwrap();
    let (ra, rb) = (record(&list, "Rail 1"), record(&list, "Rail 2"));
    // A right-angle miter: both ends cut 45° from square, long points at
    // the outside corner, and the centerline length kept.
    for (r, partner) in [(ra, b), (rb, a)] {
        assert!(r.issues.is_empty(), "{:?}", r.issues);
        assert_eq!(r.ends[0].kind.is_some(), r.name == "Rail 2");
        let joint = &r.ends[usize::from(r.name == "Rail 1")];
        assert_eq!(joint.kind, Some(ConnectionKind::Miter));
        assert_eq!(joint.connection, Some(miter));
        assert_eq!(joint.partner, Some(partner));
        near(joint.angle_deg, 45.);
        assert!(joint.terminates);
    }
    near(ra.length_mm, 820.);
    near(rb.length_mm, 520.);
    rel(ra.volume_mm3, SHS_AREA * 800.);
    rel(rb.volume_mm3, SHS_AREA * 500.);
    near(bounds(&m, a).max[0], 820.);
    near(bounds(&m, b).min[1], -20.);
    // The two members touch on the miter plane without overlapping.
    let solid = |p: EntityId| m.solid(m.snapshot().bodies[&p]).unwrap();
    let overlap = solid(a)
        .boolean(
            solid(b),
            multicad_kernel::BooleanOperation::Intersect,
            Default::default(),
        )
        .unwrap();
    assert!(overlap.is_none_or(|s| s.volume_mm3().unwrap() < 1e-6));

    // Opening the corner to 135° updates both cuts and both records.
    move_points(&mut f.s, sketch, &[(pts[2], [1300., 500.])]);
    let m = compile(&f.s);
    let list = m.member_cut_list(f.s.document()).unwrap();
    let (ra, rb) = (record(&list, "Rail 1"), record(&list, "Rail 2"));
    near(ra.ends[1].angle_deg, 22.5);
    near(rb.ends[0].angle_deg, 22.5);
    near(ra.length_mm, 800. + 20. * 22.5_f64.to_radians().tan());
    rel(ra.volume_mm3, SHS_AREA * 800.);
    rel(rb.volume_mm3, SHS_AREA * 500. * 2_f64.sqrt());

    // A 2 mm weld gap is shared by both members along the miter normal.
    let mut gapped = model::read_connection(f.s.document(), miter).unwrap();
    gapped.gap = Some(length(2.));
    let p = model::prepare_connection_edit(&f.s, f.s.revision(), actor(), miter, &gapped).unwrap();
    checked(&mut f.s, p);
    let list = compile(&f.s).member_cut_list(f.s.document()).unwrap();
    let shift = 1. / 22.5_f64.to_radians().cos();
    rel(
        record(&list, "Rail 1").volume_mm3,
        SHS_AREA * (800. - shift),
    );

    // Save and reopen keep the connection and the measured records.
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
    assert!(opened.capabilities().contains(model::CONNECTION_CAPABILITY));
    let reopened = Compiler::default()
        .compile(&opened.document, 0)
        .unwrap()
        .member_cut_list(&opened.document)
        .unwrap();
    assert_eq!(reopened, list);
    // Undo back to the unconnected members restores square ends.
    while f.s.document() != &before {
        f.s.undo(f.s.revision(), actor()).unwrap();
    }
    let list = compile(&f.s).member_cut_list(f.s.document()).unwrap();
    assert!(list.iter().all(|r| r.ends.iter().all(|e| e.kind.is_none())));
}

#[test]
fn butt_joints_identify_the_continuing_member_and_trim_square() {
    let mut f = fixture();
    let rail = MemberRequest::new("Rail", points(f.rhs, [0., 0., 0.], [1000., 0., 0.]));
    let post = MemberRequest::new("Post", points(f.shs, [500., 0., 0.], [500., 600., 0.]));
    let end_rail = MemberRequest::new("End rail", points(f.rhs, [2000., 0., 0.], [3000., 0., 0.]));
    let end_post = MemberRequest::new(
        "End post",
        points(f.shs, [3000., 0., 0.], [3000., 600., 0.]),
    );
    let slanted = MemberRequest::new("Slant", points(f.shs, [200., 0., 0.], [500., 300., 0.]));
    let brace = MemberRequest::new("Brace", points(f.chs, [0., 2000., 0.], [800., 2000., 0.]));
    let stub = MemberRequest::new("Stub", points(f.shs, [400., 2000., 0.], [400., 2400., 0.]));
    let all = [rail, post, end_rail, end_post, slanted, brace, stub];
    create(&mut f, &all);
    let id = |i: usize| all[i].body;
    // T joint: the post stops 1 mm short of the rail's 30 mm face.
    let tee = connect(&mut f, "Tee", ConnectionKind::Butt, [id(1), id(0)], 1.);
    // Corner: the end post butts the end rail, which is cut square to cover it.
    let corner = connect(&mut f, "Corner", ConnectionKind::Butt, [id(3), id(2)], 0.);
    let m = compile(&f.s);
    let list = m.member_cut_list(f.s.document()).unwrap();
    let post = record(&list, "Post");
    near(post.span_mm[0], 31.);
    near(post.length_mm, 569.);
    rel(post.volume_mm3, SHS_AREA * 569.);
    assert_eq!(post.ends[0].kind, Some(ConnectionKind::Butt));
    assert_eq!(post.ends[0].partner, Some(id(0)));
    assert_eq!(post.ends[0].connection, Some(tee));
    near(post.ends[0].angle_deg, 0.);
    assert!(post.ends[0].terminates);
    let rail = record(&list, "Rail");
    assert!(rail.ends.iter().all(|e| e.kind.is_none()));
    rel(rail.volume_mm3, (2400. - 54. * 34.) * 1000.);
    let end_rail = record(&list, "End rail");
    near(end_rail.length_mm, 1020.);
    assert_eq!(end_rail.ends[1].kind, Some(ConnectionKind::Butt));
    assert!(!end_rail.ends[1].terminates);
    assert_eq!(end_rail.ends[1].connection, Some(corner));
    near(bounds(&m, id(2)).max[0], 3020.);
    near(record(&list, "End post").span_mm[0], 30.);

    // A slanted member is not square to the face; round members have none.
    let msg = refused(&f, ConnectionKind::Butt, [id(4), id(0)]);
    assert!(msg.contains("square to a flat face"), "{msg}");
    let msg = refused(&f, ConnectionKind::Butt, [id(6), id(5)]);
    assert!(msg.contains("cope"), "{msg}");
    // Different sections cannot be mitered with a bisector cut.
    let mitered = MemberRequest::new("Mitered", points(f.shs, [0., 3000., 0.], [500., 3000., 0.]));
    let other = MemberRequest::new("Other", points(f.rhs, [500., 3000., 0.], [500., 3500., 0.]));
    create(&mut f, &[mitered.clone(), other.clone()]);
    let msg = refused(&f, ConnectionKind::Miter, [mitered.body, other.body]);
    assert!(msg.contains("mirror"), "{msg}");
    // The post's start end is already cut by the tee.
    let crossing = MemberRequest::new("Crossing", points(f.rhs, [300., 0., 0.], [800., 0., 0.]));
    create(&mut f, std::slice::from_ref(&crossing));
    let msg = refused(&f, ConnectionKind::Butt, [id(1), crossing.body]);
    assert!(msg.contains("both cut the start of"), "{msg}");
}

#[test]
fn copes_fit_the_actual_receiving_member_with_clearance() {
    let mut f = fixture();
    let r: f64 = 33.7 / 2.;
    let (c, s) = (60_f64.to_radians().cos(), 60_f64.to_radians().sin());
    let main = MemberRequest::new("Main", points(f.chs, [0., 0., 0.], [600., 0., 0.]));
    let branch = MemberRequest::new("Branch", points(f.chs, [300., 0., 0.], [300., 0., 400.]));
    let angled = MemberRequest::new(
        "Angled",
        points(f.chs, [100., 0., 0.], [100. + 300. * c, 0., 300. * s]),
    );
    let beam = MemberRequest::new("Beam", points(f.rhs, [0., 1000., 0.], [1000., 1000., 0.]));
    let stud = MemberRequest::new("Stud", points(f.shs, [500., 1000., 0.], [500., 1500., 0.]));
    let wide = MemberRequest::new("Wide", points(f.rhs, [500., 0., 0.], [500., -300., 0.]));
    let skew = MemberRequest::new("Skew", points(f.chs, [200., 100., 50.], [200., 400., 50.]));
    let all = [main, branch, angled, beam, stud, wide, skew];
    create(&mut f, &all);
    let id = |i: usize| all[i].body;
    let uncoped = record(
        &compile(&f.s).member_cut_list(f.s.document()).unwrap(),
        "Branch",
    )
    .volume_mm3;
    // A fish-mouth cope of a round branch onto an equal round tube, 1 mm clear.
    connect(&mut f, "Saddle", ConnectionKind::Cope, [id(1), id(0)], 1.);
    connect(
        &mut f,
        "Angled saddle",
        ConnectionKind::Cope,
        [id(2), id(0)],
        0.,
    );
    // A square stud onto a rectangular beam: the cope is a square notch at
    // the beam face.
    connect(
        &mut f,
        "Stud notch",
        ConnectionKind::Cope,
        [id(4), id(3)],
        0.,
    );
    let m = compile(&f.s);
    let list = m.member_cut_list(f.s.document()).unwrap();
    let b = bounds(&m, id(1));
    // The saddle reaches down the sides of the clearance envelope.
    near(b.min[2], ((r + 1.).powi(2) - r * r).sqrt());
    near(b.max[2], 400.);
    let branch = record(&list, "Branch");
    assert!(branch.volume_mm3 < uncoped);
    assert_eq!(branch.ends[0].kind, Some(ConnectionKind::Cope));
    near(branch.ends[0].angle_deg, 90.);
    assert_eq!(branch.ends[0].partner, Some(id(0)));
    // Nothing of either branch is left inside the receiving tube.
    let solid = |p: EntityId| m.solid(m.snapshot().bodies[&p]).unwrap();
    for coped in [id(1), id(2)] {
        let overlap = solid(coped)
            .boolean(
                solid(id(0)),
                multicad_kernel::BooleanOperation::Intersect,
                Default::default(),
            )
            .unwrap();
        assert!(overlap.is_none_or(|s| s.volume_mm3().unwrap() < 1e-6));
    }
    near(record(&list, "Angled").ends[0].angle_deg, 60.);
    let stud = record(&list, "Stud");
    near(stud.span_mm[0], 30.);
    rel(stud.volume_mm3, SHS_AREA * 470.);
    // The receiving member is unchanged.
    let main = record(&list, "Main");
    assert!(main.ends.iter().all(|e| e.kind.is_none()));

    // A branch wider than the receiving tube, and members that pass each
    // other without meeting, are refused.
    let msg = refused(&f, ConnectionKind::Cope, [id(5), id(0)]);
    assert!(msg.contains("wide"), "{msg}");
    let msg = refused(&f, ConnectionKind::Cope, [id(6), id(0)]);
    assert!(msg.contains("do not meet"), "{msg}");
    // Only member Parts can be connected.
    let e = model::prepare_connection(
        &f.s,
        f.s.revision(),
        actor(),
        "Bad",
        &ConnectionDefinition {
            kind: ConnectionKind::Cope,
            members: [id(1), f.steel],
            gap: None,
        },
    )
    .unwrap_err();
    assert!(e.message.contains("not a weldment member"), "{}", e.message);
}
