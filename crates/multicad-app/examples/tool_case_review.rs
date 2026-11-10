//! The rolling tool case in one native editable project: a birch-ply carcass
//! on four purchased casters, one drawer on a pair of purchased ball-bearing
//! slides, a printed drawer and a printed bit-holder insert with editable
//! FDM aids. The slides' hole pattern, driven by `rail_pitch`,
//! `rail_holes` and `rail_offset`, drills both carcass sides and both drawer
//! walls through the associative transfer engine. The drawer is a Slider
//! joint with travel limits and named poses; its travel is sampled against
//! the carcass. Two export sets write the panel drawing (PDF and DXF) and
//! the print package (3MF with and without aids); the BOM and the panel cut
//! list are written as CSV. The app test `tests/tool_case.rs` builds the
//! same project and checks it.
use multicad_core::{
    assembly_inspection::{
        InspectionStamp, MotionRangeQuery, MotionRangeReport, OccurrencePair, PairQuery, PairReport,
    },
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    exchange::*,
    inspection::GeometryReference,
    materials::*,
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
};
use multicad_model::{
    Compiler, Model,
    assembly_inspection::{inspect_assembly, sample_assembly_range},
    export::Exporter,
    schedules::{self, BomOptions, Schedule},
};
use std::{io::Write, path::Path};

pub type Error = Box<dyn std::error::Error>;
pub const PRINTED: &str = "printed";
pub const PURCHASED: &str = "purchased";

// Fixed layout, millimeters. The carcass front-left-bottom corner sits on
// the casters at z = 60; its front faces −Y.
pub const CASTER_H: f64 = 60.;
pub const CASE: [f64; 3] = [300., 300., 200.]; // width, depth, height
pub const PLY: f64 = 18.;
pub const SLIDE: [f64; 3] = [6.5, 250., 35.]; // one member: thickness, length, height
pub const SLIDE_Z: f64 = 102.5;
pub const HOLE_Z: f64 = SLIDE_Z + SLIDE[2] / 2.;
pub const RAIL_HOLE_R: f64 = 2.25;
pub const PILOT_D: f64 = 3.;
pub const SIDE_DEPTH: f64 = 12.;
pub const WALL_DEPTH: f64 = 3.;
pub const DRAWER_X: f64 = PLY + 2. * SLIDE[0]; // 31
pub const DRAWER_W: f64 = CASE[0] - 2. * DRAWER_X; // 238
pub const DRAWER_D: f64 = 250.;
pub const DRAWER_Z: f64 = CASTER_H + PLY + 2.; // 80
pub const WALL: f64 = 4.;
pub const FLOOR: f64 = 3.;
pub const TRAVEL: f64 = 250.;

pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}

/// Native preflight, then the atomic commit.
pub fn checked(session: &mut Session, prepared: Prepared) -> Result<(), Error> {
    let model = Compiler::default().compile(prepared.document(), session.revision())?;
    if !model
        .snapshot()
        .references
        .for_model(prepared.document())?
        .all_resolved()
    {
        return Err("Native references are unresolved".into());
    }
    session.commit(prepared)?;
    Ok(())
}
fn parameter(name: &str, expression: &str, unit: &str, description: &str) -> Entity {
    let mut e = Entity::new(
        name,
        EntityData::Parameter {
            expression: expression.into(),
            unit: unit.into(),
            exposed: true,
            inputs: vec![],
        },
    );
    e.description = description.into();
    e
}
fn bound(session: &Session, text: &str, unit: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(session.document(), text, unit, None)?,
    })
}
pub fn xyz(v: [f64; 3]) -> [Dimension; 3] {
    v.map(length)
}
pub fn frame(origin: [f64; 3]) -> PrimitiveFrame {
    PrimitiveFrame {
        origin: xyz(origin),
        ..Default::default()
    }
}
pub fn primitive(
    session: &mut Session,
    name: &str,
    parameters: PrimitiveParameters,
) -> Result<PrimitiveRequest, Error> {
    let request = PrimitiveRequest::new(name, parameters);
    let prepared = prepare_primitive(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(request)
}
pub fn block(
    session: &mut Session,
    name: &str,
    size: [Dimension; 3],
    origin: [f64; 3],
) -> Result<PrimitiveRequest, Error> {
    primitive(
        session,
        name,
        PrimitiveParameters::Box {
            size,
            frame: frame(origin),
        },
    )
}
/// Cut `tool` from `target`; returns the cut feature, the new Part head.
pub fn cut(session: &mut Session, target: BodyInput, tool: BodyInput) -> Result<EntityId, Error> {
    let request = BooleanRequest::new(Operation::Cut, target, vec![tool]);
    let prepared = prepare_boolean(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(request.feature)
}
pub fn input(p: &PrimitiveRequest) -> BodyInput {
    BodyInput {
        body: p.body,
        producer: p.feature,
    }
}

/// One slide member: a 6.5 × 250 × 35 mm bar at `x` with the rail hole
/// pattern through it along X. Returns the Part and its head feature.
pub fn slide_member(
    session: &mut Session,
    name: &str,
    x: f64,
) -> Result<(EntityId, EntityId), Error> {
    let bar = block(session, name, xyz(SLIDE), [x, 0., SLIDE_Z])?;
    let seed = primitive(
        session,
        &format!("{name} hole"),
        PrimitiveParameters::Cylinder {
            radius: length(RAIL_HOLE_R),
            height: length(SLIDE[0] + 2.),
            frame: PrimitiveFrame {
                origin: [
                    length(x - 1.),
                    bound(session, "=$rail_offset", "mm")?,
                    length(HOLE_Z),
                ],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
        },
    )?;
    let pattern = AdvancedRequest::on_body(
        input(&seed),
        AdvancedParameters::LinearPattern(LinearPatternParameters {
            count: bound(session, "=$rail_holes", "count")?,
            step: [
                length(0.),
                bound(session, "=$rail_pitch", "mm")?,
                length(0.),
            ],
            merge: false,
        }),
        vec![],
    );
    let prepared = prepare_advanced(session, session.revision(), actor(), &pattern)?;
    checked(session, prepared.prepared)?;
    let head = cut(
        session,
        input(&bar),
        BodyInput {
            body: seed.body,
            producer: pattern.feature,
        },
    )?;
    Ok((bar.body, head))
}

/// A planar face of `producer`'s topology, captured for `owner`, whose
/// normal is `normal` and whose plane passes through `at` along it.
pub fn plane(
    model: &Model,
    producer: EntityId,
    owner: EntityId,
    normal: [f64; 3],
    at: f64,
) -> Result<AlignmentInput, Error> {
    let mut topology = model
        .snapshot()
        .shapes
        .get(&producer)
        .ok_or("Producer has no native shape")?
        .topology
        .clone();
    topology.owner = owner;
    let element = topology
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && matches!(e.geometry, Some(AnalyticGeometry::Plane { frame })
                    if frame.normal().iter().zip(normal).all(|(a, b)| (a - b).abs() < 1e-9)
                        && (frame.origin_mm.iter().zip(normal).map(|(o, n)| o * n).sum::<f64>() - at).abs() < 1e-6)
        })
        .ok_or_else(|| format!("Planar face {normal:?} at {at} is missing"))?;
    Ok(AlignmentInput {
        source: owner,
        intent: GeometryIntent::capture(&topology, element.key)?,
    })
}
/// A face of a world Part's current head, as a drawing reference.
pub fn part_face(
    model: &Model,
    part: EntityId,
    normal: [f64; 3],
    at: f64,
) -> Result<GeometryReference, Error> {
    let head = model.snapshot().bodies[&part];
    let face = plane(model, head, part, normal, at)?;
    Ok(GeometryReference {
        source: part,
        intent: face.intent,
    })
}
/// The full circular edge of a transferred pilot hole on `part`'s entry face
/// nearest `y`, as a drawing reference.
pub fn hole_edge(
    model: &Model,
    part: EntityId,
    entry_x: f64,
    y: f64,
) -> Result<GeometryReference, Error> {
    let head = model.snapshot().bodies[&part];
    let mut topology = model.snapshot().shapes[&head].topology.clone();
    topology.owner = part;
    let element = topology
        .elements
        .iter()
        .filter(|e| {
            e.key.kind == TopologyKind::Edge
                && matches!(e.geometry, Some(AnalyticGeometry::CircularArc { frame, radius_mm, .. })
                    if (radius_mm - PILOT_D / 2.).abs() < 1e-6 && (frame.origin_mm[0] - entry_x).abs() < 1e-6)
        })
        .min_by(|a, b| {
            let d = |e: &&multicad_core::references::TopologyElement| match e.geometry {
                Some(AnalyticGeometry::CircularArc { frame, .. }) => (frame.origin_mm[1] - y).abs(),
                _ => f64::MAX,
            };
            d(a).total_cmp(&d(b))
        })
        .ok_or("Transferred hole edge is missing")?;
    Ok(GeometryReference {
        source: part,
        intent: GeometryIntent::capture(&topology, element.key)?,
    })
}

/// Centers (y, z) of the full circular edges of radius `r` lying in the
/// plane x = `x` of a solid's topology, sorted by y.
pub fn hole_centers(model: &Model, producer: EntityId, x: f64, r: f64) -> Vec<[f64; 2]> {
    let mut centers: Vec<[f64; 2]> = model.snapshot().shapes[&producer]
        .topology
        .elements
        .iter()
        .filter_map(|e| match e.geometry {
            Some(AnalyticGeometry::CircularArc {
                frame,
                radius_mm,
                sweep_rad,
                ..
            }) if e.key.kind == TopologyKind::Edge
                && (radius_mm - r).abs() < 1e-6
                && (sweep_rad.abs() - 2. * std::f64::consts::PI).abs() < 1e-6
                && (frame.origin_mm[0] - x).abs() < 1e-6 =>
            {
                Some([frame.origin_mm[1], frame.origin_mm[2]])
            }
            _ => None,
        })
        .collect();
    centers.sort_by(|a, b| a[0].total_cmp(&b[0]));
    centers.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6);
    centers
}

/// A placed assembly member.
#[derive(Clone, Copy, Debug)]
pub struct Member {
    pub part: EntityId,
    pub head: EntityId,
    pub instance: EntityId,
}
pub struct Scenario {
    pub session: Session,
    pub rail_pitch: EntityId,
    pub rail_holes: EntityId,
    pub rail_offset: EntityId,
    pub drawer_height: EntityId,
    pub birch: EntityId,
    pub petg: EntityId,
    pub ply: EntityId,
    pub carcass: EntityId,
    /// Carcass panel Parts and features by role.
    pub left_side: CreatedBody,
    pub right_side: CreatedBody,
    pub top: CreatedBody,
    pub bottom: CreatedBody,
    pub cabinet_member: EntityId,
    pub drawer_member: EntityId,
    pub caster: EntityId,
    pub slide_left: Member,
    pub slide_right: Member,
    pub runner_left: Member,
    pub runner_right: Member,
    pub drawer: Member,
    pub insert: Member,
    pub casters: Vec<EntityId>,
    pub assembly: EntityId,
    pub root: EntityId,
    pub slider: JointRequest,
    pub closed: EntityId,
    pub open: EntityId,
    /// Transfer groups: left side, right side, drawer left, drawer right.
    pub transfers: [TransferRequest; 4],
    pub process: EntityId,
    pub tabs: Vec<EntityId>,
    pub support: EntityId,
    pub book: EntityId,
    pub panel_page: EntityId,
    pub length_dimension: EntityId,
    pub height_dimension: EntityId,
    pub hole_dimension: EntityId,
    pub drawing_set: EntityId,
    pub print_set: EntityId,
    pub with_aids: EntityId,
    pub without_aids: EntityId,
}
impl Scenario {
    pub fn path(&self, member: &Member) -> OccurrencePath {
        OccurrencePath {
            root: self.root,
            members: vec![member.instance],
        }
    }
    pub fn panel(&self, part: EntityId) -> OccurrencePath {
        OccurrencePath {
            root: part,
            members: vec![],
        }
    }
    /// The drawer against the carcass panels, its runner against the left
    /// slide and the insert against the drawer.
    pub fn travel_pairs(&self) -> Vec<OccurrencePair> {
        let drawer = self.path(&self.drawer);
        let mut pairs: Vec<_> = [&self.left_side, &self.right_side, &self.bottom, &self.top]
            .into_iter()
            .map(|p| OccurrencePair {
                a: drawer.clone(),
                b: self.panel(p.body),
            })
            .collect();
        pairs.push(OccurrencePair {
            a: self.path(&self.runner_left),
            b: self.path(&self.slide_left),
        });
        pairs.push(OccurrencePair {
            a: self.path(&self.insert),
            b: drawer,
        });
        pairs
    }
    pub fn pair_query(&self) -> Result<PairQuery, Error> {
        Ok(PairQuery {
            expected: InspectionStamp::capture(self.session.document(), self.session.revision())?,
            pairs: self.travel_pairs(),
            contact_tolerance_mm: 1e-6,
        })
    }
    /// Sample the drawer travel from closed to 25 mm past its limit.
    pub fn travel_query(&self, step: f64) -> Result<MotionRangeQuery, Error> {
        let pair = self.pair_query()?;
        Ok(MotionRangeQuery {
            expected: pair.expected,
            pairs: pair.pairs,
            contact_tolerance_mm: pair.contact_tolerance_mm,
            joint: self.slider.driver,
            coordinate: JointCoordinate::Z,
            start: 0.,
            end: TRAVEL + 25.,
            step,
        })
    }
}

fn place(
    session: &mut Session,
    part: EntityId,
    head: EntityId,
    name: &str,
    translation: [f64; 3],
) -> Result<Member, Error> {
    let mut request = ComponentPlacementRequest::new(
        part,
        NumericTransform {
            translation: xyz(translation),
            ..Default::default()
        },
    );
    request.name = name.into();
    let prepared = prepare_place_component(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(Member {
        part,
        head,
        instance: request.instance,
    })
}
fn joint(session: &mut Session, request: &JointRequest) -> Result<(), Error> {
    let prepared = prepare_joint(session, session.revision(), actor(), request)?;
    checked(session, prepared.prepared)
}
/// A joint frame whose normal (the slider axis) points out of the front, −Y.
fn pull_frame() -> PrimitiveFrame {
    PrimitiveFrame {
        u: [1., 0., 0.],
        v: [0., 0., 1.],
        ..Default::default()
    }
}
/// Rigidly attach `owner` to `parent`, offset by `offset`.
fn fixed(owner: EntityId, parent: EntityId, offset: [f64; 3]) -> JointRequest {
    JointRequest::new(
        owner,
        JointParameters {
            kind: JointKind::Fixed,
            parent: Some(parent),
            target: JointAnchor::Frame {
                frame: Box::new(frame(offset)),
            },
            ..Default::default()
        },
        vec![],
    )
}
fn pose(
    session: &mut Session,
    name: &str,
    joint: EntityId,
    travel: f64,
) -> Result<EntityId, Error> {
    let request = NamedPoseRequest::new(
        name,
        vec![JointPoseValue {
            joint,
            values: JointValues {
                translation: [length(0.), length(0.), length(travel)],
                ..Default::default()
            },
        }],
    );
    let prepared = prepare_named_pose(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(request.pose)
}
pub fn apply_pose(session: &mut Session, pose: EntityId) -> Result<(), Error> {
    let prepared = prepare_apply_pose(session, session.revision(), actor(), pose)?;
    checked(session, prepared.prepared)
}
fn tag(session: &mut Session, parts: &[EntityId], tag: &str) -> Result<(), Error> {
    let prepared = prepare_tags(
        session,
        session.revision(),
        actor(),
        parts,
        &TagEdit::Add([tag.to_string()].into()),
    )?;
    session.commit(prepared)?;
    Ok(())
}
fn material(name: &str, density: f64, color: [u8; 4]) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: Some(density),
            color_rgba: color,
            section_hatch: None,
        },
    )
}
fn assign(session: &mut Session, parts: &[EntityId], material: EntityId) -> Result<(), Error> {
    let prepared = prepare_assignment(
        session,
        session.revision(),
        actor(),
        parts,
        &Assignment {
            material: Some(material),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )?;
    session.commit(prepared)?;
    Ok(())
}

/// One transfer group: holes of `source` (its face at local `face_x` with
/// `face_normal`) drill `target` from its entry face at world `entry_x`.
#[allow(clippy::too_many_arguments)]
fn transfer(
    session: &mut Session,
    name: &str,
    source: &Member,
    source_path: OccurrencePath,
    face_normal: [f64; 3],
    face_x: f64,
    target: BodyInput,
    target_path: OccurrencePath,
    entry_normal: [f64; 3],
    entry_x: f64,
    depth: f64,
) -> Result<TransferRequest, Error> {
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let face = plane(
        &model,
        source.head,
        source.part,
        face_normal,
        face_normal[0] * face_x,
    )?;
    let mut request = TransferRequest::new(
        name,
        TransferSource {
            component: source.part,
            path: source_path,
            selection: TransferSelection::FaceHoles {
                face: face.intent.id,
            },
        },
    );
    request.references.push(face);
    let entry = plane(
        &model,
        target.producer,
        target.producer,
        entry_normal,
        entry_normal[0] * entry_x,
    )?;
    request.targets.push(TransferTarget::new(
        target,
        target_path,
        entry.intent.id,
        TransferTargetParameters {
            diameter: Some(length(PILOT_D)),
            depth: TransferDepth::Blind {
                depth: length(depth),
            },
            ..Default::default()
        },
    ));
    request.references.push(entry);
    let prepared = prepare_transfer(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(request)
}

pub fn rail_parameters() -> [Entity; 3] {
    [
        parameter("rail_pitch", "96", "mm", "Slide screw hole pitch"),
        parameter("rail_holes", "3", "count", "Slide screw holes per member"),
        parameter(
            "rail_offset",
            "29",
            "mm",
            "First slide screw hole behind the front edge",
        ),
    ]
}

pub fn build() -> Result<Scenario, Error> {
    let [rail_pitch, rail_holes, rail_offset] = rail_parameters();
    let drawer_height = parameter("drawer_height", "80", "mm", "Printed drawer height");
    let ids = (
        rail_pitch.id,
        rail_holes.id,
        rail_offset.id,
        drawer_height.id,
    );
    let mut session = Session::new(Document::new("Rolling tool case")?.edited(&[
        Mutation::Insert(rail_pitch),
        Mutation::Insert(rail_holes),
        Mutation::Insert(rail_offset),
        Mutation::Insert(drawer_height),
    ])?)?;

    // Materials and panel stock.
    let birch = material("Birch plywood", 680., [214, 186, 140, 255]);
    let petg = material("PETG", 1270., [40, 110, 170, 255]);
    let steel = material("Zinc-plated steel", 7850., [175, 178, 182, 255]);
    let rubber = material("Caster (rubber/steel)", 1500., [40, 40, 40, 255]);
    let ply = LibraryDefinition::Stock(Stock {
        material: birch.id,
        specification: StockKind::Panel {
            thickness_mm: PLY,
            length_mm: 2500.,
            width_mm: 1250.,
            grain: Grain::AlongLength,
        },
    })
    .entity(EntityId::new(), "Birch ply 18")?;
    let mids = (birch.id, petg.id, steel.id, rubber.id, ply.id);
    let prepared = prepare_library(
        &session,
        session.revision(),
        actor(),
        &[birch, petg, steel, rubber],
    )?;
    session.commit(prepared)?;
    let prepared = prepare_library(&session, session.revision(), actor(), &[ply])?;
    session.commit(prepared)?;

    // The carcass: full-height sides, no back, no shelves.
    let created = prepare_carcass(
        &session,
        session.revision(),
        actor(),
        &CarcassRequest::new(
            "Tool case",
            CarcassDefinition {
                cabinet: None,
                origin: xyz([0., 0., CASTER_H]),
                rotation: angle(0.),
                width: length(CASE[0]),
                height: length(CASE[2]),
                depth: length(CASE[1]),
                stock: mids.4,
                construction: Construction::SidesFullHeight,
                back: Back::None,
                shelves: Shelves {
                    layout: ShelfLayout::Even { count: 0 },
                    setback: length(0.),
                    clearance: length(0.),
                },
                front_band: None,
                group: None,
            },
        ),
    )?;
    let carcass = created.carcass;
    let role = |r: PanelRole| {
        created
            .panels
            .iter()
            .find(|(x, _)| *x == r)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| format!("Carcass has no {r:?}"))
    };
    let (left_side, right_side, top, bottom) = (
        role(PanelRole::LeftSide)?,
        role(PanelRole::RightSide)?,
        role(PanelRole::Top)?,
        role(PanelRole::Bottom)?,
    );
    checked(&mut session, created.model.prepared)?;

    // Purchased hardware: the two slide members carry the parametric hole
    // pattern; the caster is an envelope.
    let (cabinet_member, cabinet_head) =
        slide_member(&mut session, "Slide 250 cabinet member", PLY)?;
    let (drawer_member, drawer_head) =
        slide_member(&mut session, "Slide 250 drawer member", PLY + SLIDE[0])?;
    let caster = block(
        &mut session,
        "Caster 60",
        xyz([50., 50., CASTER_H]),
        [0.; 3],
    )?;

    // Printed drawer: an open box with 4 mm walls and a 3 mm floor.
    let height = bound(&session, "=$drawer_height", "mm")?;
    let shell = block(
        &mut session,
        "Printed drawer",
        [length(DRAWER_W), length(DRAWER_D), height.clone()],
        [DRAWER_X, 0., DRAWER_Z],
    )?;
    let cavity = block(
        &mut session,
        "Drawer cavity",
        [
            length(DRAWER_W - 2. * WALL),
            length(DRAWER_D - 2. * WALL),
            height,
        ],
        [DRAWER_X + WALL, WALL, DRAWER_Z + FLOOR],
    )?;
    let drawer_head_feature = cut(&mut session, input(&shell), input(&cavity))?;
    // Printed bit holder: a bridge whose tunnel needs a removable support.
    let holder = block(
        &mut session,
        "Printed bit holder",
        xyz([120., 60., 40.]),
        [90., 20., DRAWER_Z + FLOOR],
    )?;
    let tunnel = block(
        &mut session,
        "Bit holder tunnel",
        xyz([80., 62., 21.]),
        [110., 19., DRAWER_Z + FLOOR - 1.],
    )?;
    let insert_head = cut(&mut session, input(&holder), input(&tunnel))?;
    assign(&mut session, &[shell.body, holder.body], mids.1)?;
    assign(&mut session, &[cabinet_member, drawer_member], mids.2)?;
    assign(&mut session, &[caster.body], mids.3)?;
    tag(
        &mut session,
        &[cabinet_member, drawer_member, caster.body],
        PURCHASED,
    )?;
    tag(&mut session, &[shell.body, holder.body], PRINTED)?;

    // The running gear: slides, drawer, insert and casters as one assembly.
    let right = CASE[0] - PLY - SLIDE[0] - PLY; // 257
    let slide_left = place(
        &mut session,
        cabinet_member,
        cabinet_head,
        "Left slide",
        [0.; 3],
    )?;
    let slide_right = place(
        &mut session,
        cabinet_member,
        cabinet_head,
        "Right slide",
        [right, 0., 0.],
    )?;
    let runner_left = place(
        &mut session,
        drawer_member,
        drawer_head,
        "Left runner",
        [0.; 3],
    )?;
    let runner_right = place(
        &mut session,
        drawer_member,
        drawer_head,
        "Right runner",
        [0.; 3],
    )?;
    let drawer = place(
        &mut session,
        shell.body,
        drawer_head_feature,
        "Drawer",
        [0.; 3],
    )?;
    let insert = place(
        &mut session,
        holder.body,
        insert_head,
        "Bit holder",
        [0.; 3],
    )?;
    let mut casters = vec![];
    for (i, (x, y)) in [(10., 10.), (240., 10.), (10., 240.), (240., 240.)]
        .into_iter()
        .enumerate()
    {
        casters.push(
            place(
                &mut session,
                caster.body,
                caster.feature,
                &format!("Caster {}", i + 1),
                [x, y, 0.],
            )?
            .instance,
        );
    }
    // The drawer rides on the left slide along −Y within its travel; the
    // runners and the insert move with it.
    let slider = JointRequest::new(
        drawer.instance,
        JointParameters {
            kind: JointKind::Slider,
            parent: Some(slide_left.instance),
            moving: JointAnchor::Frame {
                frame: Box::new(pull_frame()),
            },
            target: JointAnchor::Frame {
                frame: Box::new(pull_frame()),
            },
            limits: JointLimits {
                translation: [
                    None,
                    None,
                    Some(JointLimit {
                        min: length(0.),
                        max: length(TRAVEL),
                    }),
                ],
                rotation: None,
            },
            ..Default::default()
        },
        vec![],
    );
    joint(&mut session, &slider)?;
    let runner_offset = DRAWER_W + SLIDE[0]; // 244.5
    joint(
        &mut session,
        &fixed(runner_left.instance, drawer.instance, [0.; 3]),
    )?;
    joint(
        &mut session,
        &fixed(
            runner_right.instance,
            drawer.instance,
            [runner_offset, 0., 0.],
        ),
    )?;
    joint(
        &mut session,
        &fixed(insert.instance, drawer.instance, [0.; 3]),
    )?;
    let mut members = vec![
        slide_left.instance,
        slide_right.instance,
        runner_left.instance,
        runner_right.instance,
        drawer.instance,
        insert.instance,
    ];
    members.extend(&casters);
    let assembly = AssemblyRequest::new(
        "Running gear",
        members,
        vec![
            cabinet_member,
            drawer_member,
            caster.body,
            shell.body,
            holder.body,
        ],
    );
    let prepared = prepare_assembly(&session, session.revision(), actor(), &assembly)?;
    checked(&mut session, prepared.prepared)?;
    let mut root = ComponentPlacementRequest::new(assembly.assembly, NumericTransform::default());
    root.name = "Running gear".into();
    let prepared = prepare_place_component(&session, session.revision(), actor(), &root)?;
    checked(&mut session, prepared.prepared)?;
    let closed = pose(&mut session, "Drawer closed", slider.driver, 0.)?;
    let open = pose(&mut session, "Drawer open", slider.driver, TRAVEL)?;

    // The slides' hole pattern drills both carcass sides and both drawer
    // walls through the transfer engine.
    let in_gear = |m: &Member| OccurrencePath {
        root: root.instance,
        members: vec![m.instance],
    };
    let world = |p: &CreatedBody| OccurrencePath {
        root: p.body,
        members: vec![],
    };
    let body = |p: &CreatedBody| BodyInput {
        body: p.body,
        producer: p.feature,
    };
    let left_transfer = transfer(
        &mut session,
        "Left slide drilling",
        &slide_left,
        in_gear(&slide_left),
        [-1., 0., 0.],
        PLY,
        body(&left_side),
        world(&left_side),
        [1., 0., 0.],
        PLY,
        SIDE_DEPTH,
    )?;
    let right_transfer = transfer(
        &mut session,
        "Right slide drilling",
        &slide_right,
        in_gear(&slide_right),
        [1., 0., 0.],
        PLY + SLIDE[0],
        body(&right_side),
        world(&right_side),
        [-1., 0., 0.],
        CASE[0] - PLY,
        SIDE_DEPTH,
    )?;
    let drawer_left = transfer(
        &mut session,
        "Left runner drilling",
        &runner_left,
        in_gear(&runner_left),
        [1., 0., 0.],
        PLY + 2. * SLIDE[0],
        BodyInput {
            body: shell.body,
            producer: drawer_head_feature,
        },
        in_gear(&drawer),
        [-1., 0., 0.],
        DRAWER_X,
        WALL_DEPTH,
    )?;
    let drawer_right = transfer(
        &mut session,
        "Right runner drilling",
        &runner_right,
        in_gear(&runner_right),
        [-1., 0., 0.],
        PLY + SLIDE[0],
        BodyInput {
            body: shell.body,
            producer: drawer_left.targets[0].feature,
        },
        in_gear(&drawer),
        [1., 0., 0.],
        DRAWER_X + DRAWER_W,
        WALL_DEPTH,
    )?;
    let drawer = Member {
        head: drawer_right.targets[0].feature,
        ..drawer
    };
    let left_side_head = CreatedBody {
        feature: left_transfer.targets[0].feature,
        body: left_side.body,
    };
    let right_side_head = CreatedBody {
        feature: right_transfer.targets[0].feature,
        body: right_side.body,
    };

    // FDM aids: mouse-ear tabs at the drawer's four bottom corners and a
    // removable support in the bit holder's tunnel.
    let (process, prepared) = prepare_process(
        &session,
        session.revision(),
        actor(),
        None,
        "PETG 0.2 / 0.45",
        &ProcessDefinition {
            layer_height: length(0.2),
            line_width: length(0.45),
            min_layers: 2,
            min_lines: 2,
            support_gap: length(0.2),
            fit_clearance: length(0.3),
        },
    )?;
    session.commit(prepared)?;
    let aid = |session: &mut Session, name: &str, target: EntityId, shape: AidShape| {
        let request = AidRequest::new(
            name,
            AidDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                target,
                shape,
                process: Some(process),
            },
        );
        let prepared = prepare_aid(session, session.revision(), actor(), &request)?;
        checked(session, prepared.prepared)?;
        Ok::<_, Error>(request.body)
    };
    let mut tabs = vec![];
    for (i, (x, y, deg)) in [
        (DRAWER_X + 15., 0., 270.),
        (DRAWER_X + DRAWER_W - 15., 0., 270.),
        (DRAWER_X + 15., DRAWER_D, 90.),
        (DRAWER_X + DRAWER_W - 15., DRAWER_D, 90.),
    ]
    .into_iter()
    .enumerate()
    {
        tabs.push(aid(
            &mut session,
            &format!("Drawer tab {}", i + 1),
            shell.body,
            AidShape::Tab {
                origin: xyz([x, y, DRAWER_Z]),
                angle: angle(deg),
                length: length(8.),
                width: length(10.),
                thickness: length(0.6),
                overlap: length(2.),
            },
        )?);
    }
    let support = aid(
        &mut session,
        "Bit holder tunnel support",
        holder.body,
        AidShape::Support {
            origin: [length(112.), length(22.)],
            angle: angle(0.),
            length: length(76.),
            width: length(56.),
            base: length(DRAWER_Z + FLOOR),
            gap: length(0.2),
        },
    )?;
    let mut aids = tabs.clone();
    aids.push(support);
    // Aid Parts join the printed scope; their `fdm.aid` feature, not the
    // tag, makes them removable.
    tag(&mut session, &aids, PRINTED)?;

    // Panel drawing: both sides seen from the inside at 1:2, overall and
    // hole dimensions, a note and the cut list table.
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let view = |name: &str, part: EntityId, u: [f64; 3], at: [f64; 2]| DrawingView {
        id: EntityId::new(),
        name: name.into(),
        source: ViewSource::Model {
            entities: vec![part],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u,
            v: [0., 0., 1.],
        },
        kind: ViewKind::Projection,
        position_mm: at,
        scale: Scale {
            numerator: 1,
            denominator: 2,
        },
        hidden_lines: true,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    };
    let left_view = view(
        "Left side, inside",
        left_side.body,
        [0., 1., 0.],
        [40., 260.],
    );
    let right_view = view(
        "Right side, inside",
        right_side.body,
        [0., -1., 0.],
        [380., 260.],
    );
    let dimension = |view: EntityId, measure: DimensionMeasure, offset: f64| Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Dimension {
            view,
            measure: Box::new(measure),
            offset_mm: offset,
            precision: 0,
            reference: false,
        },
    };
    let length_dimension = dimension(
        left_view.id,
        DimensionMeasure::Linear {
            a: part_face(&model, left_side.body, [0., -1., 0.], 0.)?,
            b: part_face(&model, left_side.body, [0., 1., 0.], CASE[1])?,
            direction: LinearDirection::Horizontal,
        },
        12.,
    );
    let height_dimension = dimension(
        left_view.id,
        DimensionMeasure::Linear {
            a: part_face(&model, left_side.body, [0., 0., -1.], -CASTER_H)?,
            b: part_face(&model, left_side.body, [0., 0., 1.], CASTER_H + CASE[2])?,
            direction: LinearDirection::Vertical,
        },
        12.,
    );
    let hole_dimension = dimension(
        left_view.id,
        DimensionMeasure::Diameter {
            edge: hole_edge(&model, left_side.body, PLY, 125.)?,
        },
        8.,
    );
    let dimension_ids = (length_dimension.id, height_dimension.id, hole_dimension.id);
    let cut_list = schedules::cut_list_schedule(
        session.document(),
        session.revision(),
        &model.cut_list(session.document())?,
    )?;
    let mut page = Page::new("TC-101");
    page.settings.paper = PaperSize::A3;
    page.info = DrawingInfo {
        title: "Tool case side panels".into(),
        author: "MultiCAD".into(),
        project: "Rolling tool case".into(),
        revision: "A".into(),
        checker: "Pending human review".into(),
    };
    page.annotations
        .extend([length_dimension, height_dimension, hole_dimension]);
    page.annotations.push(Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm: [40., 246.],
            text: "Slide pilot holes Ø3 x 12 follow the slide hole pattern (rail_pitch)".into(),
        },
    });
    page.annotations.push(Annotation {
        id: EntityId::new(),
        kind: cut_list.table([20., 40.]),
    });
    page.views.push(left_view);
    page.views.push(right_view);
    let panel_page = page.id;
    let (book, prepared) = prepare_create(
        &session,
        session.revision(),
        actor(),
        "Tool case drawings",
        Some(page),
    )?;
    checked(&mut session, prepared)?;

    // Export sets: the panel drawing, and the print package with and
    // without its removable aids.
    let drawing_set = EntityId::new();
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor(),
        drawing_set,
        "Panel drawing",
        &SetDefinition {
            scope: Scope {
                root: ScopeRoot::Explicit {
                    entities: vec![left_side.body, right_side.body],
                },
                tags: None,
            },
            pages: Pages::Project,
            recipes: vec![
                Recipe {
                    id: EntityId::new(),
                    name: "PDF".into(),
                    output: RecipeOutput::Pdf,
                },
                Recipe {
                    id: EntityId::new(),
                    name: "DXF".into(),
                    output: RecipeOutput::DrawingDxf {
                        units: LengthUnit::Millimeter,
                        layers: Layers::default(),
                    },
                },
            ],
            ..Default::default()
        },
    )?;
    session.commit(prepared)?;
    let mesh = |name: &str, aids: AidPolicy| Recipe {
        id: EntityId::new(),
        name: name.into(),
        output: RecipeOutput::Mesh {
            format: MeshFormat::ThreeMf,
            units: LengthUnit::Millimeter,
            hierarchy: HierarchyMode::PreserveAssembly,
            names: NameHandling::ProductNames,
            aids,
            quality: MeshQuality::default(),
        },
    };
    let with_aids = mesh("3MF with aids", AidPolicy::IncludeAll);
    let without_aids = mesh("3MF without aids", AidPolicy::ExcludeRemovable);
    let recipe_ids = (with_aids.id, without_aids.id);
    let print_set = EntityId::new();
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor(),
        print_set,
        "Print package",
        &SetDefinition {
            scope: Scope {
                root: ScopeRoot::Project,
                tags: Some(TagQuery {
                    all: [PRINTED.into()].into(),
                    ..Default::default()
                }),
            },
            pages: Pages::None,
            recipes: vec![with_aids, without_aids],
            ..Default::default()
        },
    )?;
    session.commit(prepared)?;
    apply_pose(&mut session, closed)?;

    Ok(Scenario {
        session,
        rail_pitch: ids.0,
        rail_holes: ids.1,
        rail_offset: ids.2,
        drawer_height: ids.3,
        birch: mids.0,
        petg: mids.1,
        ply: mids.4,
        carcass,
        left_side: left_side_head,
        right_side: right_side_head,
        top,
        bottom,
        cabinet_member,
        drawer_member,
        caster: caster.body,
        slide_left,
        slide_right,
        runner_left,
        runner_right,
        drawer,
        insert,
        casters,
        assembly: assembly.assembly,
        root: root.instance,
        slider,
        closed,
        open,
        transfers: [left_transfer, right_transfer, drawer_left, drawer_right],
        process,
        tabs,
        support,
        book,
        panel_page,
        length_dimension: dimension_ids.0,
        height_dimension: dimension_ids.1,
        hole_dimension: dimension_ids.2,
        drawing_set,
        print_set,
        with_aids: recipe_ids.0,
        without_aids: recipe_ids.1,
    })
}

/// Preview and run an export set into `root`; returns the receipt.
pub fn export(session: &Session, set: EntityId, root: &Path) -> Result<RunReceipt, Error> {
    let exporter = Exporter::default();
    let preview = exporter.preview(session.document(), session.revision(), set, &actor(), None)?;
    if !preview.can_run() {
        return Err(format!("Export set cannot run: {:?}", preview.recipes).into());
    }
    Ok(exporter.run(
        session.document(),
        session.revision(),
        &preview.key,
        root,
        &actor(),
        None,
    )?)
}

pub fn archive(session: &Session) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    Ok(bytes)
}

/// Set a project parameter's expression as one checked transaction.
pub fn set_parameter(session: &mut Session, id: EntityId, expression: &str) -> Result<(), Error> {
    let mut entity = session
        .document()
        .get(id)
        .ok_or("Missing parameter")?
        .clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        return Err("Not a parameter".into());
    };
    *e = expression.into();
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Edit parameter".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    checked(session, prepared)
}

/// The BOM of purchased and manufactured Parts, without aids.
pub fn bom(session: &Session, model: &Model) -> Result<Schedule, Error> {
    let bom = model.bom(
        session.document(),
        &BomOptions {
            include_aids: false,
            include_purchased: true,
            roots: None,
        },
    )?;
    Ok(schedules::bom_schedule(&bom))
}
pub fn cut_list(session: &Session, model: &Model) -> Result<Schedule, Error> {
    Ok(schedules::cut_list_schedule(
        session.document(),
        session.revision(),
        &model.cut_list(session.document())?,
    )?)
}
pub fn interference(s: &Scenario, model: &Model) -> Result<PairReport, Error> {
    Ok(inspect_assembly(model, &s.pair_query()?)?)
}
pub fn travel(s: &Scenario, step: f64) -> Result<MotionRangeReport, Error> {
    Ok(sample_assembly_range(
        &Compiler::default(),
        s.session.document(),
        &s.travel_query(step)?,
        None,
    )?)
}
/// A JSON summary of a sampled travel report, stating its limits.
pub fn travel_summary(s: &Scenario, report: &MotionRangeReport) -> serde_json::Value {
    let name = |path: &OccurrencePath| {
        let id = path.members.last().copied().unwrap_or(path.root);
        s.session
            .document()
            .get(id)
            .map_or_else(|| id.to_string(), |e| e.name.clone())
    };
    serde_json::json!({
        "joint": "Drawer slider",
        "unit": report.unit,
        "coverage": report.coverage,
        "limit_mm": [0., TRAVEL],
        "sample_values": report.sample_values,
        "intervals": report.actual_intervals,
        "statement": "Exact native checks at the listed samples only; motion between samples is not checked, and samples outside the joint limit are refused, not evaluated.",
        "samples": report.samples.iter().map(|sample| match &sample.result {
            Ok(r) => serde_json::json!({
                "value": sample.value,
                "pairs": r.pairs.iter().map(|p| match &p.result {
                    Ok(v) => serde_json::json!({
                        "a": name(&p.pair.a), "b": name(&p.pair.b),
                        "relation": v.relation,
                        "overlap_mm3": v.overlap_volume_mm3,
                        "clearance_mm": v.minimum_clearance_mm,
                    }),
                    Err(e) => serde_json::json!({ "a": name(&p.pair.a), "b": name(&p.pair.b), "error": e.message }),
                }).collect::<Vec<_>>(),
            }),
            Err(e) => serde_json::json!({ "value": sample.value, "refused": format!("{:?}: {}", e.kind, e.message) }),
        }).collect::<Vec<_>>(),
    })
}

pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let scenario = build()?;
    let session = &scenario.session;
    let project = directory.join("tool-case.multicad");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&project)?;
    file.write_all(&archive(session)?)?;
    file.sync_all()?;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let exports = directory.join("exports");
    std::fs::create_dir(&exports)?;
    let exports = exports.canonicalize()?;
    let drawing = export(session, scenario.drawing_set, &exports)?;
    let print = export(session, scenario.print_set, &exports)?;
    let schedules = directory.join("schedules");
    std::fs::create_dir(&schedules)?;
    let bom_csv = schedules.join("bom.csv");
    std::fs::write(&bom_csv, bom(session, &model)?.csv())?;
    let cut_csv = schedules.join("panel-cut-list.csv");
    std::fs::write(&cut_csv, cut_list(session, &model)?.csv())?;
    let interference = interference(&scenario, &model)?;
    let travel = travel(&scenario, 50.)?;
    let files = |receipt: &RunReceipt| {
        receipt
            .files
            .iter()
            .map(|f| Path::new(&receipt.directory).join(&f.artifact.output.filename))
            .collect::<Vec<_>>()
    };
    let manifest = serde_json::json!({
        "generator": "tool_case_review",
        "project": project,
        "revision": session.revision(),
        "panel_drawing": files(&drawing),
        "print_package": files(&print),
        "schedules": [bom_csv, cut_csv],
        "transfer_counts": scenario.transfers.iter().map(|t| {
            serde_json::json!({ "group": t.name, "holes": model.snapshot().transfer_counts.get(&t.targets[0].feature) })
        }).collect::<Vec<_>>(),
        "interference_closed": interference.pairs.iter().map(|p| match &p.result {
            Ok(v) => serde_json::json!({ "relation": v.relation, "overlap_mm3": v.overlap_volume_mm3, "clearance_mm": v.minimum_clearance_mm }),
            Err(e) => serde_json::json!({ "error": e.message }),
        }).collect::<Vec<_>>(),
        "travel": travel_summary(&scenario, &travel),
        "verification": {
            "native_geometry": true,
            "interactive_macos": false,
            "windows_linux_runtime": false,
            "human_acceptance": false,
            "manufacturing_or_load_rating": false,
            "slicer_or_printer": false,
        },
    });
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Usage: cargo run -p multicad-app --example tool_case_review -- NEW_DIRECTORY")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&generate(Path::new(&path))?)?
    );
    Ok(())
}
