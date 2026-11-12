//! Cardinal half-space clipping with authored cap ancestry.
use super::*;
use crate::split::checkpoint;

impl Solid {
    pub(crate) fn split_by_cardinal_plane(
        &self,
        plane: PlaneFrame,
        feature: Option<EntityId>,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<[Self; 2], GeometryError> {
        if self.policy != policy || policy.boolean_fuzzy_mm != 0. {
            return Err(unsupported(
                "Rust Plane Split requires the stock's exact geometry policy",
            ));
        }
        let normal = plane.normal();
        if normal.iter().filter(|&&n| n != 0.).count() != 1 {
            return Err(unsupported("Rust Plane Split requires a world-axis normal"));
        }
        let cut_axis = axis(normal);
        let offset = plane.origin[cut_axis];

        let mut sources = Sources::default();
        let Some((stock_feature, cells)) = self.orthogonal_operand(false, &mut sources)? else {
            return Err(unsupported(
                "Rust Plane Split requires world-aligned rectilinear stock without curved cuts",
            ));
        };
        let feature = feature.unwrap_or(stock_feature);
        let stock = extent(cells.iter().flat_map(|cell| [cell.min, cell.max]));
        if offset <= stock.min[cut_axis] || offset >= stock.max[cut_axis] {
            return Err(invalid(
                "Split plane must cross the target with positive-volume material on both sides",
            ));
        }
        let clearance = policy.minimum_feature_mm.max(policy.linear_tolerance_mm);
        // A cap on a stock-cell plane can coincide with old exposed material.
        // Accept only sections for which every coplanar output face is new.
        if cells.iter().any(|cell| {
            [cell.min[cut_axis], cell.max[cut_axis]]
                .into_iter()
                .any(|value| (value - offset).abs() < clearance)
        }) {
            return Err(invalid(
                "Split plane is too close to an existing rectilinear boundary",
            ));
        }

        let clips = [true, false].map(|positive| {
            let mut clip = stock;
            if positive == (normal[cut_axis] > 0.) {
                clip.min[cut_axis] = offset;
            } else {
                clip.max[cut_axis] = offset;
            }
            let frame = ReferenceFrame {
                origin_mm: plane.origin,
                u: plane.u,
                v: plane.v.map(|v| if positive { -v } else { v }),
            };
            let cap_normal = frame.normal();
            let origin = Origin {
                source: feature,
                role: if positive {
                    "split/plane/positive"
                } else {
                    "split/plane/negative"
                }
                .into(),
            };
            let mut face = element(
                TopologyKind::Face,
                0,
                BTreeSet::from([origin.clone()]),
                AnalyticGeometry::Plane { frame },
            );
            face.frames.push(AnchoredFrame { origin, frame });
            sources.faces.insert(
                plane_key(cap_normal, plane.origin),
                vec![Support {
                    patches: vec![box_face(clip, cap_normal)],
                    face,
                }],
            );
            clip
        });

        let build = |clip: Box3| {
            checkpoint(interrupt)?;
            let geometry = orthogonal::boolean(
                &cells,
                &[clip],
                Operation::Intersect,
                policy.linear_tolerance_mm,
            )
            .map_err(operation_error)?
            .ok_or_else(|| {
                invalid(
                    "Split plane must cross the target with positive-volume material on both sides",
                )
            })?;
            let body = OrthogonalBody::build(feature, geometry, Some(&sources))?;
            Ok::<_, GeometryError>(Self {
                body: SolidBody::Orthogonal(Rc::new(body), PlaneFrame::default()),
                policy,
                provenance: vec![],
                rust_origins: vec![],
                retained_placement: None,
            })
        };
        Ok([build(clips[0])?, build(clips[1])?])
    }
}
