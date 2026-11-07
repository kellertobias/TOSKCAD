use crate::viewport::Scene;
use multicad_core::{
    document::EntityId,
    model::{
        self, ITEM_LINE_8_GROOVE_DEPTH_MM, ITEM_LINE_8_WIDTH_MM, MemberEnd, ProfileAttachment,
        ProfileConnectionRequest, ProfileConnectorAnchor, ProfileGrooveSide, ProfileStation,
        ProfileStationDatum,
    },
    parameters::EvaluatedParameters,
    references::{
        AnalyticGeometry, ReferenceFrame, TopologyElement, TopologyKind, TopologySnapshot,
    },
};

fn planar_faces(snapshot: &TopologySnapshot) -> impl Iterator<Item = &TopologyElement> {
    snapshot.elements.iter().filter(|element| {
        element.key.kind == TopologyKind::Face
            && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
    })
}

pub(crate) fn profile_cap_frame(
    snapshot: &TopologySnapshot,
    owner: EntityId,
    end: MemberEnd,
) -> Option<ReferenceFrame> {
    let role = match end {
        MemberEnd::Start => "cap.start",
        MemberEnd::End => "cap.end",
    };
    let mut frames = planar_faces(snapshot)
        .flat_map(|element| &element.frames)
        .filter(|frame| frame.origin.source == owner && frame.origin.role == role);
    let frame = frames.next()?.frame;
    frames.next().is_none().then_some(frame)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}

fn datum(scene: &Scene, attachment: &ProfileAttachment) -> Result<ProfileStationDatum, String> {
    let leaf = scene
        .assembly_evaluation()
        .and_then(|info| {
            info.leaves.iter().find(|leaf| {
                leaf.path == attachment.occurrence && leaf.component == attachment.member.body
            })
        })
        .ok_or("The selected member occurrence no longer matches the current native model")?;
    // An end connection can own a later access-bore producer on the same Part.
    // Its mating datum still belongs to the uncut member source in that occurrence.
    let owner = attachment.member.producer;
    let snapshot = scene
        .geometry_snapshot(owner)
        .ok_or("The selected member has no current native source geometry")?;
    let start = profile_cap_frame(&snapshot, owner, MemberEnd::Start)
        .ok_or("The native member start face needs repair")?;
    let end = profile_cap_frame(&snapshot, owner, MemberEnd::End)
        .ok_or("The native member end face needs repair")?;
    let ends = std::array::from_fn(|i| end.origin_mm[i] - start.origin_mm[i]);
    let axis = start.normal();
    let signed_length = dot(ends, axis);
    if !signed_length.is_finite()
        || signed_length.abs() <= 1e-6
        || dot(end.normal(), axis).abs() < 1. - 1e-8
    {
        return Err("The selected member needs two unambiguous parallel end faces".into());
    }
    let length_mm = signed_length.abs();
    let axis = axis.map(|value| value * signed_length.signum());
    let half = ITEM_LINE_8_WIDTH_MM / 2.;
    let center = |frame: ReferenceFrame| {
        std::array::from_fn(|i| frame.origin_mm[i] + half * (frame.u[i] + frame.v[i]))
    };
    let frame = match &attachment.anchor {
        ProfileConnectorAnchor::End { end: which } => {
            let (cap, sign) = match which {
                MemberEnd::Start => (start, -1.),
                MemberEnd::End => (end, 1.),
            };
            ReferenceFrame {
                origin_mm: center(cap),
                u: cap.u,
                v: cross(axis.map(|value| value * sign), cap.u),
            }
        }
        ProfileConnectorAnchor::Groove { side, .. } => {
            let n = match side {
                ProfileGrooveSide::MinX => start.u.map(|value| -value),
                ProfileGrooveSide::MaxX => start.u,
                ProfileGrooveSide::MinY => start.v.map(|value| -value),
                ProfileGrooveSide::MaxY => start.v,
            };
            let role = format!("member.groove.{}.floor", side.token());
            let mut floors = planar_faces(&snapshot)
                .filter(|face| {
                    face.origins
                        .iter()
                        .any(|origin| origin.source == owner && origin.role == role)
                })
                .filter_map(|face| match face.geometry {
                    Some(AnalyticGeometry::Plane { frame }) => Some(frame),
                    _ => None,
                });
            let floor = floors
                .next()
                .ok_or("The selected groove has no unambiguous native floor")?;
            let center = center(start);
            let to_floor = std::array::from_fn(|i| floor.origin_mm[i] - center[i]);
            if floors.next().is_some()
                || dot(floor.normal(), n).abs() < 1. - 1e-8
                || (dot(to_floor, n) - (half - ITEM_LINE_8_GROOVE_DEPTH_MM)).abs() > 1e-5
            {
                return Err("The selected groove floor no longer matches its member datum".into());
            }
            ReferenceFrame {
                origin_mm: std::array::from_fn(|i| center[i] + half * n[i]),
                u: axis,
                v: cross(n, axis),
            }
        }
    };
    Ok(ProfileStationDatum {
        frame: ReferenceFrame {
            origin_mm: leaf.pose.point(frame.origin_mm),
            u: leaf.pose.vector(frame.u),
            v: leaf.pose.vector(frame.v),
        },
        length_mm,
    })
}

pub(super) fn stations(
    scene: &Scene,
    request: &ProfileConnectionRequest,
    parameters: &EvaluatedParameters,
) -> Result<[Option<f64>; 2], String> {
    let station = |attachment: &ProfileAttachment| -> Result<ProfileStation, String> {
        let ProfileConnectorAnchor::Groove { station, .. } = &attachment.anchor else {
            return Ok(ProfileStation::End);
        };
        let value = station.value_in(
            multicad_core::parameters::Unit::Millimeter,
            Some(parameters),
        )?;
        Ok(if station.expression().is_some() {
            ProfileStation::Bound(value)
        } else {
            ProfileStation::Literal(value)
        })
    };
    model::snap_profile_connection_stations(
        request.kind,
        [
            datum(scene, &request.first)?,
            datum(scene, &request.second)?,
        ],
        [station(&request.first)?, station(&request.second)?],
    )
    .map_err(|error| error.to_string())
}
