use super::{
    ProfileAttachment, ProfileConnectionKind, ProfileConnectionRequest, ProfileConnectorAnchor,
};
use crate::{
    document::{Document, DocumentError, EntityId},
    model::{
        ITEM_LINE_8_WIDTH_MM, ResolvedMember, resolve_occurrence_pose,
        source::invalid,
        weldment::vec3::{add, cross, distance, dot, scale, sub},
    },
    parameters::EvaluatedParameters,
    references::{ReferenceFrame, TopologySnapshot},
};
use std::collections::BTreeMap;

const MOUNT_TOLERANCE_MM: f64 = 0.001;
const ALIGNMENT_TOLERANCE: f64 = 1e-8;
const BRACKET_STATION_MM: f64 = 20.0;
const PLATE_PITCH_MM: f64 = 40.0;
pub const PROFILE_ACCESS_FROM_END_MM: f64 = 20.0;
pub const PROFILE_BARREL_DEPTH_MM: f64 = 16.0;
pub const PROFILE_ACCESS_RADIUS_MM: f64 = 10.0;
pub const PROFILE_PILOT_DEPTH_MM: f64 = 33.0;
pub const PROFILE_PILOT_RADIUS_MM: f64 = 3.5;
const NUT_LONG_SIDE_MM: f64 = 15.5;

/// Fastener frames point into the receiving groove. Access frames start at
/// the original exterior face and point inward along the radial bore.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileConnectionFrames {
    pub sites: [ReferenceFrame; 2],
    pub fasteners: [ReferenceFrame; 2],
    pub access: Option<[ReferenceFrame; 2]>,
}

fn site(
    attachment: &ProfileAttachment,
    member: &ResolvedMember,
    parameters: &EvaluatedParameters,
    footprint: f64,
) -> Result<ReferenceFrame, DocumentError> {
    match &attachment.anchor {
        ProfileConnectorAnchor::Groove { side, .. } => {
            let station = attachment
                .groove_station(parameters)?
                .ok_or_else(|| invalid("A groove needs a station"))?;
            if station < footprint - MOUNT_TOLERANCE_MM
                || member.length_mm() - station < footprint - MOUNT_TOLERANCE_MM
            {
                return Err(invalid(format!(
                    "Move the groove station at least {footprint} mm from both member ends to support the full fastening footprint"
                )));
            }
            member.groove_frame(*side, station)
        }
        ProfileConnectorAnchor::End { end } => member.slotted_end_frame(*end),
    }
}

fn inward(surface: ReferenceFrame) -> ReferenceFrame {
    ReferenceFrame {
        v: scale(surface.v, -1.0),
        ..surface
    }
}

pub fn resolve_profile_connection_frames(
    document: &Document,
    request: &ProfileConnectionRequest,
    members: [&ResolvedMember; 2],
    parameters: &EvaluatedParameters,
    revision: u64,
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<ProfileConnectionFrames, DocumentError> {
    request.validate()?;
    let half_width = ITEM_LINE_8_WIDTH_MM / 2.0;
    let universal_offset = half_width - PROFILE_BARREL_DEPTH_MM / 2.0;
    let footprint = if request.kind == ProfileConnectionKind::UniversalEnd {
        universal_offset + NUT_LONG_SIDE_MM
    } else {
        BRACKET_STATION_MM
    };
    let resolve = |attachment: &ProfileAttachment, member: &ResolvedMember| {
        attachment.validate_member(document)?;
        let local = site(attachment, member, parameters, footprint)?;
        let pose = resolve_occurrence_pose(
            document,
            &attachment.occurrence,
            attachment.member.body,
            parameters,
            revision,
            snapshots,
        )?;
        Ok::<_, DocumentError>(ReferenceFrame {
            origin_mm: pose.point(local.origin_mm),
            u: pose.vector(local.u),
            v: pose.vector(local.v),
        })
    };
    let first = resolve(&request.first, members[0])?;
    let second = resolve(&request.second, members[1])?;
    let normals = [first.normal(), second.normal()];
    let between = sub(second.origin_mm, first.origin_mm);
    let aligned = |a, b| dot(a, b) > 1.0 - ALIGNMENT_TOLERANCE;
    let parallel = |a, b| dot(a, b).abs() > 1.0 - ALIGNMENT_TOLERANCE;
    let perpendicular = |a, b| dot(a, b).abs() < ALIGNMENT_TOLERANCE;
    let close = |a, b| distance(a, b) <= MOUNT_TOLERANCE_MM;

    let (fasteners, access) = match request.kind {
        ProfileConnectionKind::AngleBracket => {
            if !perpendicular(normals[0], normals[1])
                || !parallel(first.u, normals[1])
                || !parallel(second.u, normals[0])
                || !close(
                    between,
                    scale(sub(normals[0], normals[1]), BRACKET_STATION_MM),
                )
            {
                return Err(invalid(
                    "The 40 mm angle needs perpendicular, inward-facing groove surfaces with both screw axes 20 mm from their shared corner. Adjust the member poses or groove stations.",
                ));
            }
            ([inward(first), inward(second)], None)
        }
        ProfileConnectionKind::JoiningPlate => {
            if !aligned(normals[0], normals[1])
                || !parallel(first.u, second.u)
                || dot(between, normals[0]).abs() > MOUNT_TOLERANCE_MM
                || dot(between, first.u).abs() > MOUNT_TOLERANCE_MM
                || (distance(between, [0.; 3]) - PLATE_PITCH_MM).abs() > MOUNT_TOLERANCE_MM
            {
                return Err(invalid(
                    "The flat plate needs coplanar parallel grooves 40 mm apart at the same longitudinal station. Adjust both groove stations or the member poses.",
                ));
            }
            ([inward(first), inward(second)], None)
        }
        ProfileConnectionKind::UniversalEnd => {
            if !aligned(normals[0], scale(normals[1], -1.0))
                || !close(first.origin_mm, second.origin_mm)
                || !(parallel(first.u, second.u) || parallel(first.v, second.u))
            {
                return Err(invalid(
                    "Align the member end face against the receiving groove. Its bore center must meet the selected station and one pair of host grooves must align with the receiving groove direction.",
                ));
            }
            if members[0].length_mm()
                < PROFILE_ACCESS_FROM_END_MM + PROFILE_ACCESS_RADIUS_MM + MOUNT_TOLERANCE_MM
            {
                return Err(invalid(
                    "The receiving end member is too short for the 20 mm access bores",
                ));
            }
            let at_end = sub(
                first.origin_mm,
                scale(normals[0], PROFILE_ACCESS_FROM_END_MM),
            );
            let fasteners = [-1.0, 1.0].map(|sign| ReferenceFrame {
                origin_mm: add(second.origin_mm, scale(second.u, sign * universal_offset)),
                ..inward(second)
            });
            let access = [-1.0, 1.0].map(|sign| {
                let side = scale(second.u, sign);
                ReferenceFrame {
                    origin_mm: add(at_end, scale(side, half_width)),
                    u: normals[0],
                    v: cross(scale(side, -1.0), normals[0]),
                }
            });
            (fasteners, Some(access))
        }
    };
    Ok(ProfileConnectionFrames {
        sites: [first, second],
        fasteners,
        access,
    })
}
