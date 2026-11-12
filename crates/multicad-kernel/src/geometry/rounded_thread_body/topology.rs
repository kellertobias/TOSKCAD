use super::*;
impl RoundedThreadBody {
    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let cap = |i| self.origin(CAPS[i]);
        let band = |i| self.origin(format!("thread/rounded/{}", BANDS[i]));
        let rail = |i| self.origin(format!("thread/rounded/rail/{}", RAILS[i]));
        let rim = |e, i| self.origin(format!("{}/rounded/rim/{}", CAPS[e], BANDS[i]));
        let mut boundaries = vec![BTreeSet::new(); 7];
        for i in 0..5 {
            let next = (i + 1) % 5;
            boundaries[i + 2].extend([rail(i), rail(next)]);
            for e in 0..2 {
                let r = rim(e, i);
                boundaries[e].insert(r.clone());
                boundaries[i + 2].insert(r);
            }
        }
        let mut elements = Vec::with_capacity(32);
        for (i, boundary) in boundaries.into_iter().enumerate() {
            let origin = if i < 2 { cap(i) } else { band(i - 2) };
            let plane = (i < 2).then(|| self.cap_frame(frame, i));
            let mut origins = BTreeSet::from([origin.clone()]);
            if i == 4 {
                origins.insert(self.origin("thread.root"));
            }
            elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: i as u32,
                },
                origins,
                boundary,
                frames: plane
                    .into_iter()
                    .map(|f| AnchoredFrame {
                        origin: origin.clone(),
                        frame: f,
                    })
                    .collect(),
                geometry: plane.map(|frame| AnalyticGeometry::Plane { frame }),
            });
        }
        for i in 0..15 {
            let role = i % 5;
            let end = i / 5;
            let origin = if end == 0 {
                rail(role)
            } else {
                rim(end - 1, role)
            };
            let boundary = if end == 0 {
                BTreeSet::from([band(role), band((role + 4) % 5)])
            } else {
                BTreeSet::from([cap(end - 1), band(role)])
            };
            elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Edge,
                    ordinal: i as u32,
                },
                origins: BTreeSet::from([origin]),
                boundary,
                frames: vec![],
                geometry: None,
            });
        }
        let breaks = self.geometry.profile_breaks();
        let length = self.geometry.spec().length_mm;
        for (end, cap_role) in CAPS.iter().enumerate() {
            for (i, &phase) in breaks[..5].iter().enumerate() {
                elements.push(TopologyElement {
                    key: TopologyKey {
                        kind: TopologyKind::Vertex,
                        ordinal: (end * 5 + i) as u32,
                    },
                    origins: BTreeSet::from([
                        self.origin(format!("{}/rounded/corner/{}", cap_role, RAILS[i]))
                    ]),
                    boundary: BTreeSet::from([cap(end), band(i), band((i + 4) % 5)]),
                    frames: vec![],
                    geometry: Some(AnalyticGeometry::Point {
                        position_mm: position(frame, self.point(phase, end as f64 * length)),
                    }),
                });
            }
        }
        if let Some(identity) = &self.import_identity {
            let stamp = |o: &Origin| Origin {
                source: self.feature,
                role: format!("{}/{}", identity.role, o.role),
            };
            for e in &mut elements {
                e.origins = e.origins.iter().map(stamp).collect();
                e.boundary = e.boundary.iter().map(stamp).collect();
                for f in &mut e.frames {
                    f.origin = stamp(&f.origin);
                }
            }
        }
        let snapshot = TopologySnapshot {
            owner,
            revision,
            elements,
        };
        snapshot
            .validate()
            .map_err(|e| failure(GeometryErrorKind::InvalidResult, e))?;
        Ok(snapshot)
    }
}
