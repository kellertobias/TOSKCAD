//! Bevel one complete straight edge of a world-aligned rectangular solid.
use super::*;
use crate::TreatmentEdge;
use multicad_core::references::{TopologyElement, TopologyKey};

fn unsupported() -> GeometryError {
    error(
        "chamfer",
        GeometryErrorKind::Unsupported,
        "Rust straight chamfer requires one complete edge of a world-aligned rectangular solid",
    )
}

impl Solid {
    /// Use a convex five-sided section along the selected stock edge.
    pub(crate) fn rust_chamfer_box_edge(
        &self,
        edge: &TreatmentEdge,
        reference_face: Option<TopologyKey>,
        distances_mm: [f64; 2],
        feature: EntityId,
        policy: GeometryPolicy,
        mut topology: TopologySnapshot,
    ) -> Result<Self, GeometryError> {
        if self.policy != policy || policy.boolean_fuzzy_mm != 0. {
            return Err(unsupported());
        }
        let cells = self.orthogonal_cells()?.ok_or_else(unsupported)?;
        let [stock] = cells.as_slice() else {
            return Err(unsupported());
        };
        let Some(AnalyticGeometry::Line { a_mm, b_mm }) = topology
            .elements
            .iter()
            .find(|element| element.key == edge.key)
            .and_then(|element| element.geometry)
        else {
            return Err(unsupported());
        };
        let axes: Vec<_> = (0..3).filter(|&i| a_mm[i] != b_mm[i]).collect();
        let [along] = axes.as_slice() else {
            return Err(unsupported());
        };
        let along = *along;
        if a_mm[along].min(b_mm[along]) != stock.min[along]
            || a_mm[along].max(b_mm[along]) != stock.max[along]
        {
            return Err(unsupported());
        }
        let mut across = [(along + 1) % 3, (along + 2) % 3];
        if across
            .iter()
            .any(|&i| a_mm[i] != stock.min[i] && a_mm[i] != stock.max[i])
        {
            return Err(unsupported());
        }

        let inward = |axis| {
            if a_mm[axis] == stock.min[axis] {
                1.
            } else {
                -1.
            }
        };
        if let Some(key) = reference_face {
            let frame = topology.elements.iter().find_map(|element| {
                if element.key == key
                    && let Some(AnalyticGeometry::Plane { frame }) = element.geometry
                {
                    Some(frame)
                } else {
                    None
                }
            });
            let side = frame.and_then(|frame| {
                across.iter().position(|&axis| {
                    let mut normal = [0.; 3];
                    normal[axis] = -inward(axis);
                    frame.normal() == normal && frame.origin_mm[axis] == a_mm[axis]
                })
            });
            match side {
                // The first distance lies in the selected face.
                Some(0) => across.swap(0, 1),
                Some(1) => (),
                _ => return Err(input("Chamfer reference face must touch the selected edge")),
            }
        }

        let [a, b] = across;
        let [da, db] = distances_mm;
        let [width, height] = across.map(|i| stock.max[i] - stock.min[i]);
        for length in [da, db, width - da, height - db] {
            policy.length(length)?;
        }
        let coordinate = |axis, offset: f64, extent: f64| {
            if offset == extent {
                if inward(axis) > 0. {
                    stock.max[axis]
                } else {
                    stock.min[axis]
                }
            } else {
                a_mm[axis] + inward(axis) * offset
            }
        };
        let mut lower = [
            [da, 0.],
            [width, 0.],
            [width, height],
            [0., height],
            [0., db],
        ]
        .map(|[u, v]| {
            let mut point = stock.min;
            point[a] = coordinate(a, u, width);
            point[b] = coordinate(b, v, height);
            point
        });
        if cross(subtract(lower[1], lower[0]), subtract(lower[2], lower[0]))[along] < 0. {
            lower.reverse();
        }
        let upper = lower.map(|mut point| {
            point[along] = stock.max[along];
            point
        });
        let mut faces = vec![lower.iter().rev().copied().collect(), upper.to_vec()];
        faces.extend((0..lower.len()).map(|i| {
            let next = (i + 1) % lower.len();
            vec![lower[i], lower[next], upper[next], upper[i]]
        }));

        let tolerance = policy.linear_tolerance_mm;
        let material = ConvexPiece::new(faces, tolerance).map_err(planar_error)?;
        let geometry =
            Union::new(std::slice::from_ref(&material), tolerance).map_err(planar_error)?;
        let expected = (width * height - da * db * 0.5) * (stock.max[along] - stock.min[along]);
        if geometry.solid_count() != 1
            || geometry.shell_count() != 1
            || geometry.faces().len() != 7
            || (geometry.volume_mm3() - expected).abs() > expected * 1e-8
        {
            return Err(error(
                "chamfer",
                GeometryErrorKind::InvalidResult,
                "Rust chamfer did not preserve the expected beveled shell",
            ));
        }
        let generated = topology.elements.len();
        topology.elements.push(TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: u32::MAX,
            },
            origins: BTreeSet::from([edge.origin.clone()]),
            geometry: None,
            frames: Vec::new(),
            boundary: BTreeSet::new(),
        });
        let mut sources = vec![None; 7];
        // Reversing the section still leaves the last-to-first edge as the bevel.
        sources[6] = Some(generated);
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
            Some(vec![material]),
        )
    }
}
