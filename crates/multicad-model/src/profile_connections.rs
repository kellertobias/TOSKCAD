//! Solids for item Line 8 mounting recipes. Access bores deliberately
//! cross interrupted stock, so they use a stepped cutter with retained proof.
use crate::{GeometryBackend, ModelError, error};
use multicad_core::{
    document::{Document, EntityId},
    model::{
        self, CountersunkScrewHeadParameters, CrownedScrewHeadParameters,
        ITEM_LINE_8_GROOVE_LIP_DEPTH_MM, MechanicalPartParameters, PROFILE_ACCESS_FROM_END_MM,
        PROFILE_ACCESS_RADIUS_MM, PROFILE_BARREL_DEPTH_MM, PROFILE_PILOT_DEPTH_MM,
        PROFILE_PILOT_RADIUS_MM, PathSegment, ProfileAttachment, ProfileConnectionFrames,
        ProfileConnectionKind, ProfileConnectionNut, ProfileOutputKind, ProfilePartRole,
        ResolvedMechanicalPart, ResolvedThread, ScrewDrive, ScrewHead, ScrewLengthInput,
        ThreadMode, ThreadParameters, length,
    },
    parameters::EvaluatedParameters,
    references::{AnalyticGeometry, Origin, ReferenceFrame, TopologySnapshot},
};
use multicad_kernel::{
    BooleanOperation, CircularPocketTool, CurvedEdge2d, CurvedProfile, EdgeTreatment,
    GeometryPolicy, PlaneFrame, RevolutionAxis, RigidTransform, Solid, TreatmentEdge,
};
use std::{collections::BTreeMap, f64::consts::TAU};

const SHEET_MM: f64 = 5.0;
const NUT_THREAD_MM: f64 = 4.1;
const BORE_RADIUS_MM: f64 = 4.9;
const BORE_MOUTH_RADIUS_MM: f64 = 8.04;
const BORE_OVERTRAVEL_MM: f64 = 0.02;
const PLATE_PITCH_MM: f64 = 40.;
const PLATE_WIDTH_MM: f64 = 80.;
const PLATE_DEPTH_MM: f64 = 40.;
const HARDWARE_CLEARANCE_VOLUME_MM3: f64 = 0.001;
// Assembly engineering clearance, independent of catalog and ISO fit classes.
const THREAD_RADIAL_CLEARANCE_MM: f64 = 0.08;
const THREAD_AXIAL_CLEARANCE_MM: f64 = 0.04;

type Point = [f64; 3];
fn add(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn scale(v: Point, factor: f64) -> Point {
    v.map(|x| x * factor)
}
fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn magnitude([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}
fn shift(frame: ReferenceFrame, distance: f64) -> ReferenceFrame {
    ReferenceFrame {
        origin_mm: add(frame.origin_mm, scale(frame.normal(), distance)),
        ..frame
    }
}
fn plane(frame: ReferenceFrame) -> PlaneFrame {
    PlaneFrame {
        origin: frame.origin_mm,
        u: frame.u,
        v: frame.v,
    }
}
fn placement(frame: ReferenceFrame) -> RigidTransform {
    let [u, v, n] = [frame.u, frame.v, frame.normal()];
    RigidTransform {
        rotation: std::array::from_fn(|i| [u[i], v[i], n[i]]),
        translation_mm: frame.origin_mm,
    }
}
fn polygon(points: &[[f64; 2]]) -> Vec<CurvedEdge2d> {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| CurvedEdge2d::Line { a, b })
        .collect()
}
fn rotated(frame: ReferenceFrame, angle: f64) -> ReferenceFrame {
    let (s, c) = angle.sin_cos();
    ReferenceFrame {
        u: add(scale(frame.u, c), scale(frame.v, s)),
        v: add(scale(frame.u, -s), scale(frame.v, c)),
        ..frame
    }
}

fn joining_plate_frame([a, b]: [ReferenceFrame; 2]) -> ReferenceFrame {
    let u = scale(sub(b.origin_mm, a.origin_mm), 1. / PLATE_PITCH_MM);
    let v = cross(a.normal(), u);
    ReferenceFrame {
        origin_mm: sub(
            sub(a.origin_mm, scale(u, PLATE_PITCH_MM / 2.)),
            scale(v, PLATE_DEPTH_MM / 2.),
        ),
        u,
        v,
    }
}

pub(crate) struct Inputs<'a> {
    pub document: &'a Document,
    pub parameters: &'a EvaluatedParameters,
    pub revision: u64,
    pub snapshots: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub paths: &'a BTreeMap<EntityId, PathSegment>,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub policy: GeometryPolicy,
}

struct Builder<'a> {
    inputs: Inputs<'a>,
    feature: EntityId,
}
impl Builder<'_> {
    fn checked<T, E: std::fmt::Display>(&self, value: Result<T, E>) -> Result<T, ModelError> {
        value.map_err(|e| error(Some(self.feature), e))
    }
    fn fail(&self, message: impl std::fmt::Display) -> ModelError {
        error(Some(self.feature), message)
    }
    fn solid(&self, producer: EntityId) -> Result<&Solid, ModelError> {
        self.inputs.solids.get(&producer).ok_or_else(|| {
            self.fail("A profile predecessor has no current native geometry. Repair the member reference.")
        })
    }
    fn member(&self, site: &ProfileAttachment) -> Result<model::ResolvedMember, ModelError> {
        let source = self.checked(model::read_member(
            self.inputs.document,
            site.member.producer,
        ))?;
        let path = self
            .inputs
            .paths
            .get(&site.member.producer)
            .ok_or_else(|| {
                self.fail(
                    "The retained profile member path is unavailable. Repair the member source.",
                )
            })?;
        self.checked(model::resolve_member(
            self.inputs.document,
            self.inputs.parameters,
            &source,
            *path,
        ))
    }
    fn cylinder(&self, frame: ReferenceFrame, r: f64, h: f64) -> Result<Solid, ModelError> {
        self.checked(Solid::cylinder(plane(frame), r, h, self.inputs.policy))
    }
    fn cuboid(&self, frame: ReferenceFrame, size: Point) -> Result<Solid, ModelError> {
        self.checked(Solid::box_solid(size, self.inputs.policy))?
            .transformed(placement(frame))
            .map_err(|e| self.fail(e))
    }
    fn subtract(&self, stock: &Solid, tool: &Solid, label: &str) -> Result<Solid, ModelError> {
        self.checked(stock.boolean(tool, BooleanOperation::Subtract, self.inputs.policy))?
            .ok_or_else(|| self.fail(label))
    }
    fn check_clear(&self, a: &Solid, b: &Solid, message: &str) -> Result<(), ModelError> {
        if let Some(overlap) =
            self.checked(a.boolean(b, BooleanOperation::Intersect, self.inputs.policy))?
            && self.checked(overlap.volume_mm3())? > HARDWARE_CLEARANCE_VOLUME_MM3
        {
            return Err(self.fail(message));
        }
        Ok(())
    }

    fn countersink(&self, frame: ReferenceFrame) -> Result<Solid, ModelError> {
        let top = shift(frame, -SHEET_MM - BORE_OVERTRAVEL_MM);
        let radius = BORE_MOUTH_RADIUS_MM;
        let taper_end = radius - BORE_RADIUS_MM;
        let depth = SHEET_MM + 2. * BORE_OVERTRAVEL_MM;
        let outline = [
            [0.0, 0.0],
            [radius, 0.0],
            [BORE_RADIUS_MM, taper_end],
            [BORE_RADIUS_MM, depth],
            [0.0, depth],
        ];
        self.checked(Solid::revolve_curved_profile(
            &CurvedProfile {
                frame: PlaneFrame {
                    origin: top.origin_mm,
                    u: top.u,
                    v: top.normal(),
                },
                outer: polygon(&outline),
                holes: vec![],
            },
            RevolutionAxis {
                origin_mm: top.origin_mm,
                direction: top.normal(),
            },
            TAU,
            self.inputs.policy,
        ))
    }

    fn plate(
        &self,
        kind: ProfileConnectionKind,
        mount: ProfileConnectionFrames,
    ) -> Result<Solid, ModelError> {
        let [a, b] = mount.sites;
        let mut plate = if kind == ProfileConnectionKind::AngleBracket {
            let corner = sub(a.origin_mm, scale(b.normal(), 20.0));
            let leg = |face: ReferenceFrame, other: ReferenceFrame| {
                let u = other.normal();
                let v = cross(face.normal(), u);
                self.cuboid(
                    ReferenceFrame {
                        origin_mm: sub(corner, scale(v, 20.0)),
                        u,
                        v,
                    },
                    [40.0, 40.0, SHEET_MM],
                )
            };
            self.checked(leg(a, b)?.boolean(
                &leg(b, a)?,
                BooleanOperation::Union,
                self.inputs.policy,
            ))?
            .ok_or_else(|| self.fail("Angle bracket legs do not meet at their bend"))?
        } else {
            self.cuboid(
                joining_plate_frame(mount.sites),
                [PLATE_WIDTH_MM, PLATE_DEPTH_MM, SHEET_MM],
            )?
        };
        for frame in mount.fasteners {
            plate = self.subtract(
                &plate,
                &self.countersink(frame)?,
                "Countersink removed the joining part",
            )?;
        }
        Ok(plate)
    }

    fn rust_joining_plate(&self, mount: ProfileConnectionFrames) -> Result<Solid, ModelError> {
        let frame = joining_plate_frame(mount.sites);
        self.checked(frame.validate())?;
        let policy = self.inputs.policy;
        let normal = frame.normal();
        let outer = [
            [0., 0.],
            [PLATE_WIDTH_MM, 0.],
            [PLATE_WIDTH_MM, PLATE_DEPTH_MM],
            [0., PLATE_DEPTH_MM],
        ];
        // A Line 8 groove datum is an outward support plane of the full
        // member section. A corner check therefore proves clearance of the
        // plate enclosure, including for rigidly placed profile occurrences.
        let tolerance = policy.linear_tolerance_mm;
        let penetration =
            tolerance.min(HARDWARE_CLEARANCE_VOLUME_MM3 / (2. * PLATE_WIDTH_MM * PLATE_DEPTH_MM));
        for site in mount.sites {
            for point in outer.map(|uv| plane(frame).point(uv)) {
                for point in [point, add(point, scale(normal, SHEET_MM))] {
                    if dot(sub(point, site.origin_mm), site.normal()) < -penetration {
                        return Err(self.fail(
                            "Joining plate cannot be certified clear of the retained profile surfaces",
                        ));
                    }
                }
            }
        }

        let mut tools = Vec::with_capacity(2);
        let mut countersinks = Vec::with_capacity(2);
        for (i, axis) in mount.fasteners.into_iter().enumerate() {
            let inward = axis.normal();
            if dot(normal, inward) >= 0. || magnitude(cross(normal, inward)) > 1e-12 {
                return Err(self.fail(
                    "Rust joining plate countersinks require axes perpendicular to the plate",
                ));
            }
            let top = shift(axis, -SHEET_MM - BORE_OVERTRAVEL_MM);
            let entry = dot(sub(top.origin_mm, frame.origin_mm), normal) - SHEET_MM;
            let chamfer = BORE_MOUTH_RADIUS_MM - BORE_RADIUS_MM - entry;
            if !(0. ..2. * BORE_OVERTRAVEL_MM).contains(&entry)
                || chamfer <= 0.
                || chamfer >= SHEET_MM
            {
                return Err(self.fail(
                    "The joining plate countersink tool must cut through both plate surfaces",
                ));
            }
            tools.push(CircularPocketTool {
                frame: plane(top),
                center_uv: [0., 0.],
                radius_mm: BORE_RADIUS_MM,
                displacement_mm: scale(inward, SHEET_MM + 2. * BORE_OVERTRAVEL_MM),
                origin: Origin {
                    source: self.feature,
                    role: format!("joining_plate.bore.{i}"),
                },
            });
            countersinks.push((add(top.origin_mm, scale(inward, entry)), chamfer));
        }
        let profile = CurvedProfile {
            frame: plane(frame),
            outer: polygon(&outer),
            holes: vec![],
        };
        let origins: Vec<_> = (0..profile.outer.len())
            .map(|i| Origin {
                source: self.feature,
                role: format!("joining_plate.outline.{i}"),
            })
            .collect();
        let blank =
            crate::rust_backend::stock_profile(self.feature, &profile, &origins, SHEET_MM, policy)?;
        let mut plate =
            self.checked(blank.rust_drill_circular_machining(&tools, &[true; 2], policy))?;
        for (i, (mouth, distance_mm)) in countersinks.into_iter().enumerate() {
            let snapshot =
                self.checked(plate.topology_snapshot(self.feature, self.inputs.revision))?;
            let edge = snapshot
                .elements
                .iter()
                .find(|element| {
                    matches!(
                        element.geometry,
                        Some(AnalyticGeometry::CircularArc { frame, radius_mm, .. })
                            if magnitude(sub(frame.origin_mm, mouth)) <= tolerance
                                && (radius_mm - BORE_RADIUS_MM).abs() <= tolerance
                    )
                })
                .ok_or_else(|| self.fail("Joining plate lost the intended countersink mouth"))?;
            plate = self.checked(plate.treat_edges_named(
                &[TreatmentEdge {
                    key: edge.key,
                    origin: Origin {
                        source: self.feature,
                        role: format!("joining_plate.countersink.{i}"),
                    },
                }],
                EdgeTreatment::Chamfer { distance_mm },
                self.feature,
                policy,
            ))?;
        }
        Ok(plate)
    }

    fn stepped_bores(
        &self,
        stock: &Solid,
        frames: [ReferenceFrame; 2],
    ) -> Result<Solid, ModelError> {
        let mut result = self.checked(stock.transformed(RigidTransform::default()))?;
        let initial_volume = self.checked(result.volume_mm3())?;
        for frame in frames {
            // Start a little outside the actual face while retaining the
            // exact face-relative endpoints of the manufacturer operation.
            let outside = 0.02;
            let start = shift(frame, -outside);
            let counterbore = self.cylinder(
                start,
                PROFILE_ACCESS_RADIUS_MM,
                PROFILE_BARREL_DEPTH_MM + outside,
            )?;
            let pilot = self.cylinder(
                start,
                PROFILE_PILOT_RADIUS_MM,
                PROFILE_PILOT_DEPTH_MM + outside,
            )?;
            let before = self.checked(result.volume_mm3())?;
            result = self.subtract(
                &result,
                &counterbore,
                "Access counterbore removed the member",
            )?;
            if before - self.checked(result.volume_mm3())? < 1.0 {
                return Err(self.fail("The access counterbore missed the profile material. Repair its end or groove orientation."));
            }
            result = self.subtract(&result, &pilot, "Access drill removed the member")?;
        }
        if self.checked(result.solid_count())? != 1
            || self.checked(result.volume_mm3())? >= initial_volume
        {
            return Err(self.fail(
                "Stepped access bores must remove material and preserve one receiving solid",
            ));
        }
        Ok(result)
    }

    fn barrel(&self, access: ReferenceFrame, screw: ReferenceFrame) -> Result<Solid, ModelError> {
        let barrel = self.cylinder(
            shift(access, 0.01),
            PROFILE_ACCESS_RADIUS_MM - 0.02,
            PROFILE_BARREL_DEPTH_MM - 0.02,
        )?;
        let station = shift(screw, -PROFILE_ACCESS_FROM_END_MM);
        let passage = self.cylinder(shift(station, -0.02), 4.1, 12.0)?;
        let drive = self.cylinder(shift(station, -12.0), 7.04, 12.0)?;
        let body = self.subtract(
            &barrel,
            &passage,
            "Universal fastener lost its screw passage",
        )?;
        self.subtract(&body, &drive, "Universal fastener lost its head seat")
    }

    fn screw(
        &self,
        kind: ProfileConnectionKind,
        frame: ReferenceFrame,
    ) -> Result<(Solid, ResolvedThread, ReferenceFrame), ModelError> {
        let universal = kind == ProfileConnectionKind::UniversalEnd;
        let height = 4.4;
        let (head, depth, underhead, runout, neck) = if universal {
            (
                ScrewHead::Button {
                    profile: Box::new(CrownedScrewHeadParameters {
                        diameter: length(14.0),
                        height: length(height),
                        crown_radius: length(7.7),
                        top_land_diameter: length(7.7),
                        neck_radius: length(0.7),
                    }),
                },
                2.6,
                30.0,
                1.95,
                -PROFILE_ACCESS_FROM_END_MM,
            )
        } else {
            (
                ScrewHead::Countersunk {
                    profile: Box::new(CountersunkScrewHeadParameters {
                        diameter: length(16.0),
                        height: length(height),
                        included_angle: model::angle(90.0),
                        edge_chamfer: length(0.05),
                        neck_radius: length(0.4),
                    }),
                },
                3.2,
                14.0 - height,
                1.25,
                -SHEET_MM + height,
            )
        };
        let authored = if universal {
            ScrewLengthInput::Underhead {
                value: length(underhead),
            }
        } else {
            ScrewLengthInput::Overall {
                value: length(14.0),
            }
        };
        let part = MechanicalPartParameters::MachineScrew {
            underhead_length: None,
            length_input: Some(authored),
            shank_diameter: length(8.0),
            head,
            drive: ScrewDrive::HexSocket {
                across_flats: length(5.08),
                depth: length(depth),
            },
            thread: Box::new(ThreadParameters {
                length: length(underhead - runout),
                radial_allowance: length(THREAD_RADIAL_CLEARANCE_MM),
                axial_allowance: length(THREAD_AXIAL_CLEARANCE_MM),
                ..Default::default()
            }),
        };
        let resolved = self.checked(part.resolve(self.inputs.parameters))?;
        let ResolvedMechanicalPart::MachineScrew { thread, .. } = &resolved else {
            unreachable!()
        };
        let thread = thread.as_ref().clone();
        let solid =
            crate::mechanical_parts::build_resolved(self.feature, resolved, self.inputs.policy)?;
        let placed = self.checked(solid.transformed(placement(shift(frame, neck))))?;
        Ok((placed, thread, shift(frame, neck + runout)))
    }

    fn nut(
        &self,
        kind: ProfileConnectionKind,
        variant: ProfileConnectionNut,
        frame: ReferenceFrame,
    ) -> Result<Solid, ModelError> {
        let size = variant.dimensions();
        let half_width = size.width_mm / 2.;
        let seat = shift(frame, ITEM_LINE_8_GROOVE_LIP_DEPTH_MM);
        let section = [
            [-half_width, 0.0],
            [half_width, 0.0],
            [6.5, 3.0],
            [5.4, 5.0],
            [3.3, size.depth_mm],
            [-3.3, size.depth_mm],
            [-5.4, 5.0],
            [-6.5, 3.0],
        ];
        let blank = self.checked(Solid::extrude_curved_profile(
            &CurvedProfile {
                frame: PlaneFrame {
                    origin: sub(seat.origin_mm, scale(seat.u, size.bore_from_end_mm)),
                    u: seat.v,
                    v: seat.normal(),
                },
                outer: polygon(&section),
                holes: vec![],
            },
            size.length_mm,
            self.inputs.policy,
        ))?;
        let (screw, external, external_start) = self.screw(kind, frame)?;
        let params = ThreadParameters {
            length: length(NUT_THREAD_MM),
            radial_allowance: length(THREAD_RADIAL_CLEARANCE_MM),
            axial_allowance: length(THREAD_AXIAL_CLEARANCE_MM),
            minimum_wall: length(0.5),
            ..Default::default()
        };
        let mut internal =
            self.checked(params.resolve(self.inputs.parameters, ThreadMode::CutInternal))?;
        let phase = self.checked(model::straight_thread_phase(
            &external,
            external_start,
            &internal,
            seat,
            self.inputs.policy.linear_tolerance_mm,
        ))?;
        let start = phase.axial_station_mm;
        let engagement = external.length_mm.min(start + internal.length_mm) - start.max(0.0);
        if engagement < 2.0 * external.pitch_mm {
            return Err(self.fail(
                "The M8 hardware needs at least two complete modeled turns inside the T-slot nut",
            ));
        }
        let inner = rotated(seat, phase.roll_rad);
        internal.frame = multicad_core::sketch::Frame {
            origin: inner.origin_mm,
            u: inner.u,
            v: inner.v,
            n: inner.normal(),
        };
        let tapped = crate::threads::build_resolved(
            self.feature,
            internal,
            ThreadMode::CutInternal,
            Some(&blank),
            self.inputs.policy,
        )?;
        let relief = self.cylinder(
            shift(seat, NUT_THREAD_MM - 0.001),
            4.10,
            size.depth_mm - NUT_THREAD_MM + 0.02,
        )?;
        let nut = self.subtract(&tapped, &relief, "T-slot nut crown relief removed the part")?;
        self.check_clear(
            &screw,
            &nut,
            "The supported M8 screw and T-slot nut threads interfere",
        )?;
        Ok(nut)
    }
}

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn build(
    inputs: Inputs<'_>,
    feature: EntityId,
    backend: GeometryBackend,
) -> Result<Solid, ModelError> {
    let builder = Builder { inputs, feature };
    let ctx = &builder.inputs;
    let output = builder.checked(model::read_profile_connection_output(ctx.document, feature))?;
    let request = builder.checked(model::read_profile_connection_request(
        ctx.document,
        output.connection,
    ))?;
    let first = builder.member(&request.first)?;
    let second = builder.member(&request.second)?;
    let frames = builder.checked(model::resolve_profile_connection_frames(
        ctx.document,
        &request,
        [&first, &second],
        ctx.parameters,
        ctx.revision,
        ctx.snapshots,
    ))?;
    if backend == GeometryBackend::Rust {
        return match output.geometry {
            ProfileOutputKind::AccessBores { .. } => Err(builder.fail(
                "Rust profile connections do not yet support stepped access bores across interrupted stock",
            )),
            ProfileOutputKind::Part { index } => match request.kind.roles()[index] {
                ProfilePartRole::Connector(_) => match request.kind {
                    ProfileConnectionKind::JoiningPlate => builder.rust_joining_plate(frames),
                    ProfileConnectionKind::AngleBracket => Err(builder.fail(
                        "Rust profile connections do not yet support countersinks on both angle bracket legs",
                    )),
                    ProfileConnectionKind::UniversalEnd => Err(builder.fail(
                        "Rust profile connections do not yet support the intersecting barrel screw passage and head seat",
                    )),
                },
                ProfilePartRole::Screw(_) => Err(builder.fail(
                    "Rust profile connections do not yet support the required modeled screw head and socket",
                )),
                ProfilePartRole::Nut(_) => Err(builder.fail(
                    "Rust profile connections do not yet support the modeled tap and crown relief of the Line 8 nut",
                )),
            },
        };
    }
    let ProfileOutputKind::Part { index } = output.geometry else {
        let ProfileOutputKind::AccessBores { target } = output.geometry else {
            unreachable!()
        };
        let access = frames
            .access
            .ok_or_else(|| builder.fail("The end connection lost its access bore datums"))?;
        return builder.stepped_bores(builder.solid(target.producer)?, access);
    };
    let solid = match request.kind.roles()[index] {
        ProfilePartRole::Connector(index) => {
            if let Some(access) = frames.access {
                builder.barrel(access[index], frames.fasteners[index])?
            } else {
                builder.plate(request.kind, frames)?
            }
        }
        ProfilePartRole::Screw(index) => builder.screw(request.kind, frames.fasteners[index])?.0,
        ProfilePartRole::Nut(index) => {
            builder.nut(request.kind, request.nut, frames.fasteners[index])?
        }
    };
    for (site, producer) in [
        (
            &request.first,
            output.access_bores.unwrap_or(request.first.member.producer),
        ),
        (&request.second, request.second.member.producer),
    ] {
        let pose = builder.checked(model::resolve_occurrence_pose(
            ctx.document,
            &site.occurrence,
            site.member.body,
            ctx.parameters,
            ctx.revision,
            ctx.snapshots,
        ))?;
        let receiving = builder.checked(builder.solid(producer)?.transformed(RigidTransform {
            rotation: pose.rotation,
            translation_mm: pose.translation_mm,
        }))?;
        builder.check_clear(&solid, &receiving, "Connector hardware interferes with the retained profile section. Check both stations, groove sides and the end orientation.")?;
    }
    if builder.checked(solid.solid_count())? != 1 {
        return Err(builder.fail("Each retained connector component must be one physical solid"));
    }
    Ok(solid)
}
