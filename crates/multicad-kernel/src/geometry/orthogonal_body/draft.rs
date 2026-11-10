//! One planar side of a rectangular prism drafted about a cardinal neutral plane.
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
        let [selection] = faces else {
            return Err(draft_error(
                GeometryErrorKind::Unsupported,
                "Rust Draft requires exactly one selected side face",
            ));
        };
        if self.policy != policy {
            return Err(draft_error(
                GeometryErrorKind::Unsupported,
                "Rust Draft requires the source geometry policy",
            ));
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
        let (&(side, positive), _) = supports
            .iter()
            .find(|(_, item)| item.key == selection.key)
            .ok_or_else(unsupported_stock)?;
        if side == up {
            return Err(draft_error(
                GeometryErrorKind::Unsupported,
                "Rust Draft requires a side face parallel to the pull direction",
            ));
        }
        let u = (up + 1) % 3;
        let v = (up + 2) % 3;
        let chart = [u, v];
        let sign = if positive { 1. } else { -1. };
        let slope = -sign * pull[up] * angle.tan();
        let section = |level| {
            let mut ranges = chart.map(|i| [0., stock.max[i] - stock.min[i]]);
            let changing = usize::from(side == v);
            ranges[changing][usize::from(positive)] += slope * (level - neutral.origin[up]);
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
            if item.key == selection.key {
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
