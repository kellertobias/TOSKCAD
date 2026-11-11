//! Exact end planes, recess and shoulder topology for a headed screw.

use super::{
    boundary::{Boundary, edge_midpoint, hex_wall_plane, placed_frame},
    *,
};
use multicad_rust_kernel::headed_screw::{ScrewFace, ScrewHead, ScrewSpec};

#[derive(Clone, Copy)]
enum MountingFrame {
    Underhead(PlaneFrame),
    Thread(PlaneFrame),
}

impl Solid {
    /// Build a screw in its canonical underhead mounting frame. The thread,
    /// optional plain shank and recessed or hex head share one boundary.
    pub fn rust_machine_screw(
        spec: HelicalThread,
        root_radius_mm: Option<f64>,
        screw: ScrewSpec,
        feature: EntityId,
        underhead: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::machine_screw_in_frame(
            spec,
            root_radius_mm,
            screw,
            feature,
            MountingFrame::Underhead(underhead),
            policy,
        )
    }

    pub(super) fn rust_machine_screw_at_thread_frame(
        spec: HelicalThread,
        root_radius_mm: Option<f64>,
        screw: ScrewSpec,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::machine_screw_in_frame(
            spec,
            root_radius_mm,
            screw,
            feature,
            MountingFrame::Thread(frame),
            policy,
        )
    }

    fn machine_screw_in_frame(
        spec: HelicalThread,
        root_radius_mm: Option<f64>,
        screw: ScrewSpec,
        feature: EntityId,
        mounting: MountingFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let (MountingFrame::Underhead(underhead) | MountingFrame::Thread(underhead)) = mounting;
        policy.validate()?;
        underhead.checked(policy)?;
        policy.length(screw.shank_radius_mm)?;
        if screw.shank_length_mm != 0. {
            policy.length(screw.shank_length_mm)?;
        }
        policy.length(screw.head.height_mm())?;
        let head_minimum = match screw.head {
            ScrewHead::SocketCap {
                radius_mm,
                height_mm,
                socket_across_flats_mm,
                socket_depth_mm,
            } => {
                policy.length(radius_mm)?;
                policy.length(socket_across_flats_mm)?;
                policy.length(socket_depth_mm)?;
                policy.length(height_mm - socket_depth_mm)?;
                policy.length(radius_mm - socket_across_flats_mm / 3_f64.sqrt())?;
                radius_mm
            }
            ScrewHead::ExternalHex {
                across_flats_mm, ..
            } => across_flats_mm / 2.,
        };
        policy.length(head_minimum - screw.shank_radius_mm.max(spec.major_radius_mm))?;
        if screw.shank_length_mm > 0. && screw.shank_radius_mm < spec.major_radius_mm {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "The plain shank radius must cover the full thread crest",
            ));
        }
        let frame = match mounting {
            MountingFrame::Thread(frame) => frame,
            MountingFrame::Underhead(underhead) => PlaneFrame {
                origin: position(underhead, [0., 0., screw.shank_length_mm]),
                ..underhead
            },
        };
        Self::rust_helical_body(spec, root_radius_mm, feature, frame, policy, |thread| {
            Screw::new(thread, screw)
                .map(HelicalForm::Screw)
                .map_err(|cause| match cause {
                    ThreadError::Invalid => error(
                        OPERATION,
                        GeometryErrorKind::Input,
                        "Invalid analytic machine screw head, drive or shank",
                    ),
                    ThreadError::Limit => error(
                        OPERATION,
                        GeometryErrorKind::Limit,
                        "Machine screw exceeds the geometry envelope",
                    ),
                })
        })
    }
}

fn polygon_area(across_flats: f64) -> f64 {
    3_f64.sqrt() * across_flats.powi(2) / 2.
}

fn circle_area(radius: f64) -> f64 {
    PI * radius.powi(2)
}

pub(super) fn extend_face_properties(screw: &Screw, faces: &mut Vec<(Point, f64)>) {
    let spec = screw.spec();
    let outer = spec.head.cross_section_area_mm2();
    let shank = circle_area(spec.shank_radius_mm);
    let seat = screw.head_seat_z();
    let top = screw.head_top_z();
    let height = spec.head.height_mm();
    let [socket_af, socket_depth] = spec.head.socket().map_or([0., 0.], |(a, d)| [a, d]);
    let socket_area = polygon_area(socket_af);
    let (removed_center, removed_area) = faces[0];
    let shoulder = if spec.shank_length_mm > 0. {
        shank
    } else {
        outer
    } - removed_area;
    faces[0] = (
        removed_center.map(|coordinate| -coordinate * removed_area / shoulder),
        shoulder,
    );
    for face in screw.faces().into_iter().skip(faces.len()) {
        faces.push(match face {
            ScrewFace::ShankWall => (
                [0., 0., seat / 2.],
                TAU * spec.shank_radius_mm * spec.shank_length_mm,
            ),
            ScrewFace::HeadSeat => ([0., 0., seat], outer - shank),
            ScrewFace::HeadTop => ([0., 0., top], outer - socket_area),
            ScrewFace::HeadWall(i) => match spec.head {
                ScrewHead::SocketCap { radius_mm, .. } => {
                    ([0., 0., top.midpoint(seat)], TAU * radius_mm * height)
                }
                ScrewHead::ExternalHex {
                    across_flats_mm, ..
                } => (
                    edge_midpoint(across_flats_mm, i, top.midpoint(seat)),
                    across_flats_mm * height / 3_f64.sqrt(),
                ),
            },
            ScrewFace::SocketFloor => ([0., 0., top + socket_depth], socket_area),
            ScrewFace::SocketWall(i) => (
                edge_midpoint(socket_af, i, top + socket_depth / 2.),
                socket_af * socket_depth / 3_f64.sqrt(),
            ),
            _ => unreachable!("Thread face properties were already constructed"),
        });
    }
}

pub(super) fn plane(screw: &Screw, role: ScrewFace, frame: PlaneFrame) -> Option<ReferenceFrame> {
    let seat = screw.head_seat_z();
    let top = screw.head_top_z();
    let axial = |z, sign| placed_frame(frame, [0., 0., z], [1., 0., 0.], [0., sign, 0.]);
    match role {
        ScrewFace::HeadSeat => Some(axial(seat, 1.)),
        ScrewFace::HeadTop => Some(axial(top, -1.)),
        ScrewFace::HeadWall(i) => match screw.spec().head {
            ScrewHead::ExternalHex {
                across_flats_mm, ..
            } => Some(hex_wall_plane(frame, across_flats_mm, i, top, 1.)),
            ScrewHead::SocketCap { .. } => None,
        },
        ScrewFace::SocketFloor => screw
            .spec()
            .head
            .socket()
            .map(|(_, depth)| axial(top + depth, -1.)),
        ScrewFace::SocketWall(i) => screw
            .spec()
            .head
            .socket()
            .map(|(af, _)| hex_wall_plane(frame, af, i, top, -1.)),
        _ => None,
    }
}

fn face_name(role: ScrewFace) -> String {
    match role {
        ScrewFace::ShankWall => "screw.shank/edge/0".into(),
        ScrewFace::HeadSeat => "screw.head/end".into(),
        ScrewFace::HeadTop => "screw.head/start".into(),
        ScrewFace::HeadWall(i) => format!("screw.head/edge/{i}"),
        ScrewFace::SocketFloor => "screw.socket/end".into(),
        ScrewFace::SocketWall(i) => format!("screw.socket/edge/{i}"),
        _ => unreachable!("Thread face identities already exist"),
    }
}

pub(super) fn extend_topology(
    body: &HelicalBody,
    screw: &Screw,
    frame: PlaneFrame,
    elements: &mut Vec<TopologyElement>,
) {
    let roles = screw.faces();
    let wall_count = body.rails.len();
    for (ordinal, &role) in roles.iter().enumerate().skip(2 + wall_count) {
        let origin = body.origin(face_name(role));
        let plane = plane(screw, role, frame);
        elements.push(TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: ordinal as u32,
            },
            origins: BTreeSet::from([origin.clone()]),
            boundary: BTreeSet::new(),
            frames: plane
                .into_iter()
                .map(|frame| AnchoredFrame {
                    origin: origin.clone(),
                    frame,
                })
                .collect(),
            geometry: plane.map(|frame| AnalyticGeometry::Plane { frame }),
        });
    }
    let mut add = Boundary::new(body, frame, &roles, elements, false);
    let spec = screw.spec();
    let seat = screw.head_seat_z();
    let top = screw.head_top_z();
    let seat_role = if spec.shank_length_mm > 0. {
        let shoulder = ScrewFace::ThreadShoulder;
        let shank = ScrewFace::ShankWall;
        let radius = spec.shank_radius_mm;
        if radius == screw.thread().spec().major_radius_mm {
            let rate = screw.thread().helical_rate();
            for i in 0..wall_count {
                if (body.band(i) - 2) % 4 == 1 {
                    let edge = body.rim_origin(0, i);
                    add.face(shoulder).boundary.remove(&edge);
                    add.face(shank).boundary.insert(edge);
                } else {
                    let phase = body.rails[i];
                    add.arc(
                        format!("screw.shank/rim/end/{}", body.rail_name(i)),
                        0.,
                        radius,
                        [-phase / rate, (phase - body.next_phase(i)) / rate],
                        &[shoulder, shank],
                    );
                }
            }
        } else {
            add.arc(
                "screw.shank/rim/end".into(),
                0.,
                radius,
                [0., TAU],
                &[shoulder, shank],
            );
        }
        add.arc(
            "screw.shank/rim/start".into(),
            seat,
            radius,
            [0., TAU],
            &[ScrewFace::HeadSeat, shank],
        );
        ScrewFace::HeadSeat
    } else {
        let origin = body.origin(face_name(ScrewFace::HeadSeat));
        let cap = body.cap_frame(frame, 0);
        let face = add.face(ScrewFace::ThreadShoulder);
        face.origins.insert(origin.clone());
        face.frames.push(AnchoredFrame { origin, frame: cap });
        ScrewFace::ThreadShoulder
    };
    let top_role = ScrewFace::HeadTop;
    match spec.head {
        ScrewHead::SocketCap {
            radius_mm,
            socket_across_flats_mm,
            socket_depth_mm,
            ..
        } => {
            for (name, z, cap) in [("end", seat, seat_role), ("start", top, top_role)] {
                add.arc(
                    format!("screw.head/rim/{name}"),
                    z,
                    radius_mm,
                    [0., TAU],
                    &[cap, ScrewFace::HeadWall(0)],
                );
            }
            add.hex_prism(
                "screw.socket",
                socket_across_flats_mm,
                [top, top + socket_depth_mm],
                [top_role, ScrewFace::SocketFloor],
                ScrewFace::SocketWall,
            );
        }
        ScrewHead::ExternalHex {
            across_flats_mm, ..
        } => {
            add.hex_prism(
                "screw.head",
                across_flats_mm,
                [top, seat],
                [top_role, seat_role],
                ScrewFace::HeadWall,
            );
        }
    }
}
