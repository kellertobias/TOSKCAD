//! Finished aluminium trim, purchased slide envelopes and installed hardware.
use super::{CaseBuilder, Error, actor, length};
use multicad_core::{
    commands::{Request, Session},
    document::{Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest,
    },
};

const WIDTH: f64 = 345.;
const DEPTH: f64 = 235.;
const HEIGHT: f64 = 310.;
const ANGLE: f64 = 20.;
const THICKNESS: f64 = 2.;
const RETURN: f64 = 70.;
const SLIDE_LENGTH: f64 = 200.;
const SLIDE_REAR: f64 = 224.;
const SLIDE_FRONT: f64 = SLIDE_REAR - SLIDE_LENGTH;
const SLIDE_HEIGHT: f64 = 45.1;
const SLIDE_WIDTH: f64 = 12.75;
const CASE_HOLES: [f64; 3] = [52.5, 147.4, 193.75];
const DRAWER_HOLES: [f64; 2] = [20.75, 164.5];
const HANDLE_Y: f64 = 136.;
const FRAME_BOLT_X: [f64; 2] = [14., 331.];
const HANDLE_POST_X: [f64; 2] = [108.5, 236.5];

pub(super) struct Metal {
    pub(super) moving: [[EntityId; 2]; 2],
    pub(super) fixed: [[EntityId; 2]; 2],
    pub(super) moving_hardware: [Vec<EntityId>; 2],
    pub(super) trim: Vec<EntityId>,
}

fn stock(name: &str, density: f64, color_rgba: [u8; 4]) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: Some(density),
            color_rgba,
            section_hatch: None,
        },
    )
}

#[derive(Clone, Copy)]
struct Block {
    origin: [f64; 3],
    size: [f64; 3],
}
impl Block {
    fn new(x: f64, y: f64, z: f64, dx: f64, dy: f64, dz: f64) -> Self {
        Self {
            origin: [x, y, z],
            size: [dx, dy, dz],
        }
    }
    fn mirrored(self, right: bool) -> Self {
        if right {
            let mut result = self;
            result.origin[0] = WIDTH - self.origin[0] - self.size[0];
            result
        } else {
            self
        }
    }
    fn parameters(self) -> PrimitiveParameters {
        PrimitiveParameters::Box {
            size: self.size.map(length),
            frame: PrimitiveFrame {
                origin: self.origin.map(length),
                ..Default::default()
            },
        }
    }
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
    Z,
}
fn frame(origin: [f64; 3], axis: Axis, reverse: bool) -> PrimitiveFrame {
    let (u, mut v) = match axis {
        Axis::X => ([0., 1., 0.], [0., 0., 1.]),
        Axis::Y => ([0., 0., 1.], [1., 0., 0.]),
        Axis::Z => ([1., 0., 0.], [0., 1., 0.]),
    };
    if reverse {
        v = v.map(|component| -component);
    }
    PrimitiveFrame {
        origin: origin.map(length),
        u,
        v,
    }
}
fn cylinder(origin: [f64; 3], axis: Axis, radius: f64, span: f64) -> PrimitiveParameters {
    PrimitiveParameters::Cylinder {
        radius: length(radius),
        height: length(span),
        frame: frame(origin, axis, false),
    }
}

/// The constituent solids are private until one complete candidate has passed
/// native compilation. Consumed join/cut bodies remain editable source.
fn solid(
    case: &mut CaseBuilder,
    name: &str,
    pieces: impl IntoIterator<Item = PrimitiveParameters>,
    cuts: impl IntoIterator<Item = PrimitiveParameters>,
) -> Result<EntityId, Error> {
    fn primitive(
        session: &mut Session,
        name: &str,
        parameters: PrimitiveParameters,
    ) -> Result<BodyInput, Error> {
        let request = PrimitiveRequest::new(name, parameters);
        let prepared = model::prepare_primitive(session, session.revision(), actor(), &request)?;
        session.commit(prepared.prepared)?;
        Ok(BodyInput {
            body: request.body,
            producer: request.feature,
        })
    }
    fn boolean(
        session: &mut Session,
        name: &str,
        operation: Operation,
        target: BodyInput,
        tools: Vec<BodyInput>,
    ) -> Result<BodyInput, Error> {
        if tools.is_empty() {
            return Ok(target);
        }
        let body = target.body;
        let mut request = BooleanRequest::new(operation, target, tools);
        request.name = name.into();
        let prepared = model::prepare_boolean(session, session.revision(), actor(), &request)?;
        session.commit(prepared.prepared)?;
        Ok(BodyInput {
            body,
            producer: request.feature,
        })
    }
    let mut staged = Session::new(case.session.document().clone())?;
    let mut pieces = pieces.into_iter();
    let first = primitive(
        &mut staged,
        name,
        pieces.next().ok_or("Metal part has no stock")?,
    )?;
    let join = pieces
        .map(|parameters| primitive(&mut staged, "Section stock", parameters))
        .collect::<Result<Vec<_>, _>>()?;
    let body = boolean(
        &mut staged,
        "Join formed section",
        Operation::Join,
        first,
        join,
    )?;
    let drills = cuts
        .into_iter()
        .map(|parameters| primitive(&mut staged, "Mounting bore", parameters))
        .collect::<Result<Vec<_>, _>>()?;
    let output = boolean(
        &mut staged,
        "Bore mounting pattern",
        Operation::Cut,
        body,
        drills,
    )?;
    commit_staged(case, &staged, &format!("Form {name}"))?;
    Ok(output.body)
}

fn commit_staged(case: &mut CaseBuilder, staged: &Session, label: &str) -> Result<(), Error> {
    let original = case.session.document();
    let candidate = staged.document();
    let mut edits = candidate
        .entities()
        .filter_map(|entity| match original.get(entity.id) {
            None => Some(Mutation::Insert(entity.clone())),
            Some(before) if before != entity => Some(Mutation::Replace(entity.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    if candidate.hierarchy() != original.hierarchy() {
        edits.push(Mutation::SetHierarchy(candidate.hierarchy().clone()));
    }
    let prepared = case.session.prepare(Request {
        expected_revision: case.session.revision(),
        actor: actor(),
        label: label.into(),
        edits,
    })?;
    if prepared.document() != candidate {
        return Err("The formed metal transaction differs from its prepared source".into());
    }
    case.checked(prepared)?;
    Ok(())
}

fn side_frame(case: &mut CaseBuilder, side: &str, right: bool) -> Result<EntityId, Error> {
    let (t, a, h, d, r) = (THICKNESS, ANGLE, HEIGHT, DEPTH, RETURN);
    let profile = [
        Block::new(0., 0., 0., t, a, h),
        Block::new(0., d - a, 0., t, a, h),
        Block::new(0., 0., 0., t, r, a),
        Block::new(0., d - r, 0., t, r, a),
        Block::new(0., 0., h - a, t, d, a),
        Block::new(0., 0., 0., 10., t, h),
        Block::new(0., d - t, 0., a, t, h),
        Block::new(0., 0., 0., a, r, t),
        Block::new(0., d - r, 0., a, r, t),
        Block::new(0., 0., h - t, a, d, t),
    ];
    solid(
        case,
        &format!("{side} aluminium side frame · L 20 × 20 × 2 · blank 995 mm"),
        profile
            .into_iter()
            .map(|block| block.mirrored(right).parameters()),
        [cylinder(
            [FRAME_BOLT_X[usize::from(right)], HANDLE_Y, h - t - 1.],
            Axis::Z,
            2.75,
            t + 2.,
        )],
    )
}

fn cross_angles(case: &mut CaseBuilder, aluminum: EntityId) -> Result<Vec<EntityId>, Error> {
    let (d, h, a, t) = (DEPTH, HEIGHT, ANGLE, THICKNESS);
    let handle = solid(
        case,
        "Handle cross-angle · L 20 × 20 × 2 · 328 mm",
        [
            Block::new(8.5, 125., 306., 328., a, t),
            Block::new(8.5, 125., 288., 328., t, a),
        ]
        .map(Block::parameters),
        FRAME_BOLT_X
            .into_iter()
            .chain(HANDLE_POST_X)
            .map(|x| cylinder([x, HANDLE_Y, 305.], Axis::Z, 2.75, 4.))
            .chain([52.5, 172.5, 292.5].map(|x| cylinder([x, 124., 298.], Axis::Y, 2.25, 4.))),
    )?;
    case.manufacture(handle, "fabricated", Some(aluminum))?;
    let mut parts = vec![handle];
    let x = a;
    let l = WIDTH - 2. * a;
    for (name, pieces) in [
        (
            "Top rear cross-angle · L 20 × 20 × 2 · 305 mm",
            [
                Block::new(x, d - a, h - t, l, a, t),
                Block::new(x, d - t, h - a, l, t, a),
            ],
        ),
        (
            "Bottom rear cross-angle · L 20 × 20 × 2 · 305 mm",
            [
                Block::new(x, d - a, 0., l, a, t),
                Block::new(x, d - t, 0., l, t, a),
            ],
        ),
        (
            "Bottom front cross-angle · L 20 × 10 × 2 · 305 mm",
            [
                Block::new(x, 0., 0., l, a, t),
                Block::new(x, 0., 0., l, t, 10.),
            ],
        ),
    ] {
        let part = solid(case, name, pieces.map(Block::parameters), [])?;
        case.manufacture(part, "fabricated", Some(aluminum))?;
        parts.push(part);
    }
    Ok(parts)
}

fn carry_handle(case: &mut CaseBuilder) -> Result<EntityId, Error> {
    let posts =
        HANDLE_POST_X.map(|x| Block::new(x - 6., HANDLE_Y - 6., 308., 12., 12., 35.).parameters());
    solid(
        case,
        "Carry handle · 128 mm fixing centres",
        posts
            .into_iter()
            .chain([cylinder([97.5, HANDLE_Y, 337.], Axis::X, 6., 150.)]),
        [],
    )
}

fn slide(
    case: &mut CaseBuilder,
    drawer: usize,
    side: &str,
    right: bool,
) -> Result<(EntityId, EntityId), Error> {
    let z = 35.5 + 49. * drawer as f64;
    let low = z - SLIDE_HEIGHT / 2.;
    let x = 10.;
    let t = 1.2;
    let fixed_name = format!(
        "Drawer {} · {side} 200 mm slide · fixed channel",
        drawer + 1
    );
    let moving_name = format!(
        "Drawer {} · {side} 200 mm slide · moving member",
        drawer + 1
    );
    let fixed = [
        Block::new(x, SLIDE_FRONT, low, t, SLIDE_LENGTH, SLIDE_HEIGHT),
        Block::new(x, SLIDE_FRONT, low, SLIDE_WIDTH, SLIDE_LENGTH, t),
        Block::new(
            x,
            SLIDE_FRONT,
            low + SLIDE_HEIGHT - t,
            SLIDE_WIDTH,
            SLIDE_LENGTH,
            t,
        ),
    ]
    .map(|block| block.mirrored(right).parameters());
    let outer_x = if right { WIDTH - x - t } else { x };
    let case_drills = CASE_HOLES
        .map(|rear| cylinder([outer_x - 1., SLIDE_REAR - rear, z], Axis::X, 2.25, t + 2.));
    let fixed = solid(case, &fixed_name, fixed, case_drills)?;
    let plate = Block::new(
        x + SLIDE_WIDTH - 2.,
        SLIDE_FRONT,
        z - 12.5,
        2.,
        SLIDE_LENGTH,
        25.,
    )
    .mirrored(right);
    let plate_x = plate.origin[0];
    let drawer_drills =
        DRAWER_HOLES.map(|rear| cylinder([plate_x - 1., SLIDE_REAR - rear, z], Axis::X, 2.25, 4.));
    let moving = solid(case, &moving_name, [plate.parameters()], drawer_drills)?;
    Ok((fixed, moving))
}

fn fastener_part(
    session: &mut Session,
    name: &str,
    parameters: model::MechanicalPartParameters,
) -> Result<EntityId, Error> {
    let request = model::MechanicalPartRequest::new(name, parameters);
    let prepared = model::prepare_mechanical_part(session, session.revision(), actor(), &request)?;
    session.commit(prepared.prepared)?;
    Ok(request.body)
}

fn screw(
    session: &mut Session,
    name: &str,
    origin: [f64; 3],
    axis: Axis,
    reverse: bool,
    diameter: u8,
    overall: f64,
) -> Result<EntityId, Error> {
    // Purchased fasteners use the specified countersink envelopes and metric
    // pitch. A straight head-to-shank junction omits the supplier's fillet.
    let (pitch, width, height, chamfer, drive, depth, runout) = match diameter {
        3 => (0.5, 6.4, 1.86, 0.03125, 2.05, 1.1, 0.6),
        4 => (0.7, 8.4, 2.48, 0.03125, 2.55, 1.5, 0.9),
        5 => (0.8, 10.9, 3.1, 0.0375, 3.05, 1.9, 1.),
        _ => return Err("Toolbox screw diameter must be M3, M4 or M5".into()),
    };
    let mut mount = frame(origin, axis, reverse);
    let index = match axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    };
    mount.origin[index] = length(origin[index] + if reverse { -height } else { height });
    fastener_part(
        session,
        name,
        model::MechanicalPartParameters::MachineScrew {
            underhead_length: None,
            length_input: Some(model::ScrewLengthInput::Overall {
                value: length(overall),
            }),
            shank_diameter: length(f64::from(diameter)),
            head: model::ScrewHead::Countersunk {
                profile: Box::new(model::CountersunkScrewHeadParameters {
                    diameter: length(width),
                    height: length(height),
                    included_angle: model::angle(90.),
                    edge_chamfer: length(chamfer),
                    neck_radius: length(0.),
                }),
            },
            drive: model::ScrewDrive::HexSocket {
                across_flats: length(drive),
                depth: length(depth),
            },
            thread: Box::new(model::ThreadParameters {
                major_diameter: length(f64::from(diameter)),
                pitch: length(pitch),
                length: length(overall - height - runout),
                representation: model::ThreadRepresentation::CosmeticEnvelope,
                frame: mount,
                ..Default::default()
            }),
        },
    )
}

fn nut(
    session: &mut Session,
    name: &str,
    origin: [f64; 3],
    axis: Axis,
    reverse: bool,
    diameter: u8,
) -> Result<EntityId, Error> {
    let (pitch, across_flats, height) = if diameter == 4 {
        (0.7, 7., 3.2)
    } else {
        (0.8, 8., 4.)
    };
    fastener_part(
        session,
        name,
        model::MechanicalPartParameters::HexNut {
            across_flats: length(across_flats),
            thread: Box::new(model::ThreadParameters {
                major_diameter: length(f64::from(diameter)),
                pitch: length(pitch),
                length: length(height),
                representation: model::ThreadRepresentation::CosmeticEnvelope,
                frame: frame(origin, axis, reverse),
                ..Default::default()
            }),
        },
    )
}

fn hardware(
    case: &mut CaseBuilder,
    result: &mut Metal,
    steel: EntityId,
    brass: EntityId,
) -> Result<(), Error> {
    let mut staged = Session::new(case.session.document().clone())?;
    let mut fasteners = Vec::new();
    let mut inserts = Vec::new();
    for drawer in 0..2 {
        let z = 35.5 + 49. * drawer as f64;
        for right in [false, true] {
            let x = |left| if right { WIDTH - left } else { left };
            for offset in CASE_HOLES {
                let y = SLIDE_REAR - offset;
                fasteners.push(screw(
                    &mut staged,
                    "M4 × 12 countersunk · fixed slide",
                    [x(2.), y, z],
                    Axis::X,
                    right,
                    4,
                    12.,
                )?);
                fasteners.push(nut(
                    &mut staged,
                    "M4 nut · fixed slide",
                    [x(11.2), y, z],
                    Axis::X,
                    right,
                    4,
                )?);
            }
            for offset in DRAWER_HOLES {
                let y = SLIDE_REAR - offset;
                result.moving_hardware[drawer].extend([
                    screw(
                        &mut staged,
                        "M4 countersunk · moving slide",
                        [x(20.75), y, z],
                        Axis::X,
                        right,
                        4,
                        10.,
                    )?,
                    nut(
                        &mut staged,
                        "M4 nut · moving slide",
                        [x(24.8), y, z],
                        Axis::X,
                        right,
                        4,
                    )?,
                ]);
            }
            for y in [22., 95.] {
                for z in [157.5, 244.5] {
                    fasteners.push(screw(
                        &mut staged,
                        "M3 countersunk · rack mounting",
                        [x(2.), y, z],
                        Axis::X,
                        right,
                        3,
                        12.,
                    )?);
                    inserts.push(fastener_part(
                        &mut staged,
                        "M3 heat-set insert · rack mounting",
                        model::MechanicalPartParameters::Washer {
                            bore_diameter: length(3.1),
                            outside_diameter: length(4.6),
                            thickness: length(5.),
                            frame: frame([x(8.), y, z], Axis::X, right),
                        },
                    )?);
                }
            }
        }
    }
    for x in FRAME_BOLT_X {
        fasteners.push(screw(
            &mut staged,
            "M5 × 12 · frame to handle cross-angle",
            [x, HANDLE_Y, 310.],
            Axis::Z,
            true,
            5,
            12.,
        )?);
        fasteners.push(nut(
            &mut staged,
            "M5 nut · frame to handle cross-angle",
            [x, HANDLE_Y, 306.],
            Axis::Z,
            true,
            5,
        )?);
    }
    for x in HANDLE_POST_X {
        fasteners.push(screw(
            &mut staged,
            "M5 · carry handle mounting",
            [x, HANDLE_Y, 306.],
            Axis::Z,
            false,
            5,
            10.,
        )?);
    }
    for x in [52.5, 172.5, 292.5] {
        fasteners.push(screw(
            &mut staged,
            "M4 · cross-angle to partition",
            [x, 119., 298.],
            Axis::Y,
            false,
            4,
            12.,
        )?);
        fasteners.push(nut(
            &mut staged,
            "M4 nut · cross-angle to partition",
            [x, 127., 298.],
            Axis::Y,
            false,
            4,
        )?);
    }
    commit_staged(case, &staged, "Install toolbox mounting hardware")?;
    for part in fasteners
        .into_iter()
        .chain(result.moving_hardware.iter().flatten().copied())
    {
        case.manufacture(part, "purchased", Some(steel))?;
    }
    for part in inserts {
        case.manufacture(part, "purchased", Some(brass))?;
    }
    Ok(())
}

pub(super) fn build(case: &mut CaseBuilder) -> Result<Metal, Error> {
    let aluminum = stock("Aluminium angle", 2700., [192, 201, 210, 255]);
    let slide_steel = stock("Slide and fastener steel", 7850., [126, 143, 160, 255]);
    let grip = stock("Dark carry handle", 1250., [48, 55, 63, 255]);
    let brass = stock("Brass heat-set inserts", 8500., [183, 144, 75, 255]);
    let (aluminum_id, steel_id, grip_id, brass_id) =
        (aluminum.id, slide_steel.id, grip.id, brass.id);
    case.checked(
        case.session.prepare(Request {
            expected_revision: case.session.revision(),
            actor: actor(),
            label: "Toolbox metal and hardware materials".into(),
            edits: [aluminum, slide_steel, grip, brass]
                .into_iter()
                .map(Mutation::Insert)
                .collect(),
        })?,
    )?;
    let mut trim = Vec::new();
    for (side, right) in [("Left", false), ("Right", true)] {
        let part = side_frame(case, side, right)?;
        case.manufacture(part, "fabricated", Some(aluminum_id))?;
        trim.push(part);
    }
    trim.extend(cross_angles(case, aluminum_id)?);
    let handle = carry_handle(case)?;
    case.manufacture(handle, "purchased", Some(grip_id))?;
    let mut result = Metal {
        moving: [[EntityId::nil(); 2]; 2],
        fixed: [[EntityId::nil(); 2]; 2],
        moving_hardware: Default::default(),
        trim,
    };
    for drawer in 0..2 {
        for (side_index, (side, right)) in
            [("left", false), ("right", true)].into_iter().enumerate()
        {
            let (fixed, moving) = slide(case, drawer, side, right)?;
            result.fixed[drawer][side_index] = fixed;
            result.moving[drawer][side_index] = moving;
            case.manufacture(fixed, "purchased", Some(steel_id))?;
            case.manufacture(moving, "purchased", Some(steel_id))?;
        }
    }
    hardware(case, &mut result, steel_id, brass_id)?;
    Ok(result)
}
