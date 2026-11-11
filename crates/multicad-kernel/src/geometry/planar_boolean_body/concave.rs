//! Exact material for translated single-wire profiles in their authored chart.
use super::*;

impl Solid {
    pub(super) fn simple_concave_parts(
        &self,
        topology: &TopologySnapshot,
        policy: GeometryPolicy,
    ) -> Option<Result<Vec<ConvexPart>, GeometryError>> {
        if self.polygon_prism().is_none() && self.linear_body().is_none() {
            return None;
        }
        let selected = (|| {
            let (local, placement) = self.local_box_treatment_source()?.map_or_else(
                || self.share().map(|solid| (solid, RigidTransform::default())),
                Ok,
            )?;
            let source = local.rust_topology().ok_or_else(planar_unsupported)??;
            let Some((rings, ancestry)) = convex_faces(&local, &source, policy) else {
                return Ok(None);
            };
            let tolerance = policy.linear_tolerance_mm;
            let nonconvex = rings.iter().zip(&ancestry).any(|(ring, &index)| {
                let Some(AnalyticGeometry::Plane { frame }) = source.elements[index].geometry
                else {
                    unreachable!("planar source supports")
                };
                rings
                    .iter()
                    .flatten()
                    .any(|&point| dot(frame.normal(), subtract(point, ring[0])) > tolerance)
            });
            if !nonconvex {
                return Ok(None);
            }
            // Selecting this provider commits to its source and material proofs.
            // A failure cannot become an unrelated provider's Unsupported result.
            let boundary = Union::from_planar_rings(&rings, tolerance).map_err(planar_error)?;
            if !same_boundary(&boundary, &source) {
                return Err(planar_error(Error::Invalid));
            }
            let volume = local.volume_mm3()?;
            if (boundary.volume_mm3() - volume).abs() > volume * 1e-8 {
                return Err(planar_error(Error::Invalid));
            }
            let Material::LabeledCells {
                pieces,
                exterior_faces,
            } = scale::concave::straight_profile_material(&rings, tolerance)?
            else {
                unreachable!("translated profile cells")
            };
            let world_faces: BTreeMap<_, _> = topology
                .elements
                .iter()
                .enumerate()
                .filter(|(_, e)| e.key.kind == TopologyKind::Face)
                .map(|(i, e)| (e.key.ordinal, i))
                .collect();
            let labels = exterior_faces
                .into_iter()
                .map(|faces| {
                    faces
                        .into_iter()
                        .map(|face| {
                            face.map(|face| {
                                world_faces
                                    .get(&source.elements[ancestry[face]].key.ordinal)
                                    .copied()
                                    .ok_or_else(|| planar_error(Error::Invalid))
                            })
                            .transpose()
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let frame = placement.frame(PlaneFrame::default());
            let pieces = PlanarBooleanBody::placed_pieces(&pieces, frame, tolerance)?;
            Ok(Some(pieces.into_iter().zip(labels).collect()))
        })();
        match selected {
            Ok(None) => None,
            Ok(Some(parts)) => Some(Ok(parts)),
            Err(cause) => Some(Err(cause)),
        }
    }
}

fn same_boundary(boundary: &Union, topology: &TopologySnapshot) -> bool {
    let key = |point: Point| point.map(|value| if value == 0. { 0 } else { value.to_bits() });
    let edge = |a: Point, b: Point| {
        let mut ends = [key(a), key(b)];
        ends.sort_unstable();
        ends
    };
    let source_vertices: BTreeSet<_> = topology
        .elements
        .iter()
        .filter_map(|element| match element.geometry {
            Some(AnalyticGeometry::Point { position_mm }) => Some(key(position_mm)),
            _ => None,
        })
        .collect();
    let source_edges: BTreeSet<_> = topology
        .elements
        .iter()
        .filter_map(|element| match element.geometry {
            Some(AnalyticGeometry::Line { a_mm, b_mm }) => Some(edge(a_mm, b_mm)),
            _ => None,
        })
        .collect();
    let vertices: BTreeSet<_> = boundary.vertices().iter().copied().map(key).collect();
    let edges: BTreeSet<_> = boundary
        .faces()
        .iter()
        .flat_map(|face| {
            face.vertices
                .iter()
                .zip(face.vertices.iter().cycle().skip(1))
                .map(|(&a, &b)| edge(boundary.vertices()[a], boundary.vertices()[b]))
        })
        .collect();
    vertices == source_vertices && edges == source_edges
}
