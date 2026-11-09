//! Editable printed parts for the plywood Werkzeugkoffer 2.
use super::{CaseBuilder, Error, actor, length};
use multicad_core::{
    commands::Request,
    constraint_sketch::{Axis, ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::{EntityData, EntityId},
    model::{
        self, BodyInput, BooleanRequest, Extent, ExtrudeParameters, ExtrudeRequest, Operation,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, Termination,
    },
    sketch::{self, CreationPlane, Datum, PlaneOptions},
    sketch_regions,
};

const OVERLAP: f64 = 0.05;
pub(super) const BIG_DEPTH: f64 = 224.;
const RACK_BASE: f64 = 115.;
const RACK_TOP: f64 = 301.;
const SMALL_DEPTH: f64 = 114.5;
const RACK_BACK: f64 = 119.;
const STRIP_FRONT: f64 = 3.5;
const STRIP_BACK: f64 = 115.;

pub(super) struct Printed {
    pub(super) large: [EntityId; 2],
    pub(super) small: [EntityId; 8],
    pub(super) rack: EntityId,
    pub(super) strips: Vec<EntityId>,
    pub(super) coupons: Vec<EntityId>,
}

pub(super) fn small_magnet_seats() -> [([f64; 3], [f64; 3]); 8] {
    bays().map(|bay| {
        let (x, z) = (bay.center(), bay.bottom + bay.height / 2.);
        ([x, SMALL_DEPTH - 3.5, z], [x, RACK_BACK - 3.1, z])
    })
}

#[derive(Clone, Copy)]
struct Bay {
    row: usize,
    column: usize,
    x: f64,
    width: f64,
    bottom: f64,
    height: f64,
}

impl Bay {
    fn center(self) -> f64 {
        self.x + self.width / 2.
    }

    fn runner(self) -> f64 {
        self.bottom + self.height / 2. - 1.5
    }

    fn walls(self) -> [(f64, f64); 2] {
        [(self.x, 1.), (self.x + self.width, -1.)]
    }
}

fn bays() -> [Bay; 8] {
    std::array::from_fn(|i| {
        let row = i / 2;
        let column = i % 2;
        Bay {
            row,
            column,
            x: [14., 225.][column],
            width: [206., 106.][column],
            bottom: 117. + row as f64 * 43.5,
            height: if row == 3 { 51.5 } else { 41.5 },
        }
    })
}

pub(super) fn build(case: &mut CaseBuilder) -> Result<Printed, Error> {
    let large = [big_drawer(case, 0)?, big_drawer(case, 1)?];
    let bays = bays();
    let mut small = [EntityId::nil(); 8];
    for (index, bay) in bays.iter().enumerate() {
        small[index] = small_drawer(case, *bay)?;
    }
    let rack = rack(case, &bays)?;
    let mut strips = Vec::with_capacity(16);
    for bay in bays {
        for (index, (x, side)) in bay.walls().into_iter().enumerate() {
            let name = format!(
                "Dovetail strip R{} C{} {}",
                bay.row + 1,
                bay.column + 1,
                if index == 0 { "left" } else { "right" }
            );
            let strip = track(case, &name, x, side, bay.runner(), STRIP_FRONT, STRIP_BACK)?;
            strips.push(printed(case, strip)?);
        }
    }
    let coupons = coupons(case, rack, small[1], bays[1])?;
    Ok(Printed {
        large,
        small,
        rack,
        strips,
        coupons,
    })
}

fn printed(case: &mut CaseBuilder, body: EntityId) -> Result<EntityId, Error> {
    case.manufacture(body, "printed", Some(case.petg))?;
    Ok(body)
}

fn input(case: &CaseBuilder, body: EntityId) -> Result<BodyInput, Error> {
    let entity = case
        .session
        .document()
        .get(body)
        .ok_or("Missing printed Part")?;
    let EntityData::Part { features, .. } = &entity.data else {
        return Err("Expected printed Part".into());
    };
    Ok(BodyInput {
        body,
        producer: *features.last().ok_or("Part has no features")?,
    })
}

fn boolean(
    case: &mut CaseBuilder,
    body: EntityId,
    name: &str,
    operation: Operation,
    tools: Vec<EntityId>,
    keep_tools: bool,
) -> Result<(), Error> {
    let mut request = BooleanRequest::new(
        operation,
        input(case, body)?,
        tools
            .into_iter()
            .map(|tool| input(case, tool))
            .collect::<Result<_, _>>()?,
    );
    request.name = name.into();
    request.keep_tools = keep_tools;
    let prepared =
        model::prepare_boolean(&case.session, case.session.revision(), actor(), &request)?;
    case.checked(prepared.prepared)
}

fn primitive(
    case: &mut CaseBuilder,
    name: &str,
    parameters: PrimitiveParameters,
) -> Result<EntityId, Error> {
    let request = PrimitiveRequest::new(name, parameters);
    let prepared =
        model::prepare_primitive(&case.session, case.session.revision(), actor(), &request)?;
    case.checked(prepared.prepared)?;
    Ok(request.body)
}

fn cylinder(
    case: &mut CaseBuilder,
    name: &str,
    datum: Datum,
    origin: [f64; 3],
    radius: f64,
    height: f64,
) -> Result<EntityId, Error> {
    let basis = datum.frame();
    primitive(
        case,
        name,
        PrimitiveParameters::Cylinder {
            radius: length(radius),
            height: length(height),
            frame: PrimitiveFrame {
                origin: origin.map(length),
                u: basis.u,
                v: basis.v,
            },
        },
    )
}

fn polygon(
    case: &mut CaseBuilder,
    name: &str,
    datum: Datum,
    span: [f64; 2],
    points: &[[f64; 2]],
    edit: impl FnOnce(&mut Source, &[EntityId]),
) -> Result<EntityId, Error> {
    let normal = datum.frame().n.into_iter().find(|n| *n != 0.).unwrap_or(1.);
    let new = sketch::prepare_new(
        &case.session,
        case.session.revision(),
        actor(),
        name,
        CreationPlane::Datum(datum),
        PlaneOptions {
            normal_offset_mm: normal * span[0],
            angle_rad: 0.,
        },
    )?;
    case.checked(new.prepared)?;
    let (mut source, _, _) = Source::read(case.session.document(), new.sketch)?;
    let ids: Vec<_> = points
        .iter()
        .map(|point| source.add_point(*point))
        .collect();
    for (&start, &end) in ids.iter().zip(ids.iter().cycle().skip(1)) {
        source.add_line(start, end);
    }
    edit(&mut source, &ids);
    // Capture curve identities from the authored loop. The native compiler
    // solves bound dimensions and resolves those same identities on rebuild.
    let extraction = sketch_regions::extract_geometry(&source.sketch.points, &source.sketch.curves);
    let [region] = extraction.regions.as_slice() else {
        return Err(format!("{name} must have one closed source region").into());
    };
    let edits = source.replace(case.session.document(), new.sketch)?;
    let prepared = case.session.prepare(Request {
        expected_revision: case.session.revision(),
        actor: actor(),
        label: format!("Author {name} profile"),
        edits,
    })?;
    case.checked(prepared)?;
    let mut request = ExtrudeRequest::new(
        new.sketch,
        vec![region.intent()],
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: length(normal * (span[1] - span[0])),
                },
            },
            ..Default::default()
        },
    );
    request.outputs[0].name = name.into();
    let prepared =
        model::prepare_extrude(&case.session, case.session.revision(), actor(), &request)?;
    case.checked(prepared.prepared)?;
    Ok(request.outputs[0].body)
}

fn prism(
    case: &mut CaseBuilder,
    name: &str,
    datum: Datum,
    span: [f64; 2],
    points: &[[f64; 2]],
) -> Result<EntityId, Error> {
    polygon(case, name, datum, span, points, |_, _| {})
}

fn constrain_point(
    source: &mut Source,
    origin: EntityId,
    point: EntityId,
    x: Dimension,
    y: Dimension,
) {
    for (axis, dimension) in [(Axis::X, x), (Axis::Y, y)] {
        source.add_constraint(ConstraintKind::Distance {
            a: origin,
            b: point,
            axis,
            dimension,
        });
    }
}

#[derive(Clone, Copy)]
struct Drawer {
    x: f64,
    width: f64,
    depth: f64,
    z: f64,
    height: f64,
    floor: f64,
    side: f64,
    front: f64,
    back: f64,
    fascia: [f64; 2],
}

struct Pull {
    width: f64,
    slot: f64,
    pocket: f64,
    depth: f64,
    wall: f64,
    rim: f64,
}

impl Drawer {
    fn top(self) -> f64 {
        self.z + self.height
    }

    fn inner_floor(self) -> f64 {
        self.z + self.floor
    }

    fn shell(self, case: &mut CaseBuilder, name: &str) -> Result<EntityId, Error> {
        let body = case.box_part(
            name,
            [self.x, 0., self.z],
            [self.width, self.depth, self.height],
        )?;
        let cavity = case.box_part(
            "Drawer clear interior",
            [self.x + self.side, self.front, self.inner_floor()],
            [
                self.width - 2. * self.side,
                self.depth - self.front - self.back,
                self.height - self.floor + OVERLAP,
            ],
        )?;
        boolean(
            case,
            body,
            "Open drawer cavity",
            Operation::Cut,
            vec![cavity],
            false,
        )?;
        Ok(body)
    }

    fn fascia(self, case: &mut CaseBuilder) -> Result<EntityId, Error> {
        case.box_part(
            "Front fascia",
            [self.fascia[0], 0., self.z],
            [self.fascia[1], self.front, self.height],
        )
    }

    fn reinforcements(
        self,
        case: &mut CaseBuilder,
        rim: f64,
        fillet: f64,
        gusset: f64,
    ) -> Result<Vec<EntityId>, Error> {
        let x0 = self.x + self.side;
        let x1 = self.x + self.width - self.side;
        let y0 = self.front;
        let y1 = self.depth - self.back;
        let z = self.inner_floor();
        let top = self.top();
        let mut pieces = Vec::with_capacity(12);
        for (datum, span, a, b) in [(Datum::Xz, [y0, y1], x0, x1), (Datum::Yz, [x0, x1], y0, y1)] {
            for (at, inward) in [(a, 1.), (b, -1.)] {
                let overlap = at - inward * OVERLAP;
                pieces.push(prism(
                    case,
                    "Sloped reinforcing rim",
                    datum,
                    span,
                    &[
                        [overlap, top - 2. * rim],
                        [at + inward * rim, top - rim],
                        [at + inward * rim, top],
                        [overlap, top],
                    ],
                )?);
                pieces.push(polygon(
                    case,
                    "Internal floor fillet",
                    datum,
                    span,
                    &[
                        [overlap, z - OVERLAP],
                        [at + inward * fillet, z - OVERLAP],
                        [at + inward * fillet, z],
                        [at, z + fillet],
                        [overlap, z + fillet],
                    ],
                    |source, ids| {
                        let center = source.add_point([at + inward * fillet, z + fillet]);
                        source.sketch.curves[2] = Curve {
                            id: EntityId::new(),
                            construction: false,
                            kind: CurveKind::Arc {
                                center,
                                start: ids[2],
                                end: ids[3],
                                clockwise: inward > 0.,
                            },
                        };
                    },
                )?);
            }
        }
        for (x, dx) in [(x0, gusset), (x1, -gusset)] {
            for (y, dy) in [(y0, gusset), (y1, -gusset)] {
                pieces.push(prism(
                    case,
                    "Interior corner gusset",
                    Datum::Xy,
                    [z - OVERLAP, top],
                    &[
                        [x - dx.signum() * OVERLAP, y - dy.signum() * OVERLAP],
                        [x + dx, y - dy.signum() * OVERLAP],
                        [x - dx.signum() * OVERLAP, y + dy],
                    ],
                )?);
            }
        }
        Ok(pieces)
    }

    fn pull(self, case: &mut CaseBuilder, pull: Pull) -> Result<(EntityId, EntityId), Error> {
        let Pull {
            width,
            slot,
            pocket,
            depth,
            wall,
            rim,
        } = pull;
        let cx = self.x + self.width / 2.;
        let z = self.inner_floor();
        let backing = case.box_part(
            "Recessed pull backing",
            [cx - width / 2. - wall, 0., z - OVERLAP],
            [
                width + 2. * wall,
                depth + wall,
                self.top() - z - rim + OVERLAP,
            ],
        )?;
        let hollow = prism(
            case,
            "Recessed pull with support-free slope",
            Datum::Yz,
            [cx - width / 2., cx + width / 2.],
            &[
                [-1., z],
                [depth, z],
                [depth, z + pocket],
                [self.front, z + pocket - (depth - self.front)],
                [-1., z + slot],
            ],
        )?;
        Ok((backing, hollow))
    }
}

fn big_drawer(case: &mut CaseBuilder, row: usize) -> Result<EntityId, Error> {
    let z = 11.75 + row as f64 * 49.;
    let d = Drawer {
        x: 23.,
        width: 299.,
        depth: BIG_DEPTH,
        z,
        height: 47.5,
        floor: 3.5,
        side: 5.,
        front: 6.,
        back: 4.,
        fascia: [10.5, 324.],
    };
    let name = if row == 0 {
        "Lower large drawer · discs and pliers"
    } else {
        "Upper large drawer · measuring tools and cutters"
    };
    let body = d.shell(case, name)?;
    let mut joins = d.reinforcements(case, 3., 4., 6.)?;
    let (backing, pull) = d.pull(
        case,
        Pull {
            width: 90.,
            slot: 18.,
            pocket: 34.,
            depth: 16.,
            wall: 2.,
            rim: 3.,
        },
    )?;
    joins.extend([d.fascia(case)?, backing]);
    let divider_z = d.inner_floor() - OVERLAP;
    let divider_h = d.top() - 3. - divider_z;
    if row == 0 {
        for (name, delta) in [
            ("Disc compartment divider", 28.),
            ("Pliers compartment divider", 99.),
        ] {
            let x = case.bound(&format!("=$drawer_disk_cell + {delta} mm"))?;
            joins.push(primitive(
                case,
                name,
                PrimitiveParameters::Box {
                    size: [2., 214. + 2. * OVERLAP, divider_h].map(length),
                    frame: PrimitiveFrame {
                        origin: [x, length(6. - OVERLAP), length(divider_z)],
                        ..Default::default()
                    },
                },
            )?);
        }
        let divider_width = case.bound("=$drawer_disk_cell + 0.1 mm")?;
        joins.push(primitive(
            case,
            "Disc compartment cross divider",
            PrimitiveParameters::Box {
                size: [divider_width, length(2.), length(divider_h)],
                frame: PrimitiveFrame {
                    origin: [27.95, 136., divider_z].map(length),
                    ..Default::default()
                },
            },
        )?);
    } else {
        for (name, origin, size) in [
            (
                "Measuring-tool cross divider",
                [27.95, 90., divider_z],
                [289.1, 2., divider_h],
            ),
            (
                "Cutter compartment divider",
                [227., 91.95, divider_z],
                [2., 128.1, divider_h],
            ),
            (
                "Drill compartment divider",
                [277., 91.95, divider_z],
                [2., 128.1, divider_h],
            ),
            (
                "Short caliper divider",
                [274., 5.95, divider_z],
                [2., 84.1, 18.05],
            ),
        ] {
            joins.push(case.box_part(name, origin, size)?);
        }
    }

    let mut cuts = vec![pull];
    let center_z = z + d.height / 2.;
    for side in [1., -1.] {
        let wall = if side > 0. { d.x } else { d.x + d.width };
        for rear_offset in [20.75, 164.5] {
            let y = BIG_DEPTH - rear_offset;
            let boss = wall + side * (d.side - OVERLAP);
            joins.push(cylinder(
                case,
                "M4 mounting boss",
                Datum::Yz,
                [boss.min(boss + side * 3.55), y, center_z],
                6.5,
                3.55,
            )?);
            let hole = wall - side * OVERLAP;
            cuts.push(cylinder(
                case,
                "Ø4.5 rail screw through-hole",
                Datum::Yz,
                [hole.min(hole + side * 8.7), y, center_z],
                2.25,
                8.7,
            )?);
            let inside = wall + side * (d.side + 3.5);
            let radius = 7.4 / 3_f64.sqrt();
            let hex = std::array::from_fn::<_, 6, _>(|i| {
                let theta = i as f64 * std::f64::consts::FRAC_PI_3;
                [y + radius * theta.cos(), center_z + radius * theta.sin()]
            });
            cuts.push(prism(
                case,
                "M4 captive nut pocket · 7.4 AF",
                Datum::Yz,
                [inside - side * 3.4, inside + side * OVERLAP],
                &hex,
            )?);
        }
    }
    for x in [82.5, 262.5] {
        joins.push(cylinder(
            case,
            "Rear magnet reinforcement",
            Datum::Xz,
            [x, BIG_DEPTH - d.back + OVERLAP, center_z],
            8.,
            2.,
        )?);
        cuts.push(cylinder(
            case,
            "Ø10 magnet pocket",
            Datum::Xz,
            [x, BIG_DEPTH + OVERLAP, center_z],
            5.1,
            3.15,
        )?);
    }
    if row == 1 {
        cuts.push(case.box_part(
            "Upper drawer front trim relief",
            [10., -OVERLAP, 107.25],
            [325., 10. + OVERLAP, 1.1],
        )?);
    }
    boolean(
        case,
        body,
        "Reinforce drawer and add compartments",
        Operation::Join,
        joins,
        false,
    )?;
    boolean(
        case,
        body,
        "Cut pull, mounting and magnet pockets",
        Operation::Cut,
        cuts,
        false,
    )?;
    printed(case, body)
}

fn small_drawer(case: &mut CaseBuilder, bay: Bay) -> Result<EntityId, Error> {
    let d = Drawer {
        x: bay.x + 3.,
        width: bay.width - 6.,
        depth: SMALL_DEPTH,
        z: bay.bottom + 0.5,
        height: bay.height - 1.,
        floor: 2.5,
        side: 2.,
        front: 3.,
        back: 2.,
        fascia: [bay.x + 0.4, bay.width - 0.8],
    };
    let name = format!("Small drawer R{} C{}", bay.row + 1, bay.column + 1);
    let body = d.shell(case, &name)?;
    let mut joins = d.reinforcements(case, 2., 2., 4.)?;
    let (backing, pull) = d.pull(
        case,
        Pull {
            width: 40.,
            slot: 12.,
            pocket: 22.,
            depth: 10.,
            wall: 1.6,
            rim: 2.,
        },
    )?;
    joins.extend([d.fascia(case)?, backing]);

    let runner = bay.runner();
    for (x, side) in [(d.x, -1.), (d.x + d.width, 1.)] {
        let outside = case.bound(&format!("={x} mm + {side} * (2.8 mm - $small_runner_play)"))?;
        let inside = x - side * OVERLAP;
        let reach = x + side * 2.4;
        joins.push(polygon(
            case,
            "Small drawer sliding rib",
            Datum::Xz,
            [3. - OVERLAP, SMALL_DEPTH],
            &[
                [inside, runner - 2.4],
                [reach, runner],
                [reach, runner + 3.],
                [inside, runner + 3.],
            ],
            move |source, ids| {
                let origin = source.add_point([0., 0.]);
                source.add_constraint(ConstraintKind::Fix { point: origin });
                constrain_point(source, origin, ids[0], length(inside), length(runner - 2.4));
                constrain_point(source, origin, ids[1], outside.clone(), length(runner));
                constrain_point(source, origin, ids[2], outside, length(runner + 3.));
                constrain_point(source, origin, ids[3], length(inside), length(runner + 3.));
            },
        )?);
    }
    let mx = bay.center();
    let mz = bay.bottom + bay.height / 2.;
    joins.push(case.box_part(
        "Retained magnet column",
        [mx - 3.8, SMALL_DEPTH - 4.9, d.inner_floor() - OVERLAP],
        [7.6, 4.9, d.top() - d.inner_floor() + OVERLAP],
    )?);
    let slot = case.box_part(
        "Ø5 × 3.1 magnet slot · 0.4 rear skin",
        [mx - 2.6, SMALL_DEPTH - 3.7, mz - 2.6],
        [5.2, 3.3, d.top() - mz + 2.6 + OVERLAP],
    )?;
    let mut cuts = vec![pull, slot];
    for (x, side) in [(d.x, 1.), (d.x + d.width, -1.)] {
        let outside = x - side * 3.;
        cuts.push(prism(
            case,
            "Small drawer rear lead-in chamfer",
            Datum::Xy,
            [d.z - OVERLAP, d.top() + OVERLAP],
            &[
                [outside, SMALL_DEPTH - 3.5],
                [x + side * 1.5, SMALL_DEPTH + OVERLAP],
                [outside, SMALL_DEPTH + OVERLAP],
            ],
        )?);
    }
    boolean(
        case,
        body,
        "Reinforce drawer and add runners",
        Operation::Join,
        joins,
        false,
    )?;
    boolean(
        case,
        body,
        "Cut front pull, magnet slot and rear lead-ins",
        Operation::Cut,
        cuts,
        false,
    )?;
    printed(case, body)
}

fn rack(case: &mut CaseBuilder, bays: &[Bay; 8]) -> Result<EntityId, Error> {
    let body = case.box_part(
        "Small-drawer rack · 8 bays",
        [8., STRIP_BACK, RACK_BASE],
        [329., 4., RACK_TOP - RACK_BASE],
    )?;
    let mut joins = vec![
        case.box_part(
            "Rack left cheek",
            [8., 0., RACK_BASE],
            [6., RACK_BACK, 186.],
        )?,
        case.box_part(
            "Rack right cheek",
            [331., 0., RACK_BASE],
            [6., RACK_BACK, 186.],
        )?,
        case.box_part(
            "Rack bottom plate",
            [8., 0., RACK_BASE],
            [329., RACK_BACK, 2.],
        )?,
        case.box_part(
            "Rack top plate",
            [8., 0., RACK_TOP - 2.],
            [329., RACK_BACK, 2.],
        )?,
        case.box_part(
            "Rack center web",
            [220., 0., RACK_BASE],
            [5., RACK_BACK, 186.],
        )?,
    ];
    let shelves = [bays[0], bays[2], bays[4]].map(|bay| bay.bottom + bay.height);
    for z in shelves {
        joins.push(case.box_part("Rack dust shelf", [8., 0., z], [329., RACK_BACK, 2.])?);
    }
    boolean(
        case,
        body,
        "Join rack cheeks, plates, shelves and divider",
        Operation::Join,
        joins,
        false,
    )?;

    let mut cuts = Vec::new();
    for bay in bays {
        for (x, side) in bay.walls() {
            cuts.push(dovetail_slot(case, x, side, bay.runner())?);
        }
        cuts.push(cylinder(
            case,
            "Rear-load Ø5 magnet pocket",
            Datum::Xz,
            [
                bay.center(),
                RACK_BACK + OVERLAP,
                bay.bottom + bay.height / 2.,
            ],
            2.6,
            3.25,
        )?);
    }
    // Match the side-panel DXF holes in world coordinates.
    for z in [157.5, 244.5] {
        for y in [22., 95.] {
            for x in [8. - OVERLAP, 337. - 5.] {
                cuts.push(cylinder(
                    case,
                    "M3 heat-set insert bore · Ø4 × 5",
                    Datum::Yz,
                    [x, y, z],
                    2.,
                    5. + OVERLAP,
                )?);
            }
        }
    }
    for (origin, size) in [
        (
            [8. - OVERLAP, -OVERLAP, RACK_BASE - OVERLAP],
            [2. + OVERLAP, 10. + OVERLAP, 186. + 2. * OVERLAP],
        ),
        (
            [335., -OVERLAP, RACK_BASE - OVERLAP],
            [2. + OVERLAP, 10. + OVERLAP, 186. + 2. * OVERLAP],
        ),
        (
            [8. - OVERLAP, -OVERLAP, RACK_BASE - OVERLAP],
            [329. + 2. * OVERLAP, 10. + OVERLAP, 2. + OVERLAP],
        ),
        (
            [8. - OVERLAP, -OVERLAP, RACK_TOP - 2.],
            [329. + 2. * OVERLAP, 10. + OVERLAP, 2. + OVERLAP],
        ),
    ] {
        cuts.push(case.box_part("2 mm front trim recess", origin, size)?);
    }
    boolean(
        case,
        body,
        "Cut dovetails, magnet and insert seats, and front relief",
        Operation::Cut,
        cuts,
        false,
    )?;
    printed(case, body)
}

fn track(
    case: &mut CaseBuilder,
    name: &str,
    x: f64,
    side: f64,
    z: f64,
    y0: f64,
    y1: f64,
) -> Result<EntityId, Error> {
    let points = [
        [0., -5.7],
        [-1.6, -6.3],
        [-1.6, 7.3],
        [0., 6.7],
        [0., 5.9],
        [2.6, 5.9],
        [2.6, 3.4],
        [0., 3.4],
        [0., 0.2],
        [2.6, -2.4],
        [2.6, -4.9],
        [0., -4.9],
    ]
    .map(|[u, v]| [x + side * u, z + v]);
    prism(case, name, Datum::Xz, [y0, y1], &points)
}

fn dovetail_slot(case: &mut CaseBuilder, x: f64, side: f64, z: f64) -> Result<EntityId, Error> {
    let points = [
        [x + side * OVERLAP, z - 5.85],
        [x - side * 1.75, z - 6.45],
        [x - side * 1.75, z + 7.45],
        [x + side * OVERLAP, z + 6.85],
    ];
    let outer = case.bound(&format!("={x} mm - {side} * (1.6 mm + $dovetail_fit)"))?;
    let z_dimensions = [
        case.bound(&format!("={} mm - $dovetail_fit", z - 5.7))?,
        case.bound(&format!("={} mm - $dovetail_fit", z - 6.3))?,
        case.bound(&format!("={} mm + $dovetail_fit", z + 7.3))?,
        case.bound(&format!("={} mm + $dovetail_fit", z + 6.7))?,
    ];
    polygon(
        case,
        "Dovetail groove with editable fit",
        Datum::Xz,
        [-OVERLAP, STRIP_BACK + 0.15],
        &points,
        move |source, ids| {
            let origin = source.add_point([0., 0.]);
            source.add_constraint(ConstraintKind::Fix { point: origin });
            let inner = length(x + side * OVERLAP);
            for ((&id, along), up) in ids
                .iter()
                .zip([inner.clone(), outer.clone(), outer, inner])
                .zip(z_dimensions)
            {
                constrain_point(source, origin, id, along, up);
            }
        },
    )
}

fn coupons(
    case: &mut CaseBuilder,
    rack: EntityId,
    drawer: EntityId,
    bay: Bay,
) -> Result<Vec<EntityId>, Error> {
    let rack_coupon = case.box_part(
        "Fit coupon · rack front slice",
        [219., -OVERLAP, RACK_BASE - OVERLAP],
        [118. + OVERLAP, 30. + OVERLAP, 44. + OVERLAP],
    )?;
    boolean(
        case,
        rack_coupon,
        "Crop the finished rack to a 30 mm coupon",
        Operation::Intersect,
        vec![rack],
        true,
    )?;
    let drawer_coupon = case.box_part(
        "Fit coupon · small drawer front slice",
        [bay.x - 1., -OVERLAP, bay.bottom - OVERLAP],
        [bay.width + 2., 30. + OVERLAP, bay.height + 1.],
    )?;
    boolean(
        case,
        drawer_coupon,
        "Crop the finished drawer to a 30 mm coupon",
        Operation::Intersect,
        vec![drawer],
        true,
    )?;
    let short_strips = [
        track(
            case,
            "Fit coupon · dovetail strip A",
            360.,
            1.,
            10.,
            STRIP_FRONT,
            30.,
        )?,
        track(
            case,
            "Fit coupon · dovetail strip B",
            369.,
            1.,
            10.,
            STRIP_FRONT,
            30.,
        )?,
    ];
    [rack_coupon, drawer_coupon, short_strips[0], short_strips[1]]
        .into_iter()
        .map(|body| printed(case, body))
        .collect()
}
