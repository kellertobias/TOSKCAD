use super::*;

impl Solid {
    /// Subtract exact planar cutters from a solid whose fillets use a
    /// certified quarter-round height band.
    pub fn subtract_warped_tools(
        &self,
        tools: &[&Solid],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Solid, GeometryError>> {
        let (body, frame) = self.warped_union_body()?;
        if tools.is_empty()
            || policy.boolean_fuzzy_mm != 0.
            || self.policy != policy
            || tools
                .iter()
                .any(|tool| tool.policy != policy || !tool.uses_rust_geometry())
        {
            return None;
        }
        let axes = [frame.u, frame.v, frame.normal()];
        let inverse = RigidTransform {
            rotation: axes,
            translation_mm: axes.map(|axis| -dot(axis, frame.origin)),
        };
        let mut chart = body.chart.clone()?;
        let boss_bounds: Vec<_> = chart.bosses.iter().map(AuthoredBoss::bounds).collect();
        let tolerance = policy.linear_tolerance_mm;
        for tool in tools {
            let local = match tool.transformed(inverse) {
                Ok(local) => local,
                Err(cause) => return Some(Err(cause)),
            };
            if !boss_bounds.is_empty() {
                let bounds = match local.bounds() {
                    Ok(bounds) => bounds,
                    Err(cause) => return Some(Err(cause)),
                };
                if boss_bounds.iter().any(|boss| {
                    (0..3).all(|i| {
                        bounds.min[i] <= boss.max[i] + tolerance
                            && boss.min[i] <= bounds.max[i] + tolerance
                    })
                }) {
                    return Some(Err(error(
                        "quarter-round Boolean",
                        GeometryErrorKind::Unsupported,
                        "This cutter may intersect an attached cylinder",
                    )));
                }
            }
            if let Err(cause) = chart.append(&local, None, policy)? {
                return Some(Err(cause));
            }
        }
        let [stock, cutters] = [
            &chart.pieces[..chart.stock_count],
            &chart.pieces[chart.stock_count..],
        ];
        let union = match Union::subtract(stock, cutters, tolerance) {
            Ok(Some(union)) => union,
            Ok(None) => {
                return Some(Err(error(
                    "quarter-round Boolean",
                    GeometryErrorKind::InvalidResult,
                    "Boolean removed all material; no output body can be committed",
                )));
            }
            Err(cause) => return Some(Err(source_error(cause))),
        };
        let result = chart.finish(union, feature, policy).and_then(|local| {
            local.transformed(RigidTransform {
                rotation: std::array::from_fn(|i| axes.map(|axis| axis[i])),
                translation_mm: frame.origin,
            })
        });
        Some(result)
    }
}
