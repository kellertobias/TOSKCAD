//! Admit one certified straight extrusion without partitioning its logical faces.
use super::*;

impl Solid {
    pub(super) fn rust_scale_simple_concave(
        &self,
        transform: AffineTransform,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Option<Self>, GeometryError> {
        if self.polygon_prism().is_none() && self.linear_body().is_none() {
            return Ok(None);
        }
        let (local, placement) = self.local_box_treatment_source()?.map_or_else(
            || self.share().map(|solid| (solid, RigidTransform::default())),
            Ok,
        )?;
        let topology = local.rust_topology().ok_or_else(unsupported)??;
        let Some((rings, ancestry)) = convex_faces(&local, &topology, policy) else {
            // A face with holes needs a logical multi-wire representation.
            return Ok(None);
        };
        let tolerance = policy.linear_tolerance_mm;
        let nonconvex = rings.iter().zip(&ancestry).any(|(ring, &index)| {
            let Some(AnalyticGeometry::Plane { frame }) = topology.elements[index].geometry else {
                unreachable!("planar source faces")
            };
            rings
                .iter()
                .flatten()
                .any(|&point| dot(frame.normal(), subtract(point, ring[0])) > tolerance)
        });
        if !nonconvex {
            return Ok(None);
        }
        // Only the geometrically nonconvex source selects this provider.
        // Invalid/Limit errors from either proof propagate without fallback.
        let geometry = Union::from_planar_rings(&rings, tolerance).map_err(planar_error)?;
        let volume = local.volume_mm3()?;
        if (geometry.volume_mm3() - volume).abs() > volume * 1e-8 {
            return Err(ancestry_error());
        }
        let material = straight_profile_material(&rings, tolerance)?;
        let face_origins = ancestry
            .iter()
            .map(|&index| BoundaryOrigin {
                origins: topology.elements[index].origins.clone(),
                boundary: topology.elements[index].boundary.clone(),
            })
            .collect();
        let mut boundary = BTreeMap::new();
        for element in &topology.elements {
            let key = match element.geometry {
                Some(AnalyticGeometry::Point { position_mm }) => BoundaryKey::vertex(position_mm),
                Some(AnalyticGeometry::Line { a_mm, b_mm }) => BoundaryKey::edge([a_mm, b_mm]),
                Some(AnalyticGeometry::Plane { .. }) => continue,
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
        let mut admitted = PlanarBooleanBody::build(
            geometry,
            vec![SourcePart {
                operand: 0,
                faces: ancestry.iter().copied().map(Some).collect(),
                normal_sign: 1.,
            }],
            &[topology],
            feature,
            policy,
            Some(material),
        )?;
        let SolidBody::PlanarBoolean(body, _) = &mut admitted.body else {
            unreachable!("admitted planar boundary")
        };
        let body = Rc::get_mut(body).expect("new admitted planar boundary");
        body.scaled_ancestry = Some(Ancestry::match_boundary(body, boundary, face_origins)?);
        let result = admitted
            .transformed(placement)?
            .affine_transformed_named(transform, feature, policy)?;
        let expected = self.volume_mm3()? * transform.determinant();
        if (result.volume_mm3()? - expected).abs() > expected * 1e-8 {
            return Err(ancestry_error());
        }
        Ok(Some(result))
    }
}

/// Reuse the certified translated-section partition in the caller's local
/// chart. Exterior labels are logical face ordinals; internal seams have None.
pub(in crate::geometry::planar_boolean_body) fn straight_profile_material(
    rings: &[Vec<Point>],
    tolerance: f64,
) -> Result<Material, GeometryError> {
    let [base, end, sides @ ..] = rings else {
        return Err(unsupported());
    };
    if base.len() != end.len() || sides.len() != base.len() {
        return Err(unsupported());
    }
    let anchor = |ring: &[Point]| {
        *ring
            .iter()
            .min_by(|a, b| {
                (0..3)
                    .map(|i| a[i].total_cmp(&b[i]))
                    .find(|order| !order.is_eq())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("closed face")
    };
    let displacement = subtract(anchor(end), anchor(base));
    let translated = |p: Point| std::array::from_fn(|i| p[i] + displacement[i]);
    let near = |a, b| norm(subtract(a, b)) <= tolerance;
    if !base
        .iter()
        .all(|&p| end.iter().any(|&q| near(translated(p), q)))
    {
        return Err(unsupported());
    }
    let side_faces = base
        .iter()
        .zip(base.iter().cycle().skip(1))
        .map(|(&a, &b)| {
            let expected = [a, b, translated(a), translated(b)];
            sides
                .iter()
                .position(|ring| {
                    ring.len() == 4 && expected.iter().all(|&p| ring.iter().any(|&q| near(p, q)))
                })
                .map(|i| i + 2)
                .ok_or_else(unsupported)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if displacement[2].abs() <= tolerance
        || base.iter().any(|p| (p[2] - base[0][2]).abs() > tolerance)
    {
        return Err(unsupported());
    }
    let section: Vec<_> = base.iter().map(|p| [p[0], p[1]]).collect();
    let shifted: Vec<_> = base
        .iter()
        .map(|&p| {
            let q = translated(p);
            [q[0], q[1]]
        })
        .collect();
    let (sections, heights, caps) = if displacement[2] > 0. {
        (
            [section, shifted],
            [base[0][2], base[0][2] + displacement[2]],
            [0, 1],
        )
    } else {
        (
            [shifted, section],
            [base[0][2] + displacement[2], base[0][2]],
            [1, 0],
        )
    };
    let (_, parts) =
        multicad_rust_kernel::planar_boolean::ruled_loft(&sections, &heights, tolerance)
            .map_err(planar_error)?;
    let (pieces, exterior_faces) = parts
        .into_iter()
        .map(|part| {
            let faces = part
                .source_faces
                .into_iter()
                .map(|face| match face {
                    ProfileFace::StartCap => Some(caps[0]),
                    ProfileFace::EndCap => Some(caps[1]),
                    ProfileFace::Edge(edge) => Some(side_faces[edge]),
                    ProfileFace::Partition => None,
                })
                .collect();
            (part.piece, faces)
        })
        .unzip();
    // Face ordinals above belong to the unpartitioned admitted boundary;
    // partition walls deliberately carry no authored exterior identity.
    Ok(Material::LabeledCells {
        pieces,
        exterior_faces,
    })
}
