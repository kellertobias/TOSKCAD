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
    faces: Vec<BoundaryOrigin>,
    edges: Vec<BoundaryOrigin>,
    vertices: Vec<BoundaryOrigin>,
}

impl Ancestry {
    fn match_boundary(
        body: &PlanarBooleanBody,
        mut sources: BTreeMap<BoundaryKey, BoundaryOrigin>,
        faces: Vec<BoundaryOrigin>,
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
        Ok(Self {
            faces,
            edges,
            vertices,
        })
    }

    pub(super) fn restore(&self, elements: &mut [TopologyElement]) {
        for element in elements {
            let index = element.key.ordinal as usize;
            let source = match element.key.kind {
                TopologyKind::Face => &self.faces[index],
                TopologyKind::Edge => &self.edges[index],
                TopologyKind::Vertex => &self.vertices[index],
            };
            element.origins.extend(source.origins.iter().cloned());
            // Scale is a one-to-one map. Reconstructed adjacency may include
            // member aliases, which must not change an authored discriminator.
            element.boundary.clone_from(&source.boundary);
        }
    }
}

impl Solid {
    /// A scaled convex shell with exactly the six box supports has the
    /// corresponding single occupied cell. Keep its published Scale names
    /// and let the orthogonal providers consume the cell only when needed.
    pub(in crate::geometry) fn scaled_orthogonal_cell(&self) -> Option<Box3> {
        let (body, frame) = self.planar_boolean_body()?;
        if body.scaled_ancestry.is_none() || body.geometry.faces().len() != 6 {
            return None;
        }
        let corners: &[Point; 8] = body.geometry.vertices().try_into().ok()?;
        let corners = corners.map(|point| position(frame, point));
        let mut bounds = Box3 {
            min: corners[0],
            max: corners[0],
        };
        for corner in corners {
            for (i, coordinate) in corner.into_iter().enumerate() {
                bounds.min[i] = bounds.min[i].min(coordinate);
                bounds.max[i] = bounds.max[i].max(coordinate);
            }
        }
        if (0..3).any(|i| bounds.min[i] >= bounds.max[i])
            || body.geometry.faces().iter().any(|face| {
                vector(frame, face.normal)
                    .into_iter()
                    .filter(|&n| n != 0.)
                    .count()
                    != 1
            })
        {
            return None;
        }
        let mut octants = 0u8;
        for corner in corners {
            let mut index = 0;
            for (i, coordinate) in corner.into_iter().enumerate() {
                if coordinate == bounds.max[i] {
                    index |= 1 << i;
                } else if coordinate != bounds.min[i] {
                    return None;
                }
            }
            octants |= 1 << index;
        }
        // A closed convex shell occupies the hull of its vertices. Each
        // corner must survive placement distinctly and without snapping.
        (octants == u8::MAX).then_some(bounds)
    }

    pub(crate) fn rust_scale_planar(
        &self,
        transform: AffineTransform,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if let Some((body, placement)) = self.planar_boolean_body() {
            return self.rust_scale_indexed(body, placement, transform, feature, policy);
        }
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
        let face_origins = geometry
            .faces()
            .iter()
            .map(|face| {
                let source = &topology.elements[ancestry[face.source_face]];
                BoundaryOrigin {
                    origins: source.origins.clone(),
                    boundary: source.boundary.clone(),
                }
            })
            .collect();
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
            Some(Material::Positive(vec![piece])),
        )?;
        let SolidBody::PlanarBoolean(body, _) = &mut result.body else {
            unreachable!("constructed planar scale")
        };
        let body = Rc::get_mut(body).expect("new planar scale");
        body.scaled_ancestry = Some(Ancestry::match_boundary(body, boundary, face_origins)?);
        Ok(result)
    }
    fn rust_scale_indexed(
        &self,
        source: &PlanarBooleanBody,
        placement: PlaneFrame,
        transform: AffineTransform,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        use multicad_rust_kernel::planar_boolean::AffineMap;
        let map = AffineMap::new(transform.matrix, transform.translation_mm)
            .and_then(|map| map.with_rigid_input(placement.origin, placement.u, placement.v))
            .map_err(planar_error)?;
        let tolerance = policy.linear_tolerance_mm;
        let geometry = source
            .geometry
            .affine_transformed(map, tolerance)
            .map_err(planar_error)?;
        let pieces = |pieces: &[ConvexPiece]| {
            pieces
                .iter()
                .map(|piece| {
                    piece
                        .transformed(|p| map.point(p), tolerance)
                        .map_err(planar_error)
                })
                .collect::<Result<Vec<_>, _>>()
        };
        let material = source
            .material
            .as_ref()
            .map(|material| {
                Ok(match material {
                    Material::Positive(original) => Material::Positive(pieces(original)?),
                    Material::LabeledCells {
                        pieces: original,
                        exterior_faces,
                    } => Material::LabeledCells {
                        pieces: pieces(original)?,
                        exterior_faces: exterior_faces.clone(),
                    },
                    Material::Cut(recipe) => Material::Cut(Box::new(CutRecipe {
                        stock: pieces(&recipe.stock)?,
                        tools: pieces(&recipe.tools)?,
                        certified: OnceCell::new(),
                    })),
                })
            })
            .transpose()?;
        let mut topology = self.rust_topology().ok_or_else(unsupported)??;
        for element in &mut topology.elements {
            element.geometry = Some(
                transform
                    .analytic(element.geometry.ok_or_else(unsupported)?)?
                    .ok_or_else(unsupported)?,
            );
            for anchor in &mut element.frames {
                anchor.frame = transform.frame(anchor.frame)?;
            }
            if let Some(AnalyticGeometry::Line { a_mm, b_mm }) = element.geometry {
                policy.length(norm(subtract(a_mm, b_mm)))?;
            }
        }
        let by_kind = |kind| {
            topology
                .elements
                .iter()
                .filter(|e| e.key.kind == kind)
                .map(|element| BoundaryOrigin {
                    origins: element.origins.clone(),
                    boundary: element.boundary.clone(),
                })
                .collect::<Vec<_>>()
        };
        let ancestry = Ancestry {
            faces: by_kind(TopologyKind::Face),
            edges: by_kind(TopologyKind::Edge),
            vertices: by_kind(TopologyKind::Vertex),
        };
        let faces: Vec<_> = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .collect();
        let mut result = PlanarBooleanBody::build_labeled(
            geometry,
            feature,
            policy,
            material,
            PlaneFrame::default(),
            |i, _, _| {
                let source = faces[i];
                let mut origins = vec![Origin {
                    source: feature,
                    role: format!("planar-boolean.face.{i}"),
                }];
                origins.extend(source.origins.iter().cloned());
                (origins, source.frames.clone())
            },
        )?;
        let SolidBody::PlanarBoolean(body, _) = &mut result.body else {
            unreachable!("constructed indexed planar scale")
        };
        let body = Rc::get_mut(body).expect("new indexed planar scale");
        if body
            .edges
            .iter()
            .map(|(key, _)| key)
            .ne(source.edges.iter().map(|(key, _)| key))
        {
            return Err(ancestry_error());
        }
        body.scaled_ancestry = Some(ancestry);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_import_scale_does_not_fabricate_boolean_material() {
        let policy = GeometryPolicy::default();
        let geometry = Union::new(
            &[ConvexPiece::from_box([0.; 3], [2.; 3], policy.linear_tolerance_mm).unwrap()],
            policy.linear_tolerance_mm,
        )
        .unwrap();
        let source =
            Solid::from_imported_planar_boundary(geometry, EntityId::new(), policy).unwrap();
        let result = source
            .affine_transformed_named(
                AffineTransform {
                    matrix: [[2., 0.5, 0.], [0., 1., 0.], [0., 0., 1.]],
                    ..Default::default()
                },
                EntityId::new(),
                policy,
            )
            .unwrap();
        assert!(result.planar_boolean_body().unwrap().0.material.is_none());
        assert_eq!(
            result
                .rust_subtract_planar_tools(&[&source], EntityId::new(), policy)
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Unsupported
        );
        assert_eq!(source.volume_mm3().unwrap(), 8.);
    }
}
