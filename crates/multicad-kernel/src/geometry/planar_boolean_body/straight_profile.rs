//! Material partitions retain the logical face labels of hollow linear stock.
use super::*;
use multicad_rust_kernel::planar_boolean::{ProfileFace, straight_profile};

impl Solid {
    pub(super) fn hollow_straight_parts(
        &self,
        topology: &TopologySnapshot,
        policy: GeometryPolicy,
    ) -> Option<Result<Vec<ConvexPart>, GeometryError>> {
        let (body, frame) = self.linear_body()?;
        let (rings, height) = body.material_partition_section()?;
        if rings.len() < 2 {
            return None;
        }
        Some((|| {
            let cells = straight_profile(rings, height, policy.linear_tolerance_mm, None)
                .map_err(planar_error)?;
            let faces: BTreeMap<_, _> = topology
                .elements
                .iter()
                .enumerate()
                .filter(|(_, e)| e.key.kind == TopologyKind::Face)
                .map(|(i, e)| (e.key.ordinal as usize, i))
                .collect();
            let labels = cells
                .iter()
                .map(|cell| {
                    cell.source_faces
                        .iter()
                        .map(|label| {
                            let ordinal = match label {
                                ProfileFace::StartCap => 0,
                                ProfileFace::EndCap => 1,
                                ProfileFace::Edge(i) => 2 + i,
                                ProfileFace::Partition => return Ok(None),
                            };
                            faces
                                .get(&ordinal)
                                .copied()
                                .map(Some)
                                .ok_or_else(|| planar_error(Error::Invalid))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let pieces: Vec<_> = cells
                .into_iter()
                .map(|c| {
                    if !body.has_plane_terminations() {
                        return Ok(c.piece);
                    }
                    // The source partition has only its two cap levels. Each
                    // generator maps to the actual, already validated pair of
                    // termination planes; no display geometry is involved.
                    c.piece
                        .transformed(
                            |[x, y, z]| body.material_partition_point(z == height, [x, y]),
                            policy.linear_tolerance_mm,
                        )
                        .map_err(planar_error)
                })
                .collect::<Result<_, _>>()?;
            let pieces =
                PlanarBooleanBody::placed_pieces(&pieces, frame, policy.linear_tolerance_mm)?;
            Ok(pieces.into_iter().zip(labels).collect())
        })())
    }
}
