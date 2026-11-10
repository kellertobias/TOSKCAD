use super::*;

impl Chart {
    fn record_axial_source(&mut self, tool: &Solid) -> Result<usize, GeometryError> {
        let topology = tool
            .rust_topology()
            .ok_or_else(|| warped_error("axial source topology", warped_union::Error::Invalid))??;
        let mut faces = Vec::new();
        for (index, element) in topology.elements.iter().enumerate() {
            if element.key.kind == TopologyKind::Face {
                let ordinal = element.key.ordinal as usize;
                if ordinal >= faces.len() {
                    faces.resize(ordinal + 1, None);
                }
                faces[ordinal] = Some(index);
            }
        }
        let source = self.axial_sources.len();
        self.axial_sources.push(SourcePart {
            operand: self.topologies.len(),
            faces,
        });
        self.topologies.push(Rc::new(topology));
        self.axial_bounds.push(tool.bounds()?);
        Ok(source)
    }
}

impl Solid {
    /// Subtract certified planar and axial cutters while retaining exact
    /// quarter-round and cylindrical surfaces.
    pub fn subtract_warped_tools(
        &self,
        tools: &[&Solid],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Solid, GeometryError>> {
        if tools.is_empty()
            || policy.boolean_fuzzy_mm != 0.
            || self.policy != policy
            || tools
                .iter()
                .any(|tool| tool.policy != policy || !tool.uses_rust_geometry())
        {
            return None;
        }
        let (frame, existing, cells) = if let Some((body, frame)) = self.warped_union_body() {
            (frame, Some(body.chart.clone()?), None)
        } else {
            let (body, frame) = self.orthogonal_body()?;
            if body.has_curved_cuts()
                || !tools
                    .iter()
                    .any(|tool| tool.axial_cylinder_frame().is_some())
                || !tools.iter().any(|tool| {
                    tool.axial_cylinder_frame().is_none()
                        && tool.orthogonal_cells().is_ok_and(|cells| cells.is_none())
                })
            {
                return None;
            }
            (frame, None, Some(body.geometry.cells()))
        };
        let axes = [frame.u, frame.v, frame.normal()];
        let inverse = RigidTransform {
            rotation: axes,
            translation_mm: axes.map(|axis| -dot(axis, frame.origin)),
        };
        let mut chart = if let Some(chart) = existing {
            chart
        } else {
            let cells = cells?;
            let result = self.transformed(inverse).and_then(|stock| {
                let topology = stock.rust_topology().ok_or_else(|| {
                    warped_error("planar stock topology", warped_union::Error::Invalid)
                })??;
                flat_stock::chart(cells, topology, policy)
            });
            match result {
                Ok(chart) => chart,
                Err(cause) => return Some(Err(cause)),
            }
        };
        let tolerance = policy.linear_tolerance_mm;
        let local = match tools
            .iter()
            .map(|tool| tool.transformed(inverse))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(local) => local,
            Err(cause) => return Some(Err(cause)),
        };
        let axial_cut::Classified { planar, cuts } =
            match axial_cut::classify(&local, &chart.bosses, tolerance) {
                Ok(classified) => classified,
                Err(cause) => return Some(Err(cause)),
            };
        let curved_bounds: Vec<_> = chart
            .bosses
            .iter()
            .map(AuthoredBoss::bounds)
            .chain(
                chart
                    .axial
                    .iter()
                    .map(|cut| cut.bounds(&chart.axial_bounds)),
            )
            .collect();
        for index in planar {
            let tool = &local[index];
            if !curved_bounds.is_empty() {
                let bounds = match tool.bounds() {
                    Ok(bounds) => bounds,
                    Err(cause) => return Some(Err(cause)),
                };
                if curved_bounds.iter().any(|region| {
                    (0..3).all(|i| {
                        bounds.min[i] <= region.max[i] + tolerance
                            && region.min[i] <= bounds.max[i] + tolerance
                    })
                }) {
                    return Some(Err(error(
                        "quarter-round Boolean",
                        GeometryErrorKind::Unsupported,
                        "This planar cutter may intersect an attached cylinder or axial cut",
                    )));
                }
            }
            if let Err(cause) = chart.append(tool, None, policy)? {
                return Some(Err(cause));
            }
        }
        for mut cut in cuts {
            cut.source = match chart.record_axial_source(&local[cut.source]) {
                Ok(source) => source,
                Err(cause) => return Some(Err(cause)),
            };
            if let Some(recess) = &mut cut.recess {
                recess.source = match chart.record_axial_source(&local[recess.source]) {
                    Ok(source) => source,
                    Err(cause) => return Some(Err(cause)),
                };
            }
            chart.axial.push(AuthoredAxialCut {
                cut,
                open_side: None,
            });
        }
        let [stock, cutters] = [
            &chart.pieces[..chart.stock_count],
            &chart.pieces[chart.stock_count..],
        ];
        let union = if cutters.is_empty() {
            Union::new(stock, tolerance).map(Some)
        } else {
            Union::subtract(stock, cutters, tolerance)
        };
        let union = match union {
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
