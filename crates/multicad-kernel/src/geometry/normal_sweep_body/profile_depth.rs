//! Retained full-column source charts, before world-coordinate evaluation.
use super::*;

pub(in crate::geometry) struct RetainedCap {
    pub(in crate::geometry) local: Solid,
    pub(in crate::geometry) placement: PlaneFrame,
    pub(in crate::geometry) depth_mm: f64,
    pub(in crate::geometry) opposite: u32,
    pub(in crate::geometry) frame: ReferenceFrame,
    pub(in crate::geometry) opposite_origin: [f64; 3],
}

impl Solid {
    pub(in crate::geometry) fn retained_cap_source(
        &self,
        selected: u32,
    ) -> Result<Option<RetainedCap>, GeometryError> {
        let (placement, depth_mm, opposite) = match &self.body {
            SolidBody::Prism(prism, placement) => {
                let plain = prism.rounded_end().is_none() && prism.inner_loops().len() == 0;
                if selected >= if plain { 6 } else { 2 } {
                    return Ok(None);
                }
                let dimensions = prism.dimensions().map(Rational::as_f64);
                let axis = match selected {
                    0 | 1 => 2,
                    2 | 4 => 1,
                    _ => 0,
                };
                (
                    *placement,
                    dimensions[axis],
                    [1, 0, 4, 5, 2, 3][selected as usize],
                )
            }
            SolidBody::PolygonPrism(body)
                if selected < 2 && body.pockets.is_empty() && body.bore_treatments.is_none() =>
            {
                (body.frame, body.prism.height().as_f64(), 1 - selected)
            }
            SolidBody::AxialCylinder(body, placement, _) if selected < 2 => {
                (*placement, body.height().as_f64(), 1 - selected)
            }
            SolidBody::NormalSweep(body, placement) if selected < 2 && body.openings.is_none() => {
                (*placement, body.sweep.height_mm(), 1 - selected)
            }
            _ => return Ok(None),
        };
        let mut local = self.share()?;
        match &mut local.body {
            SolidBody::Prism(_, frame)
            | SolidBody::AxialCylinder(_, frame, _)
            | SolidBody::NormalSweep(_, frame) => *frame = PlaneFrame::default(),
            SolidBody::PolygonPrism(body) => Rc::make_mut(body).frame = PlaneFrame::default(),
            _ => unreachable!("retained column representation"),
        }
        let topology = local.topology_snapshot(EntityId::from_uuid_bits(1), 0)?;
        let find = |ordinal| {
            topology
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == multicad_core::references::TopologyKind::Face
                        && e.key.ordinal == ordinal
                })
                .and_then(|e| match e.geometry {
                    Some(AnalyticGeometry::Plane { frame }) => Some(frame),
                    _ => None,
                })
        };
        let (Some(frame), Some(opposite_frame)) = (find(selected), find(opposite)) else {
            return Ok(None);
        };
        Ok(Some(RetainedCap {
            local,
            placement,
            depth_mm,
            opposite,
            frame,
            opposite_origin: opposite_frame.origin_mm,
        }))
    }

    /// Retained depth behind an exactly coincident canonical cap. No world
    /// endpoint subtraction may replace the descriptor's authored dimension.
    pub fn rust_prismatic_cap_depth(
        &self,
        frame: PlaneFrame,
    ) -> Result<Option<f64>, GeometryError> {
        frame.checked(self.policy)?;
        for selected in 0..6 {
            if let Some(source) = self.retained_cap_source(selected)? {
                let placed = place_frame(source.placement, source.frame);
                if frame
                    == (PlaneFrame {
                        origin: placed.origin_mm,
                        u: placed.u,
                        v: placed.v,
                    })
                {
                    return Ok(Some(source.depth_mm));
                }
            }
        }
        Ok(None)
    }
}
