//! Sharp, signed shell offset of one exact rectangular prism.
use super::*;
use crate::TreatmentFace;

impl Solid {
    pub(crate) fn shell_rectangular(
        &self,
        openings: &[TreatmentFace],
        thickness: f64,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if openings.is_empty() {
            return Err(shell_error(
                GeometryErrorKind::Unsupported,
                "Rust Shell requires at least one opening face",
            ));
        }
        if self.policy != policy {
            return Err(shell_error(
                GeometryErrorKind::Unsupported,
                "Rust Shell requires the source geometry policy",
            ));
        }
        // Use the body's exact chart. Removing placement does not reorder its
        // topology, so the captured world face ordinal selects the same cap.
        let mut local = self.share()?;
        let placement = match &mut local.body {
            SolidBody::Prism(_, frame)
            | SolidBody::Linear(_, frame)
            | SolidBody::Orthogonal(_, frame) => std::mem::take(frame),
            SolidBody::PolygonPrism(body) => std::mem::take(&mut Rc::make_mut(body).frame),
            _ => return Err(unsupported_stock()),
        };
        let mut sources = Sources::default();
        let (_, cells) = local
            .orthogonal_operand(false, &mut sources)?
            .ok_or_else(unsupported_stock)?;
        let [stock] = cells.as_slice() else {
            return Err(unsupported_stock());
        };
        let stock = *stock;
        let faces = std::mem::take(&mut sources.faces);
        // Verify that this is one box with exactly six authored rectangular
        // face supports, even when the source was an earlier solid result.
        if faces.len() != 6
            || !faces.iter().all(|(&(axis, _, positive), supports)| {
                matches!(supports.as_slice(), [support]
                    if support.patches == [box_face(stock, cardinal_normal(axis, positive))])
            })
        {
            return Err(unsupported_stock());
        }
        let mut selected = BTreeMap::new();
        for opening in openings {
            let (&(axis, _, positive), _) = faces
                .iter()
                .find(|(_, supports)| supports[0].face.key == opening.key)
                .ok_or_else(unsupported_stock)?;
            selected.insert((axis, positive), opening);
        }

        let mut shifted = stock;
        for i in 0..3 {
            if !selected.contains_key(&(i, false)) {
                shifted.min[i] += thickness;
            }
            if !selected.contains_key(&(i, true)) {
                shifted.max[i] -= thickness;
            }
        }
        if (0..3).any(|i| shifted.max[i] - shifted.min[i] < policy.minimum_feature_mm) {
            return Err(shell_error(
                GeometryErrorKind::InvalidResult,
                "Shell thickness crosses the source or leaves no cavity; reduce thickness",
            ));
        }
        let inward = thickness > 0.;
        let (outer, inner) = if inward {
            (stock, shifted)
        } else {
            (shifted, stock)
        };
        let world_bounds =
            extent(box_corners(outer).map(|point| transform_point(placement, point)));
        policy.point(world_bounds.min)?;
        policy.point(world_bounds.max)?;

        for ((axis, _, positive), supports) in faces {
            let mut face = supports
                .into_iter()
                .next()
                .expect("one checked support")
                .face;
            let normal = cardinal_normal(axis, positive);
            if let Some(opening) = selected.get(&(axis, positive)) {
                let frame = offset_face(&mut face, [0.; 3], false);
                face.frames.push(AnchoredFrame {
                    origin: opening.origin.clone(),
                    frame,
                });
                face.origins.insert(opening.origin.clone());
                insert_face(&mut sources, box_face(outer, normal), normal, face);
            } else {
                let mut offset = face.clone();
                offset_face(&mut face, [0.; 3], !inward);
                insert_face(
                    &mut sources,
                    box_face(stock, normal),
                    if inward { normal } else { normal.map(|v| -v) },
                    face,
                );
                let frame = offset_face(&mut offset, normal.map(|v| -thickness * v), inward);
                let generated: Vec<_> = offset
                    .origins
                    .iter()
                    .map(|origin| Origin {
                        source: feature,
                        role: format!("offset/{}/{}", origin.source, origin.role),
                    })
                    .collect();
                offset.frames.extend(
                    generated
                        .iter()
                        .cloned()
                        .map(|origin| AnchoredFrame { origin, frame }),
                );
                offset.origins.extend(generated);
                insert_face(
                    &mut sources,
                    box_face(shifted, normal),
                    if inward { normal.map(|v| -v) } else { normal },
                    offset,
                );
            }
        }
        sources.name_with_adjacent_faces = true;
        let geometry = orthogonal::boolean(
            &[outer],
            &[inner],
            Operation::Subtract,
            policy.linear_tolerance_mm,
        )
        .map_err(operation_error)?
        .filter(|result| result.solid_count() == 1)
        .ok_or_else(|| {
            shell_error(
                GeometryErrorKind::InvalidResult,
                "Shell did not produce one connected solid",
            )
        })?;
        let body = OrthogonalBody::build(feature, geometry, Some(&sources))?;
        Ok(Self {
            body: SolidBody::Orthogonal(Rc::new(body), placement),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }
}

fn cardinal_normal(axis: usize, positive: bool) -> Point {
    let mut normal = [0.; 3];
    normal[axis] = if positive { 1. } else { -1. };
    normal
}

fn box_corners(bounds: Box3) -> [Point; 8] {
    std::array::from_fn(|mask| {
        std::array::from_fn(|i| {
            if mask & (1 << i) == 0 {
                bounds.min[i]
            } else {
                bounds.max[i]
            }
        })
    })
}

fn offset_face(face: &mut TopologyElement, displacement: Point, reversed: bool) -> ReferenceFrame {
    let shift = |frame: ReferenceFrame| ReferenceFrame {
        origin_mm: std::array::from_fn(|i| frame.origin_mm[i] + displacement[i]),
        v: frame.v.map(|value| if reversed { -value } else { value }),
        ..frame
    };
    let Some(AnalyticGeometry::Plane { frame }) = face.geometry else {
        unreachable!("certified box plane");
    };
    let frame = shift(frame);
    face.geometry = Some(AnalyticGeometry::Plane { frame });
    for anchor in &mut face.frames {
        anchor.frame = shift(anchor.frame);
        if dot(anchor.frame.normal(), frame.normal()) < 0. {
            anchor.frame.v = anchor.frame.v.map(|v| -v);
        }
    }
    frame
}

fn insert_face(sources: &mut Sources, patch: Box3, normal: Point, face: TopologyElement) {
    sources
        .faces
        .entry(plane_key(normal, patch.min))
        .or_default()
        .push(Support {
            patches: vec![patch],
            face,
        });
}

fn unsupported_stock() -> GeometryError {
    shell_error(
        GeometryErrorKind::Unsupported,
        "Rust Shell requires one rectangular prism without holes or curved faces",
    )
}

fn shell_error(kind: GeometryErrorKind, message: &str) -> GeometryError {
    error("shell", kind, message)
}
