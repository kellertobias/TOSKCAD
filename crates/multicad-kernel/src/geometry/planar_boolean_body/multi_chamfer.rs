//! Simultaneous nonintersecting bevels of full-length parallel box edges.
use super::*;
use crate::TreatmentEdge;
use multicad_core::references::{TopologyElement, TopologyKey};

fn unsupported() -> GeometryError {
    error(
        "chamfer",
        GeometryErrorKind::Unsupported,
        "Rust multi-edge chamfer requires complete parallel edges of one rectangular solid",
    )
}

impl Solid {
    pub(crate) fn rust_chamfer_parallel_box_edges(
        &self,
        edges: &[TreatmentEdge],
        distance: f64,
        feature: EntityId,
        policy: GeometryPolicy,
        mut topology: TopologySnapshot,
    ) -> Result<Self, GeometryError> {
        if self.policy != policy || policy.boolean_fuzzy_mm != 0. || !(2..=4).contains(&edges.len())
        {
            return Err(unsupported());
        }
        if let Some((source, placement)) = self.local_box_treatment_source()? {
            let local = source.topology_snapshot(topology.owner, topology.revision)?;
            return source
                .rust_chamfer_parallel_box_edges(edges, distance, feature, policy, local)?
                .transformed(placement);
        }
        let cells = self.orthogonal_cells()?.ok_or_else(unsupported)?;
        let [stock] = cells.as_slice() else {
            return Err(unsupported());
        };
        let lines: Vec<_> = edges
            .iter()
            .map(|edge| {
                topology
                    .elements
                    .iter()
                    .find_map(|element| match element.geometry {
                        Some(AnalyticGeometry::Line { a_mm, b_mm }) if element.key == edge.key => {
                            Some((a_mm, b_mm))
                        }
                        _ => None,
                    })
                    .ok_or_else(unsupported)
            })
            .collect::<Result<_, _>>()?;
        let (first, last) = lines[0];
        let axes: Vec<_> = (0..3).filter(|&axis| first[axis] != last[axis]).collect();
        let [along] = axes.as_slice() else {
            return Err(unsupported());
        };
        let along = *along;
        let a = (along + 1) % 3;
        let b = (along + 2) % 3;
        let corners = [
            [stock.min[a], stock.min[b]],
            [stock.max[a], stock.min[b]],
            [stock.max[a], stock.max[b]],
            [stock.min[a], stock.max[b]],
        ];
        let mut selected = [None; 4];
        for (index, (start, end)) in lines.into_iter().enumerate() {
            if start[along].min(end[along]) != stock.min[along]
                || start[along].max(end[along]) != stock.max[along]
                || start[a] != end[a]
                || start[b] != end[b]
            {
                return Err(unsupported());
            }
            let corner = corners
                .iter()
                .position(|point| *point == [start[a], start[b]])
                .ok_or_else(unsupported)?;
            if selected[corner].replace(index).is_some() {
                return Err(unsupported());
            }
        }
        policy.length(distance)?;
        // Every original side must survive. Adjacent bevels may not touch or
        // cross; a zero-width residual would change the source face topology.
        for i in 0..4 {
            let next = (i + 1) % 4;
            let length =
                (corners[next][0] - corners[i][0]).abs() + (corners[next][1] - corners[i][1]).abs();
            let removed =
                usize::from(selected[i].is_some()) + usize::from(selected[next].is_some());
            policy.length(length - distance * removed as f64)?;
        }
        let mut lower = Vec::with_capacity(4 + edges.len());
        let mut bevels = Vec::with_capacity(edges.len());
        for i in 0..4 {
            let corner = corners[i];
            let at = |point: [f64; 2]| {
                let mut p = stock.min;
                p[a] = point[0];
                p[b] = point[1];
                p
            };
            if let Some(source) = selected[i] {
                let offset = |toward: [f64; 2]| {
                    std::array::from_fn(|axis| {
                        if toward[axis] == corner[axis] {
                            corner[axis]
                        } else {
                            corner[axis] + (toward[axis] - corner[axis]).signum() * distance
                        }
                    })
                };
                bevels.push((lower.len(), source));
                lower.push(at(offset(corners[(i + 3) % 4])));
                lower.push(at(offset(corners[(i + 1) % 4])));
            } else {
                lower.push(at(corner));
            }
        }
        let upper: Vec<_> = lower
            .iter()
            .map(|point| {
                let mut p = *point;
                p[along] = stock.max[along];
                p
            })
            .collect();
        let mut faces = vec![lower.iter().rev().copied().collect(), upper.clone()];
        faces.extend((0..lower.len()).map(|i| {
            let next = (i + 1) % lower.len();
            vec![lower[i], lower[next], upper[next], upper[i]]
        }));
        let tolerance = policy.linear_tolerance_mm;
        let material = ConvexPiece::new(faces, tolerance).map_err(planar_error)?;
        let geometry =
            Union::new(std::slice::from_ref(&material), tolerance).map_err(planar_error)?;
        let expected = ((stock.max[a] - stock.min[a]) * (stock.max[b] - stock.min[b])
            - edges.len() as f64 * distance * distance / 2.)
            * (stock.max[along] - stock.min[along]);
        if geometry.solid_count() != 1
            || geometry.shell_count() != 1
            || geometry.faces().len() != 6 + edges.len()
            || (geometry.volume_mm3() - expected).abs() > expected * 1e-8
        {
            return Err(error(
                "chamfer",
                GeometryErrorKind::InvalidResult,
                "Rust multi-edge chamfer did not preserve the expected beveled shell",
            ));
        }
        let mut sources = vec![None; geometry.faces().len()];
        for (segment, source) in bevels {
            sources[2 + segment] = Some(topology.elements.len());
            topology.elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: u32::MAX,
                },
                origins: BTreeSet::from([edges[source].origin.clone()]),
                geometry: None,
                frames: Vec::new(),
                boundary: BTreeSet::new(),
            });
        }
        PlanarBooleanBody::build(
            geometry,
            vec![SourcePart {
                operand: 0,
                faces: sources,
                normal_sign: 1.,
            }],
            &[topology],
            feature,
            policy,
            Some(Material::Positive(vec![material])),
        )
    }
}
