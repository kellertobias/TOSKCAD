//! Planar sides of a rectangular prism drafted about a cardinal neutral plane.
use super::*;
use crate::TreatmentFace;
use multicad_rust_kernel::linear_sweep::Sweep;

impl Solid {
    pub(crate) fn draft_rectangular(
        &self,
        faces: &[TreatmentFace],
        pull: Point,
        neutral: PlaneFrame,
        angle: f64,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if self.policy != policy {
            return Err(draft_error(
                GeometryErrorKind::Unsupported,
                "Rust Draft requires the source geometry policy",
            ));
        }
        if let Some((source, placement)) = self.local_box_treatment_source()? {
            let axes: [Point; 3] = std::array::from_fn(|i| placement.rotation.map(|row| row[i]));
            let local = |direction| axes.map(|axis| dot(axis, direction));
            let bounds = source.bounds()?;
            let neutral_origin = local(subtract(neutral.origin, placement.translation_mm));
            let extent = (0..3)
                .map(|i| {
                    (bounds.max[i] - bounds.min[i])
                        .abs()
                        .max((bounds.min[i] - neutral_origin[i]).abs())
                        .max((bounds.max[i] - neutral_origin[i]).abs())
                })
                .sum::<f64>()
                .max(1.);
            // Remote plane origins and steep slopes amplify chart restoration
            // error; bound their resulting wall displacement before snapping.
            let amplification = (1. + angle.tan().abs()).powi(2);
            // Only restore roundoff introduced by the retained rigid chart.
            // An oblique pull or neutral plane needs a different Draft proof.
            let aligned = |direction: Point| {
                let up = (0..3)
                    .max_by(|&a, &b| direction[a].abs().total_cmp(&direction[b].abs()))
                    .expect("three axes");
                let residual = (0..3)
                    .map(|i| {
                        (direction[i] - if i == up { direction[i].signum() } else { 0. }).abs()
                    })
                    .fold(0_f64, f64::max);
                if residual > 64. * f64::EPSILON
                    || residual * extent * amplification > policy.linear_tolerance_mm * 0.25
                {
                    return Err(draft_error(
                        GeometryErrorKind::Unsupported,
                        "Rust Draft requires pull and neutral plane aligned with the source box chart",
                    ));
                }
                Ok((up, direction[up].signum()))
            };
            let (up, sense) = aligned(local(pull))?;
            if aligned(local(neutral.normal()))?.0 != up {
                return Err(draft_error(
                    GeometryErrorKind::Unsupported,
                    "Rust Draft requires pull normal to the neutral plane",
                ));
            }
            let basis = |axis| std::array::from_fn(|i| if i == axis { 1. } else { 0. });
            let neutral = PlaneFrame {
                origin: neutral_origin,
                u: basis((up + 1) % 3),
                v: basis((up + 2) % 3),
            };
            return source
                .draft_rectangular(
                    faces,
                    basis(up).map(|v| v * sense),
                    neutral,
                    angle,
                    feature,
                    policy,
                )?
                .transformed(placement);
        }
        let neutral_normal = neutral.normal();
        let up = axis(pull);
        if pull[up].abs() != 1.
            || (0..3).any(|i| i != up && (pull[i] != 0. || neutral_normal[i] != 0.))
        {
            return Err(draft_error(
                GeometryErrorKind::Unsupported,
                "Rust Draft requires cardinal pull normal to the neutral plane",
            ));
        }
        let [stock] = self
            .orthogonal_cells()?
            .ok_or_else(unsupported_stock)?
            .try_into()
            .map_err(|_| unsupported_stock())?;
        let snapshot = self
            .rust_topology()
            .expect("rectilinear Rust draft source")?;
        let mut supports = BTreeMap::new();
        for item in snapshot.elements {
            if let Some(AnalyticGeometry::Plane { frame }) = item.geometry {
                let normal = frame.normal();
                let i = axis(normal);
                let patch = box_face(stock, normal);
                if normal[i].abs() != 1.
                    || (0..3).any(|j| j != i && normal[j] != 0.)
                    || frame.origin_mm[i] != patch.min[i]
                    || item.origins.is_empty()
                    || supports.insert((i, normal[i] > 0.), item).is_some()
                {
                    return Err(unsupported_stock());
                }
            } else if !item.origins.is_subset(&item.boundary) {
                return Err(draft_error(
                    GeometryErrorKind::Unsupported,
                    "Rust Draft cannot preserve independently named source edges or vertices",
                ));
            }
        }
        if supports.len() != 6 {
            return Err(unsupported_stock());
        }
        let mut selected = BTreeMap::new();
        for selection in faces {
            let (&(side, positive), _) = supports
                .iter()
                .find(|(_, item)| item.key == selection.key)
                .ok_or_else(unsupported_stock)?;
            if side == up {
                return Err(draft_error(
                    GeometryErrorKind::Unsupported,
                    "Rust Draft requires side faces parallel to the pull direction",
                ));
            }
            selected.insert((side, positive), selection);
        }
        let u = (up + 1) % 3;
        let v = (up + 2) % 3;
        let chart = [u, v];
        let section = |level| {
            let mut ranges = chart.map(|i| [0., stock.max[i] - stock.min[i]]);
            for &(side, positive) in selected.keys() {
                let sign = if positive { 1. } else { -1. };
                let slope = -sign * pull[up] * angle.tan();
                let changing = usize::from(side == v);
                ranges[changing][usize::from(positive)] += slope * (level - neutral.origin[up]);
            }
            if ranges
                .iter()
                .any(|range| range[1] - range[0] < policy.minimum_feature_mm)
            {
                return Err(draft_error(
                    GeometryErrorKind::InvalidResult,
                    "Draft angle crosses the opposite side; reduce the angle",
                ));
            }
            let [[x0, x1], [y0, y1]] = ranges;
            Ok(vec![vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]])
        };
        let base = section(stock.min[up])?;
        let top = section(stock.max[up])?;
        let geometry = Sweep::new(base, top, stock.max[up] - stock.min[up]).map_err(|cause| {
            draft_error(
                GeometryErrorKind::InvalidResult,
                format!("Draft sections cannot form a closed planar solid: {cause:?}"),
            )
        })?;
        let basis = |axis| std::array::from_fn(|i| if i == axis { 1. } else { 0. });
        let frame = PlaneFrame {
            origin: stock.min,
            u: basis(u),
            v: basis(v),
        };
        let origins = [
            (up, false),
            (up, true),
            (v, false),
            (u, true),
            (v, true),
            (u, false),
        ]
        .map(|key| {
            let item = supports.remove(&key).expect("six distinct box faces");
            let mut origins = item.origins;
            if let Some(selection) = selected.get(&key) {
                origins.insert(selection.origin.clone());
            }
            origins.into_iter().collect()
        });
        Self::from_attributed_linear_draft(geometry, frame, feature, origins, policy)
    }
}

fn unsupported_stock() -> GeometryError {
    draft_error(
        GeometryErrorKind::Unsupported,
        "Rust Draft requires one world-aligned rectangular prism without holes",
    )
}

fn draft_error(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    error("draft", kind, message)
}
