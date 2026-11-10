//! Affine reconstruction of one certified convex planar shell.
use super::*;
use crate::AffineTransform;
use multicad_core::references::TopologyElement;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BoundaryKey {
    Vertex([u64; 3]),
    Edge([[u64; 3]; 2]),
}

impl BoundaryKey {
    fn vertex(point: Point) -> Self {
        Self::Vertex(point_key(point))
    }

    fn edge(ends: [Point; 2]) -> Self {
        let mut ends = ends.map(point_key);
        ends.sort_unstable();
        Self::Edge(ends)
    }
}

fn point_key(point: Point) -> [u64; 3] {
    point.map(|value| if value == 0. { 0 } else { value.to_bits() })
}

fn failure(kind: GeometryErrorKind, message: &str) -> GeometryError {
    error("scale", kind, message)
}

fn unsupported() -> GeometryError {
    failure(
        GeometryErrorKind::Unsupported,
        "Rust Scale currently requires one convex solid with straight planar boundaries",
    )
}

fn ancestry_error() -> GeometryError {
    failure(
        GeometryErrorKind::InvalidResult,
        "Rust Scale could not preserve a one-to-one boundary ancestry",
    )
}

struct BoundaryOrigin {
    origins: BTreeSet<Origin>,
    boundary: BTreeSet<Origin>,
}

pub(super) struct Ancestry {
    edges: Vec<BoundaryOrigin>,
    vertices: Vec<BoundaryOrigin>,
}

impl Ancestry {
    fn match_boundary(
        body: &PlanarBooleanBody,
        mut sources: BTreeMap<BoundaryKey, BoundaryOrigin>,
    ) -> Result<Self, GeometryError> {
        let mut take = |key| sources.remove(&key).ok_or_else(ancestry_error);
        let vertices = body
            .geometry
            .vertices()
            .iter()
            .map(|&point| take(BoundaryKey::vertex(point)))
            .collect::<Result<_, _>>()?;
        let edges = body
            .edges
            .iter()
            .map(|(indices, _)| {
                take(BoundaryKey::edge(
                    indices.map(|i| body.geometry.vertices()[i]),
                ))
            })
            .collect::<Result<_, _>>()?;
        if !sources.is_empty() {
            return Err(ancestry_error());
        }
        Ok(Self { edges, vertices })
    }

    pub(super) fn restore(&self, elements: &mut [TopologyElement]) {
        for element in elements {
            let index = element.key.ordinal as usize;
            let source = match element.key.kind {
                TopologyKind::Face => continue,
                TopologyKind::Edge => &self.edges[index],
                TopologyKind::Vertex => &self.vertices[index],
            };
            element.origins.extend(source.origins.iter().cloned());
            element.boundary.extend(source.boundary.iter().cloned());
        }
    }
}

impl Solid {
    pub(crate) fn rust_scale_planar(
        &self,
        transform: AffineTransform,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if self.solid_count()? != 1 {
            return Err(unsupported());
        }
        let mut topology = self.rust_topology().ok_or_else(unsupported)??;
        let (faces, ancestry) = convex_faces(self, &topology, policy).ok_or_else(unsupported)?;
        let mut boundary = BTreeMap::new();
        for element in &mut topology.elements {
            let geometry = element
                .geometry
                .ok_or_else(unsupported)
                .and_then(|geometry| transform.analytic(geometry))?
                .ok_or_else(unsupported)?;
            element.geometry = Some(geometry);
            let key = match geometry {
                AnalyticGeometry::Plane { .. } => {
                    for anchor in &mut element.frames {
                        anchor.frame = transform.frame(anchor.frame)?;
                    }
                    continue;
                }
                AnalyticGeometry::Point { position_mm } => BoundaryKey::vertex(position_mm),
                AnalyticGeometry::Line { a_mm, b_mm } => {
                    policy.length(norm(subtract(a_mm, b_mm)))?;
                    BoundaryKey::edge([a_mm, b_mm])
                }
                _ => return Err(unsupported()),
            };
            if boundary
                .insert(
                    key,
                    BoundaryOrigin {
                        origins: element.origins.clone(),
                        boundary: element.boundary.clone(),
                    },
                )
                .is_some()
            {
                return Err(ancestry_error());
            }
        }

        let loops = faces
            .into_iter()
            .map(|face| face.into_iter().map(|p| transform.point(p)).collect())
            .collect();
        let tolerance = policy.linear_tolerance_mm;
        let piece = ConvexPiece::new(loops, tolerance).map_err(planar_error)?;
        let geometry = Union::new(std::slice::from_ref(&piece), tolerance).map_err(planar_error)?;
        if geometry.solid_count() != 1
            || geometry.shell_count() != 1
            || geometry.faces().len() != ancestry.len()
            || geometry
                .faces()
                .iter()
                .map(|face| face.source_face)
                .collect::<BTreeSet<_>>()
                .len()
                != ancestry.len()
        {
            return Err(ancestry_error());
        }
        let expected = self.volume_mm3()? * transform.determinant();
        if !expected.is_finite() || (geometry.volume_mm3() - expected).abs() > expected * 1e-8 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Rust Scale did not conserve the affine material volume",
            ));
        }
        let mut result = PlanarBooleanBody::build(
            geometry,
            vec![SourcePart {
                operand: 0,
                faces: ancestry.into_iter().map(Some).collect(),
                normal_sign: 1.,
            }],
            &[topology],
            feature,
            policy,
            Some(vec![piece]),
        )?;
        let SolidBody::PlanarBoolean(body, _) = &mut result.body else {
            unreachable!("constructed planar scale")
        };
        let body = Rc::get_mut(body).expect("new planar scale");
        body.scaled_ancestry = Some(Ancestry::match_boundary(body, boundary)?);
        Ok(result)
    }
}
