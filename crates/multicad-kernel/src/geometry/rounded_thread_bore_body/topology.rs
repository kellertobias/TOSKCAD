use super::*;
use multicad_core::references::{AnchoredFrame, ReferenceFrame};
impl RoundedThreadBoreBody {
    fn origin(&self, role: impl Into<String>) -> Origin {
        Origin {
            source: self.feature,
            role: role.into(),
        }
    }
    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let mut result = self.inner.topology(frame, owner, revision)?;
        // The same generating meridian has opposite material side in a bore.
        // Never let a previous solid-shaft wall reference silently bind inward.
        let inward = |origin: &Origin| Origin {
            source: origin.source,
            role: if origin.role.starts_with("thread/") || origin.role == "thread.root" {
                format!("receiving.inward/{}", origin.role)
            } else {
                origin.role.clone()
            },
        };
        for element in &mut result.elements {
            element.origins = element.origins.iter().map(inward).collect();
            element.boundary = element.boundary.iter().map(inward).collect();
        }

        let wall = self.origin("receiving.outer_wall");
        let rims = [
            self.origin("receiving.outer_rim.start"),
            self.origin("receiving.outer_rim.end"),
        ];
        for (cap, rim) in rims.iter().enumerate() {
            result.elements[cap].boundary.insert(rim.clone());
            result.elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Edge,
                    ordinal: 15 + cap as u32,
                },
                origins: BTreeSet::from([rim.clone()]),
                boundary: BTreeSet::from([
                    self.origin(if cap == 0 { "cap.start" } else { "cap.end" }),
                    wall.clone(),
                ]),
                frames: vec![],
                geometry: Some(AnalyticGeometry::CircularArc {
                    frame: ReferenceFrame {
                        origin_mm: position(
                            frame,
                            [0., 0., cap as f64 * self.geometry.length_mm()],
                        ),
                        u: frame.u,
                        v: frame.v,
                    },
                    radius_mm: self.geometry.radius_mm(),
                    start_rad: 0.,
                    sweep_rad: std::f64::consts::TAU,
                }),
            });
        }
        result.elements.push(TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: 7,
            },
            origins: BTreeSet::from([wall]),
            boundary: rims.into_iter().collect(),
            frames: vec![],
            geometry: None,
        });
        if let Some((source, authored, reverse)) = &self.inherited {
            let chart = helical_body::stock_frame::Chart::new(*authored);
            for element in &mut result.elements {
                let source_key = match (element.key.kind, element.key.ordinal) {
                    (TopologyKind::Face, cap @ 0..=1) => {
                        Some((TopologyKind::Face, if *reverse { 1 - cap } else { cap }))
                    }
                    (TopologyKind::Face, 7) => Some((TopologyKind::Face, 2)),
                    (TopologyKind::Edge, rim @ 15..=16) => Some((
                        TopologyKind::Edge,
                        if *reverse { 16 - rim } else { rim - 15 },
                    )),
                    _ => None,
                };
                if let Some((kind, ordinal)) = source_key
                    && let Some(old) = source
                        .elements
                        .iter()
                        .find(|e| e.key.kind == kind && e.key.ordinal == ordinal)
                {
                    element.origins.extend(old.origins.iter().cloned());
                    for anchor in &old.frames {
                        let local_origin = chart.point(anchor.frame.origin_mm);
                        let local_u = chart.vector(anchor.frame.u);
                        let local_v = chart.vector(anchor.frame.v);
                        let direction = |p: Point| {
                            std::array::from_fn(|i| {
                                frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
                            })
                        };
                        element.frames.push(AnchoredFrame {
                            origin: anchor.origin.clone(),
                            frame: ReferenceFrame {
                                origin_mm: position(frame, local_origin),
                                u: direction(local_u),
                                v: direction(local_v),
                            },
                        });
                    }
                }
            }
        }
        if let Some(identity) = &self.import_identity {
            let stamp = |origin: &Origin| Origin {
                source: self.feature,
                role: format!("{}/{}", identity.role, origin.role),
            };
            for e in &mut result.elements {
                e.origins = e.origins.iter().map(stamp).collect();
                e.boundary = e.boundary.iter().map(stamp).collect();
                for f in &mut e.frames {
                    f.origin = stamp(&f.origin);
                }
            }
        }
        result
            .validate()
            .map_err(|e| failure(GeometryErrorKind::InvalidResult, e))?;
        Ok(result)
    }
    pub(in crate::geometry) fn display_edges(
        &self,
        frame: PlaneFrame,
        deflection: f64,
    ) -> Result<Vec<DisplayEdge>, GeometryError> {
        let mut edges = self.inner.display_edges(frame, deflection)?;
        let mut outer = self.topology(frame, self.feature, 0)?;
        outer
            .elements
            .retain(|e| e.key.kind == TopologyKind::Edge && e.key.ordinal >= 15);
        edges.extend(crate::rust_display::edges(None, &outer, deflection)?);
        Ok(edges)
    }
}
