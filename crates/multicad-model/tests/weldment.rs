//! Weldment members: a sketched frame path producing separate tube members,
//! path edits that keep member identities, Undo, save/reopen, section
//! orientation, anchors and hollow walls, and explicit diagnostics.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::{
        AnnotationKind, DrawingView, Page, Scale, ViewKind, ViewSource, WeldSide, WeldSymbol,
        prepare_create, read, svg, view_coordinates,
    },
    materials::*,
    model::{
        self, AxisAnchor, BodyOutput, ConnectionDefinition, ConnectionKind, MemberDefinition,
        MemberPath, MemberRequest, SequenceDefinition, SequenceStep, WeldDefinition, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
    sketch::Datum,
};
use multicad_kernel::{GeometryPolicy, MeshOptions, Solid};
use multicad_model::{Compiler, GeometryBackend, MemberRecord, Model};

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
        // The long point is on the outside of the corner: −Y for the rail
        // along X, +X for the rail along Y: the −x (width) face of both.
        let long = joint.long_point.unwrap();
        let outside = if r.name == "Rail 1" {
            [0., -1., 0.]
        } else {
            [1., 0., 0.]
        };
        for (got, want) in long.direction.into_iter().zip(outside) {
            near(got, want);
        }
        assert_eq!(long.face, "-x");
        // The square far end has no long point.
        assert!(r.ends[usize::from(r.name == "Rail 2")].long_point.is_none());
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
fn frame_from_a_parameter_dimensioned_sketch_follows_the_parameter() {
    use multicad_core::constraint_sketch::{
        ConstraintKind,
        binding::{self, DimensionInput},
    };
    let mut f = fixture();
    let width = Entity::new(
        "frame_width",
        EntityData::Parameter {
            expression: "900".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let width_id = width.id;
    insert(&mut f.s, vec![Mutation::Insert(width)]);
    let (sketch, edits) = Source::create(Datum::Xy, "Frame path").unwrap();
    insert(&mut f.s, edits);
    let mut source = Source::read(f.s.document(), sketch).unwrap().0;
    let bottom = source.add_rectangle([0., 0.], [900., 500.]).unwrap();
    source.add_constraint(ConstraintKind::Fix {
        point: source.sketch.points[0].id,
    });
    let along = source.add_constraint(ConstraintKind::Length {
        line: bottom,
        dimension: length(900.),
    });
    source.add_constraint(ConstraintKind::Length {
        line: source.sketch.curves[1].id,
        dimension: length(500.),
    });
    let source = binding::draft_dimension(
        f.s.document(),
        &source,
        along,
        &DimensionInput::Expression {
            text: "=$frame_width".into(),
            unit: "mm".into(),
        },
        false,
    )
    .unwrap();
    let edit =
        multicad_solver::prepare_edit(&f.s, f.s.revision(), actor(), sketch, source).unwrap();
    f.s.commit(edit.preview.unwrap()).unwrap();
    // A sketch with a parameter-bound dimension has no raw authored
    // geometry, but its lines still become members.
    assert!(Source::read(f.s.document(), sketch).is_err());
    let rails =
        model::frame_member_requests(f.s.document(), sketch, "Rail", &template(f.shs)).unwrap();
    assert_eq!(rails.len(), 4);
    create(&mut f, &rails);
    let length_of = |s: &Session| {
        record(&compile(s).member_cut_list(s.document()).unwrap(), "Rail 1").length_mm
    };
    near(length_of(&f.s), 900.);
    let mut w = f.s.document().get(width_id).unwrap().clone();
    if let EntityData::Parameter { expression, .. } = &mut w.data {
        *expression = "1200".into();
    }
    let p =
        f.s.prepare(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Wider".into(),
            edits: vec![Mutation::Replace(w)],
        })
        .unwrap();
    checked(&mut f.s, p);
    near(length_of(&f.s), 1200.);
}

#[test]
fn long_points_tell_parallel_from_trapezoid_miters() {
    let mut f = fixture();
    // A Z path: the middle rail turns left at one end and right at the
    // other, so its two miters have long points on opposite faces.
    let (sketch, ..) = path_sketch(
        &mut f.s,
        Datum::Xy,
        &[[0., 0.], [800., 0.], [800., 500.], [1600., 500.]],
        false,
    );
    let rails =
        model::frame_member_requests(f.s.document(), sketch, "Rail", &template(f.shs)).unwrap();
    create(&mut f, &rails);
    for i in 0..2 {
        connect(
            &mut f,
            &format!("Corner {}", i + 1),
            ConnectionKind::Miter,
            [rails[i].body, rails[i + 1].body],
            0.,
        );
    }
    let list = compile(&f.s).member_cut_list(f.s.document()).unwrap();
    let middle = record(&list, "Rail 2");
    let [start, end] = middle.ends.each_ref().map(|e| e.long_point.unwrap());
    assert_eq!(start.direction.map(f64::round), [1., 0., 0.]);
    assert_eq!(end.direction.map(f64::round), [-1., 0., 0.]);
    near(start.direction[0], 1.);
    near(end.direction[0], -1.);
    assert_ne!(start.face, end.face);
    let groups = multicad_model::group_cuts(&list);
    let g = groups.iter().find(|g| g.names == ["Rail 2"]).unwrap();
    assert_eq!(g.long_points, Some("opposite faces"));
    // The outer rails have one angled end each and no relation.
    assert!(
        groups
            .iter()
            .filter(|g| g.names != ["Rail 2"])
            .all(|g| g.long_points.is_none())
    );
}

#[test]
fn rust_short_parallel_miters_do_not_require_a_section_between_the_caps() {
    let mut f = fixture();
    let (sketch, ..) = path_sketch(
        &mut f.s,
        Datum::Xy,
        &[[0., -100.], [0., 0.], [10., 0.], [10., 100.]],
        false,
    );
    let rails =
        model::frame_member_requests(f.s.document(), sketch, "Rail", &template(f.shs)).unwrap();
    create(&mut f, &rails);
    for pair in rails.windows(2) {
        connect(
            &mut f,
            "Parallel miter",
            ConnectionKind::Miter,
            [pair[0].body, pair[1].body],
            0.,
        );
    }
    let rust = Compiler::default()
        .compile_with_backend(f.s.document(), f.s.revision(), GeometryBackend::Rust, None)
        .unwrap();
    let native = compile(&f.s);
    let middle = rails[1].body;
    let rust_bounds = bounds(&rust, middle);
    let native_bounds = bounds(&native, middle);
    for i in 0..3 {
        near(rust_bounds.min[i], native_bounds.min[i]);
        near(rust_bounds.max[i], native_bounds.max[i]);
    }
    let list = rust.member_cut_list(f.s.document()).unwrap();
    let middle = record(&list, "Rail 2");
    near(middle.length_mm, 50.);
    rel(middle.volume_mm3, SHS_AREA * 10.);
    assert!(
        middle
            .ends
            .iter()
            .all(|end| end.kind == Some(ConnectionKind::Miter))
    );
}

#[test]
fn rust_reports_curved_miters_and_copes_at_their_connection() {
    for (kind, diagnostic) in [
        (ConnectionKind::Miter, "straight-sided section"),
        (ConnectionKind::Cope, "coped ends"),
    ] {
        let mut f = fixture();
        let main = MemberRequest::new("Main", points(f.chs, [0., 0., 0.], [500., 0., 0.]));
        let branch = MemberRequest::new("Branch", points(f.chs, [500., 0., 0.], [500., 500., 0.]));
        let members = [branch.body, main.body];
        create(&mut f, &[main, branch]);
        let before = f.s.document().clone();
        let (connection, prepared) = model::prepare_connection(
            &f.s,
            f.s.revision(),
            actor(),
            "Joint",
            &ConnectionDefinition {
                kind,
                members,
                gap: None,
            },
        )
        .unwrap();
        let refusal = Compiler::default()
            .compile_with_backend(
                prepared.document(),
                f.s.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .err()
            .expect("unsupported Rust connection");
        assert_eq!(refusal.entity, Some(connection));
        assert!(refusal.message.contains(diagnostic), "{}", refusal.message);
        assert_eq!(f.s.document(), &before);
    }
}

#[test]
fn rust_bundled_workbench_members_preserve_geometry_and_schedules() {
    let project = project_file::read(
        &mut &include_bytes!("../../../examples/welded-workbench.tcd")[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let document = &project.document;
    let members: Vec<_> = model::members(document).map(|member| member.id).collect();
    let evaluate = |backend| {
        Compiler::default()
            .compile_sources_with_backend(document, 0, &members, backend, None)
            .unwrap_or_else(|err| panic!("{backend:?}: {err:?}"))
    };
    let rust = evaluate(GeometryBackend::Rust);
    let native = evaluate(GeometryBackend::OpenCascade);
    let rust_cuts = rust.member_cut_list(document).unwrap();
    let native_cuts = native.member_cut_list(document).unwrap();
    assert_eq!(rust_cuts.len(), 19);
    assert_eq!(rust_cuts.len(), native_cuts.len());
    for (a, b) in rust_cuts.iter().zip(&native_cuts) {
        assert_eq!((a.part, a.feature, &a.name), (b.part, b.feature, &b.name));
        assert_eq!(a.ends, b.ends, "{}", a.name);
        assert!(a.issues.is_empty(), "{}: {:?}", a.name, a.issues);
        near(a.length_mm, b.length_mm);
        rel(a.volume_mm3, b.volume_mm3);
        let a_bounds = bounds(&rust, a.part);
        let b_bounds = bounds(&native, b.part);
        for i in 0..3 {
            near(a_bounds.min[i], b_bounds.min[i]);
            near(a_bounds.max[i], b_bounds.max[i]);
        }
        let solid = rust.solid(rust.snapshot().bodies[&a.part]).unwrap();
        assert!(solid.uses_rust_geometry(), "{}", a.name);
        assert_eq!(solid.solid_count().unwrap(), 1, "{}", a.name);
        let receipt = solid
            .manufacturing_mesh_with_receipt(MeshOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", a.name));
        assert!(receipt.edge_pairs > 0 && receipt.vertex_links > 0);
    }
    let welds = rust.weld_schedule(document).unwrap();
    assert_eq!(welds.len(), 22);
    assert_eq!(welds, native.weld_schedule(document).unwrap());

    let rail = rust_cuts
        .iter()
        .find(|row| {
            row.ends
                .iter()
                .all(|end| end.kind == Some(ConnectionKind::Miter))
        })
        .unwrap();
    let rail = rust.solid(rust.snapshot().bodies[&rail.part]).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mitered-rail.step");
    rail.write_step(&path).unwrap();
    let policy = GeometryPolicy::default();
    let native_step = Solid::read_step(&path, policy).unwrap();
    let rust_step = Solid::import_rust_step(
        &std::fs::read_to_string(&path).unwrap(),
        EntityId::new(),
        policy,
    )
    .unwrap();
    let original = rail.bounds().unwrap();
    for imported in [native_step, rust_step] {
        assert_eq!(imported.solid_count().unwrap(), 1);
        rel(imported.volume_mm3().unwrap(), rail.volume_mm3().unwrap());
        let readback = imported.bounds().unwrap();
        for i in 0..3 {
            near(readback.min[i], original.min[i]);
            near(readback.max[i], original.max[i]);
        }
    }
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
fn rust_straight_copes_keep_clearance_hollow_material_and_reopen() {
    for gap in [0., 2.] {
        let mut f = fixture();
        let beam = MemberRequest::new("Beam", points(f.rhs, [0.; 3], [1000., 0., 0.]));
        let stud = MemberRequest::new("Stud", points(f.shs, [500., 0., 0.], [500., 500., 0.]));
        create(&mut f, &[beam.clone(), stud.clone()]);
        let before = f.s.document().clone();
        let connection = connect(
            &mut f,
            "Stud cope",
            ConnectionKind::Cope,
            [stud.body, beam.body],
            gap,
        );
        let evaluate = |document: &Document| {
            Compiler::default()
                .compile_with_backend(document, 0, GeometryBackend::Rust, None)
                .unwrap()
        };
        let model = evaluate(f.s.document());
        let list = model.member_cut_list(f.s.document()).unwrap();
        let stud_record = record(&list, "Stud");
        near(stud_record.span_mm[0], 30. + gap);
        rel(stud_record.volume_mm3, SHS_AREA * (470. - gap));
        assert_eq!(stud_record.ends[0].kind, Some(ConnectionKind::Cope));
        assert_eq!(stud_record.ends[0].connection, Some(connection));
        assert!(stud_record.issues.is_empty());
        let body = |id| model.solid(model.snapshot().bodies[&id]).unwrap();
        near(body(stud.body).bounds().unwrap().min[1], 30. + gap);
        near(
            body(beam.body).volume_mm3().unwrap(),
            (60. * 40. - 54. * 34.) * 1000.,
        );
        assert!(
            body(stud.body)
                .boolean(
                    body(beam.body),
                    multicad_kernel::BooleanOperation::Intersect,
                    GeometryPolicy::default()
                )
                .unwrap()
                .is_none()
        );
        body(stud.body)
            .manufacturing_mesh_with_receipt(MeshOptions::default())
            .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("stud.step");
        body(stud.body).write_step(&path).unwrap();
        let imported = Solid::import_rust_step(
            &std::fs::read_to_string(&path).unwrap(),
            EntityId::new(),
            GeometryPolicy::default(),
        )
        .unwrap();
        rel(imported.volume_mm3().unwrap(), SHS_AREA * (470. - gap));
        let project = ProjectFile::new(f.s.document().clone()).unwrap();
        let mut bytes = Vec::new();
        project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let reopened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert_eq!(
            evaluate(&reopened.document)
                .member_cut_list(&reopened.document)
                .unwrap(),
            list
        );
        let mut edited = model::read_connection(f.s.document(), connection).unwrap();
        edited.gap = Some(length(gap + 1.));
        let prepared =
            model::prepare_connection_edit(&f.s, f.s.revision(), actor(), connection, &edited)
                .unwrap();
        let candidate = evaluate(prepared.document());
        let edited_list = candidate.member_cut_list(prepared.document()).unwrap();
        near(record(&edited_list, "Stud").span_mm[0], 31. + gap);
        rel(
            record(&edited_list, "Stud").volume_mm3,
            SHS_AREA * (469. - gap),
        );
        rel(
            body(stud.body).volume_mm3().unwrap(),
            SHS_AREA * (470. - gap),
        );
        f.s.commit(prepared).unwrap();
        f.s.undo(f.s.revision(), actor()).unwrap();
        assert_eq!(
            evaluate(f.s.document())
                .member_cut_list(f.s.document())
                .unwrap(),
            list
        );
        f.s.undo(f.s.revision(), actor()).unwrap();
        assert_eq!(f.s.document(), &before);
        rel(
            record(
                &evaluate(f.s.document())
                    .member_cut_list(f.s.document())
                    .unwrap(),
                "Stud",
            )
            .volume_mm3,
            SHS_AREA * 500.,
        );
    }
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

// ---- Welds, sequence, cut groups and weld plans ----------------------------

fn weld(connection: EntityId, mark: &str, size: f64) -> WeldDefinition {
    WeldDefinition {
        connection,
        mark: mark.into(),
        symbol: WeldSymbol::Fillet,
        side: WeldSide::Arrow,
        size: Some(length(size)),
        length: None,
        all_around: true,
        field: false,
        process: "135 MAG".into(),
        notes: String::new(),
    }
}
fn add_weld(f: &mut Fixture, w: &WeldDefinition) -> EntityId {
    let (id, p) = model::prepare_weld(&f.s, f.s.revision(), actor(), w).unwrap();
    checked(&mut f.s, p);
    id
}
fn steps(welds: &[EntityId]) -> SequenceDefinition {
    SequenceDefinition {
        steps: welds
            .iter()
            .map(|w| SequenceStep {
                weld: *w,
                note: String::new(),
            })
            .collect(),
    }
}

#[test]
fn welds_follow_their_connections_in_authored_order_with_cut_groups_and_a_plan() {
    let mut f = fixture();
    let (sketch, pts, _) = path_sketch(
        &mut f.s,
        Datum::Xy,
        &[[0., 0.], [1000., 0.], [1000., 600.], [0., 600.]],
        true,
    );
    let rails =
        model::frame_member_requests(f.s.document(), sketch, "Rail", &template(f.shs)).unwrap();
    create(&mut f, &rails);
    let post = MemberRequest::new("Post", points(f.shs, [500., 0., 0.], [500., 0., 700.]));
    create(&mut f, std::slice::from_ref(&post));
    let corners: Vec<EntityId> = (0..4)
        .map(|i| {
            connect(
                &mut f,
                &format!("Corner {}", i + 1),
                ConnectionKind::Miter,
                [rails[i].body, rails[(i + 1) % 4].body],
                0.,
            )
        })
        .collect();
    let tee = connect(
        &mut f,
        "Post foot",
        ConnectionKind::Butt,
        [post.body, rails[0].body],
        0.,
    );

    // Cut groups: a mitered rectangle gives two pairs of identical rails.
    let m = compile(&f.s);
    let list = m.member_cut_list(f.s.document()).unwrap();
    near(record(&list, "Rail 1").length_mm, 1040.);
    near(record(&list, "Rail 2").length_mm, 640.);
    let groups = multicad_model::group_cuts(&list);
    let long = groups
        .iter()
        .find(|g| (g.length_mm - 1040.).abs() < 1e-6)
        .unwrap();
    assert_eq!(long.quantity, 2);
    assert_eq!(long.ends[0].0, "miter");
    near(long.ends[0].1, 45.);
    assert_eq!(long.ends[1].0, "miter");
    // A picture frame: both long points on the outside face.
    assert_eq!(long.long_points, Some("same face"));
    assert_eq!(long.long_faces[0], long.long_faces[1]);
    let schedule =
        multicad_model::schedules::member_cut_schedule(f.s.document(), f.s.revision(), &groups)
            .unwrap();
    let row = schedule.rows.iter().find(|r| r[3] == "1040").unwrap();
    let face = long.long_faces[0].unwrap();
    assert_eq!(row[4], format!("miter 45° long point {face}"));
    assert_eq!(row[6], "same face");
    let post_row = schedule.rows.iter().find(|r| r[7] == "Post").unwrap();
    assert_eq!(post_row[4..7], ["butt 0°", "square", ""]);
    assert_eq!(
        groups
            .iter()
            .find(|g| (g.length_mm - 640.).abs() < 1e-6)
            .unwrap()
            .quantity,
        2
    );
    let posts = groups.iter().find(|g| g.names == ["Post"]).unwrap();
    assert_eq!(posts.quantity, 1);
    near(posts.length_mm, 680.);

    // Two welds and a sequence that is then reordered.
    let mut w1 = weld(corners[0], "W1", 4.);
    w1.notes = "Outside corner first, flat position".into();
    let w1 = add_weld(&mut f, &w1);
    let w2 = add_weld(&mut f, &weld(tee, "W2", 3.));
    let (sequence, p) = model::prepare_sequence(
        &f.s,
        f.s.revision(),
        actor(),
        None,
        "Frame sequence",
        &steps(&[w2, w1]),
    )
    .unwrap();
    checked(&mut f.s, p);
    let schedule = compile(&f.s).weld_schedule(f.s.document()).unwrap();
    assert_eq!(
        schedule.iter().map(|r| r.mark.as_str()).collect::<Vec<_>>(),
        ["W2", "W1"]
    );
    let mut reordered = steps(&[w1, w2]);
    reordered.steps[1].note = "After the frame cools".into();
    let (same, p) = model::prepare_sequence(
        &f.s,
        f.s.revision(),
        actor(),
        Some(sequence),
        "",
        &reordered,
    )
    .unwrap();
    assert_eq!(same, sequence);
    checked(&mut f.s, p);
    let schedule = compile(&f.s).weld_schedule(f.s.document()).unwrap();
    let (r1, r2) = (&schedule[0], &schedule[1]);
    assert_eq!((r1.weld, r1.step, r1.connection), (w1, Some(1), corners[0]));
    assert_eq!((r2.weld, r2.step, r2.connection), (w2, Some(2), tee));
    assert_eq!(r2.step_note, "After the frame cools");
    assert_eq!(r1.kind, ConnectionKind::Miter);
    assert_eq!(r1.members, [rails[0].body, rails[1].body]);
    assert_eq!(r1.member_names, ["Rail 1".to_owned(), "Rail 2".to_owned()]);
    near(r1.size_mm.unwrap(), 4.);
    assert_eq!(r1.process, "135 MAG");
    // Positions are the joints; the joint perimeter is measured on the cut.
    assert_eq!(r1.position, [1000., 0., 0.]);
    near(r1.joint_perimeter_mm.unwrap(), 80. + 80. * 2_f64.sqrt());
    assert_eq!(r2.position, [500., 0., 0.]);
    near(r2.joint_perimeter_mm.unwrap(), 160.);

    // Moving the frame corner moves the weld position; references stay.
    move_points(
        &mut f.s,
        sketch,
        &[(pts[1], [1100., 0.]), (pts[2], [1100., 600.])],
    );
    let schedule = compile(&f.s).weld_schedule(f.s.document()).unwrap();
    assert_eq!(schedule[0].position, [1100., 0., 0.]);
    assert_eq!(schedule[0].connection, corners[0]);

    // Duplicate marks, welds without a connection and repeated steps are refused.
    let e = model::prepare_weld(&f.s, f.s.revision(), actor(), &weld(corners[1], "W1", 4.))
        .unwrap_err();
    assert!(e.message.contains("used twice"), "{}", e.message);
    let e = model::prepare_weld(&f.s, f.s.revision(), actor(), &weld(f.shs, "W9", 4.)).unwrap_err();
    assert!(e.message.contains("connect the members"), "{}", e.message);
    let e = model::prepare_sequence(
        &f.s,
        f.s.revision(),
        actor(),
        Some(sequence),
        "",
        &steps(&[w1, w1]),
    )
    .unwrap_err();
    assert!(e.message.contains("once"), "{}", e.message);

    // A weld plan on a top view: symbols point at the projected joints and
    // a sequence table lists the authored order.
    let view = DrawingView {
        id: EntityId::new(),
        name: "Top".into(),
        source: ViewSource::Model {
            entities: rails.iter().map(|r| r.body).chain([post.body]).collect(),
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [40., 150.],
        scale: Scale {
            numerator: 1,
            denominator: 10,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    };
    let m = compile(&f.s);
    let schedule = m.weld_schedule(f.s.document()).unwrap();
    let annotations = multicad_model::weld_plan_annotations(&schedule, &view, [20., 165.]);
    assert_eq!(annotations.len(), 3);
    let AnnotationKind::Weld {
        points_mm,
        tail,
        size,
        ..
    } = &annotations[0].kind
    else {
        panic!("weld symbol first")
    };
    let expected = view.paper_point(view_coordinates(&view, [1100., 0., 0.]));
    assert_eq!(points_mm[0], expected);
    assert_eq!(tail, "W1 135 MAG");
    assert_eq!(size, "4");
    let AnnotationKind::Table { rows, .. } = &annotations[2].kind else {
        panic!("table last")
    };
    assert_eq!(rows[0][0..3], ["1", "W1", "miter"]);
    assert!(rows[1][7].contains("After the frame cools"));
    let mut page = Page::new("1");
    page.views.push(view);
    page.annotations = annotations.clone();
    let (book, p) = prepare_create(&f.s, f.s.revision(), actor(), "Weld plan", Some(page)).unwrap();
    checked(&mut f.s, p);
    let e = compile(&f.s)
        .drawings(f.s.document(), f.s.revision(), &Default::default())
        .unwrap();
    // Sheet annotations are validated with the page and drawn by the SVG
    // page writer.
    let page_id = read(f.s.document(), book).unwrap().pages[0].id;
    let svg = svg::page_svg(f.s.document(), f.s.revision(), book, page_id, &e).unwrap();
    assert!(svg.contains("W1 135 MAG") && svg.contains("Weld sequence"));

    // Save and reopen keep welds and the sequence.
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
    assert!(opened.capabilities().contains(model::WELD_CAPABILITY));
    assert!(opened.capabilities().contains(model::SEQUENCE_CAPABILITY));
    assert_eq!(
        Compiler::default()
            .compile(&opened.document, 0)
            .unwrap()
            .weld_schedule(&opened.document)
            .unwrap(),
        schedule
    );
}
