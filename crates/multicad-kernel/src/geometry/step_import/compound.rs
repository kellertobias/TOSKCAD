//! Keep STEP representation members independent and their topology identities distinct.
use super::*;
use std::collections::BTreeMap;

fn record(tag: u64, coordinates: impl IntoIterator<Item = f64>) -> [u64; 11] {
    let mut record = [0; 11];
    record[0] = tag;
    for (slot, value) in record[1..].iter_mut().zip(coordinates) {
        *slot = if value == 0. { 0 } else { value.to_bits() };
    }
    record
}

fn unsigned_normal(normal: Point) -> Point {
    if normal
        .into_iter()
        .find(|&value| value != 0.)
        .is_some_and(|value| value < 0.)
    {
        normal.map(|value| -value)
    } else {
        normal
    }
}

// Use world geometry instead of STEP record numbers or the order of other
// members. An update must not bind a face to a different shell just because
// its shell was moved to the same list position.
fn member_signature(readback: &Readback) -> ContentHash {
    let lines = readback.edges_mm.iter().map(|&points| {
        let mut points = points;
        points.sort_unstable_by(compare_points);
        record(1, points.into_iter().flatten())
    });
    let circles = readback.circles.iter().map(|rim| {
        record(
            2,
            rim.center_mm
                .into_iter()
                .chain(unsigned_normal(rim.normal))
                .chain([rim.radius_mm]),
        )
    });
    let halves = readback.semicircles.iter().map(|rim| {
        record(
            3,
            rim.center_mm
                .into_iter()
                .chain(unsigned_normal(rim.normal))
                .chain([rim.radius_mm])
                .chain(rim.radial_mid),
        )
    });
    let arcs = readback.arcs.iter().map(|rim| {
        let mut ends = [rim.radial_start, rim.radial_end];
        ends.sort_unstable_by(compare_points);
        let radius = if rim.sweep_rad > PI {
            -rim.radius_mm
        } else {
            rim.radius_mm
        };
        record(
            4,
            rim.center_mm
                .into_iter()
                .chain(ends.into_iter().flatten())
                .chain([radius]),
        )
    });
    let mut edges: Vec<_> = lines.chain(circles).chain(halves).chain(arcs).collect();
    edges.sort_unstable();
    let mut bytes = Vec::with_capacity(64 + edges.len() * 11 * 8);
    bytes.extend((readback.face_count as u64).to_le_bytes());
    bytes.extend(readback.volume_mm3.to_le_bytes());
    for value in readback.bounds_mm.into_iter().flatten() {
        bytes.extend(if value == 0. { 0. } else { value }.to_le_bytes());
    }
    for entry in edges.into_iter().flatten() {
        bytes.extend(entry.to_le_bytes());
    }
    let mut ellipses = readback
        .ellipses
        .iter()
        .map(|arc| {
            let geometry = arc.ellipse();
            let mut ends = arc.endpoints();
            ends.sort_unstable_by(compare_points);
            ends.into_iter()
                .flatten()
                .chain(geometry.center_mm())
                .chain(unsigned_normal(geometry.major_axis()))
                .chain(geometry.semiaxes_mm())
                .chain(geometry.point_at(arc.start_rad() + arc.sweep_rad() * 0.5))
                .chain([arc.sweep_rad().abs()])
                .map(|value| if value == 0. { 0 } else { value.to_bits() })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    ellipses.sort_unstable();
    for entry in ellipses.into_iter().flatten() {
        bytes.extend(entry.to_le_bytes());
    }
    ContentHash::of(&bytes)
}

pub(super) fn rebuild(
    members: Vec<(Readback, ImportSolid)>,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    let mut occurrences = BTreeMap::<ContentHash, u32>::new();
    let mut solids = Vec::with_capacity(members.len());
    for (readback, imported) in members {
        let signature = member_signature(&readback);
        let occurrence = occurrences.entry(signature).or_default();
        let mut bytes = b"multicad.step-member/1".to_vec();
        bytes.extend(feature.uuid_bits().to_le_bytes());
        bytes.extend(signature.0);
        bytes.extend(occurrence.to_le_bytes());
        *occurrence += 1;
        let hash = ContentHash::of(&bytes);
        let child = EntityId::from_uuid_bits(u128::from_le_bytes(hash.0[..16].try_into().unwrap()));
        solids.push(Solid::verified_step_member(
            &readback, imported, child, policy,
        )?);
    }
    let members: Vec<_> = solids.iter().collect();
    Solid::make_rust_compound(&members, feature, policy, None)
}

impl Solid {
    /// Check each emitted STEP shell against the same member's exact face and
    /// edge ordinals in the published compound. The writer preserves member order.
    pub fn step_compound_topology_matches(
        topology: &TopologySnapshot,
        readbacks: &[Readback],
        tolerance: f64,
    ) -> bool {
        if !tolerance.is_finite() || tolerance <= 0. || readbacks.is_empty() {
            return false;
        }
        if let [readback] = readbacks {
            return readback.solid_count == 1
                && Self::step_topology_matches(topology, readback, tolerance);
        }
        let mut ends = Vec::with_capacity(readbacks.len());
        let mut end = [0usize; 2];
        for readback in readbacks {
            if readback.solid_count != 1 || readback.face_count == 0 {
                return false;
            }
            end[0] += readback.face_count;
            end[1] += readback.edges_mm.len()
                + readback.circles.len()
                + readback.semicircles.len()
                + readback.arcs.len()
                + readback.ellipses.len();
            ends.push(end);
        }
        let mut members = vec![Vec::new(); readbacks.len()];
        for element in &topology.elements {
            let kind = match element.key.kind {
                TopologyKind::Face => 0,
                TopologyKind::Edge => 1,
                TopologyKind::Vertex => continue,
            };
            let index = ends.partition_point(|end| end[kind] <= element.key.ordinal as usize);
            let Some(member) = members.get_mut(index) else {
                return false;
            };
            member.push(element);
        }
        members
            .into_iter()
            .zip(readbacks)
            .all(|(elements, readback)| compare_topology(elements, readback, tolerance, false))
    }
}
