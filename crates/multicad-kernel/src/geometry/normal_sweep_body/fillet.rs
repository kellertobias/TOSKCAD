//! One full-length convex box-edge blend as an exact circular normal sweep.
use super::*;
use crate::TreatmentEdge;
use multicad_core::references::{TopologyElement, TopologyKey, TopologyKind};

fn unsupported() -> GeometryError {
    error(
        "fillet",
        GeometryErrorKind::Unsupported,
        "Rust fillet requires one complete straight edge of a world-aligned box",
    )
}

impl Solid {
    pub(crate) fn rust_fillet_box_edge(
        &self,
        edge: &TreatmentEdge,
        radius: f64,
        feature: EntityId,
        policy: GeometryPolicy,
        topology: &TopologySnapshot,
    ) -> Result<Self, GeometryError> {
        if self.policy != policy || policy.boolean_fuzzy_mm != 0. {
            return Err(unsupported());
        }
        let cells = self.orthogonal_cells()?.ok_or_else(unsupported)?;
        let [stock] = cells.as_slice() else {
            return Err(unsupported());
        };
        let Some(AnalyticGeometry::Line { a_mm, b_mm }) = topology
            .elements
            .iter()
            .find(|element| element.key == edge.key)
            .and_then(|element| element.geometry)
        else {
            return Err(unsupported());
        };
        let axes: Vec<_> = (0..3).filter(|&axis| a_mm[axis] != b_mm[axis]).collect();
        let [along] = axes.as_slice() else {
            return Err(unsupported());
        };
        let along = *along;
        if a_mm[along].min(b_mm[along]) != stock.min[along]
            || a_mm[along].max(b_mm[along]) != stock.max[along]
        {
            return Err(unsupported());
        }
        let [across, up] = [(along + 1) % 3, (along + 2) % 3];
        let inward = |axis| {
            if a_mm[axis] == stock.min[axis] {
                Ok(1.)
            } else if a_mm[axis] == stock.max[axis] {
                Ok(-1.)
            } else {
                Err(unsupported())
            }
        };
        let (across_sign, up_sign) = (inward(across)?, inward(up)?);
        let mut frame = PlaneFrame {
            origin: a_mm,
            u: [0.; 3],
            v: [0.; 3],
        };
        frame.u[across] = across_sign;
        frame.v[up] = up_sign;
        frame.origin[along] = if across_sign == up_sign {
            stock.min[along]
        } else {
            stock.max[along]
        };
        let width = stock.max[across] - stock.min[across];
        let depth = stock.max[up] - stock.min[up];
        let height = stock.max[along] - stock.min[along];
        policy.length(width - radius)?;
        policy.length(depth - radius)?;
        let points = [
            [radius, 0.],
            [width, 0.],
            [width, depth],
            [0., depth],
            [0., radius],
        ];
        let mut outer: Vec<_> = points
            .windows(2)
            .map(|ends| CurvedEdge2d::Line {
                a: ends[0],
                b: ends[1],
            })
            .collect();
        outer.push(CurvedEdge2d::Arc {
            center: [radius; 2],
            a: points[4],
            b: points[0],
            clockwise: false,
        });
        let mut origins: Vec<_> = (0..4)
            .map(|i| Origin {
                source: feature,
                role: format!("fillet.side.{i}"),
            })
            .collect();
        origins.push(edge.origin.clone());
        let profile = CurvedProfile {
            frame,
            outer,
            holes: Vec::new(),
        };
        let mut result =
            Self::rust_extrude_normal_profile_named(&profile, height, feature, &origins, policy)?;
        let SolidBody::NormalSweep(body, _) = &mut result.body else {
            unreachable!("constructed normal sweep");
        };
        let body = Rc::get_mut(body).expect("new normal sweep");
        let rounded = body.topology(frame, feature, 0);
        body.stock_history = Some(Rc::new(StockHistory::capture(
            topology,
            &rounded,
            frame,
            policy.linear_tolerance_mm,
        )?));
        result
            .topology_snapshot(feature, 0)?
            .validate()
            .map_err(|message| error("fillet", GeometryErrorKind::InvalidResult, message))?;
        Ok(result)
    }
}

pub(super) struct StockHistory {
    matches: Vec<(TopologyKey, BTreeSet<Origin>, Vec<AnchoredFrame>)>,
}

impl StockHistory {
    fn capture(
        stock: &TopologySnapshot,
        result: &TopologySnapshot,
        placement: PlaneFrame,
        tolerance: f64,
    ) -> Result<Self, GeometryError> {
        let mut matches = Vec::new();
        for element in &result.elements {
            let Some(geometry) = element.geometry else {
                continue;
            };
            let mut candidates = stock.elements.iter().filter(|old| {
                old.geometry
                    .is_some_and(|old| survives(old, geometry, tolerance))
            });
            let Some(source) = candidates.next() else {
                if element.key.kind == TopologyKind::Face {
                    return Err(unsupported());
                }
                continue;
            };
            if candidates.next().is_some() {
                return Err(unsupported());
            }
            matches.push((
                element.key,
                source.origins.clone(),
                source
                    .frames
                    .iter()
                    .map(|anchored| AnchoredFrame {
                        origin: anchored.origin.clone(),
                        frame: local_frame(placement, anchored.frame),
                    })
                    .collect(),
            ));
        }
        Ok(Self { matches })
    }

    pub(super) fn add_face_origins(&self, faces: &mut [Vec<Origin>]) {
        for (key, origins, _) in &self.matches {
            if key.kind == TopologyKind::Face {
                faces[key.ordinal as usize].extend(origins.iter().cloned());
            }
        }
    }

    pub(super) fn restore(&self, placement: PlaneFrame, elements: &mut [TopologyElement]) {
        for (key, origins, frames) in &self.matches {
            let element = elements
                .iter_mut()
                .find(|element| element.key == *key)
                .expect("normal sweep stock history");
            if key.kind == TopologyKind::Face {
                element
                    .frames
                    .extend(frames.iter().map(|anchored| AnchoredFrame {
                        origin: anchored.origin.clone(),
                        frame: placed_frame(placement, anchored.frame),
                    }));
            } else {
                element.origins.extend(origins.iter().cloned());
            }
        }
    }
}

fn vector_sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn local_frame(placement: PlaneFrame, world: ReferenceFrame) -> ReferenceFrame {
    let axes = [placement.u, placement.v, placement.normal()];
    let local = |vector| axes.map(|axis| dot(axis, vector));
    ReferenceFrame {
        origin_mm: local(vector_sub(world.origin_mm, placement.origin)),
        u: local(world.u),
        v: local(world.v),
    }
}

fn placed_frame(placement: PlaneFrame, local: ReferenceFrame) -> ReferenceFrame {
    let linear = PlaneFrame {
        origin: [0.; 3],
        ..placement
    };
    ReferenceFrame {
        origin_mm: position(placement, local.origin_mm),
        u: position(linear, local.u),
        v: position(linear, local.v),
    }
}

fn survives(old: AnalyticGeometry, result: AnalyticGeometry, tolerance: f64) -> bool {
    match (old, result) {
        (AnalyticGeometry::Plane { frame: a }, AnalyticGeometry::Plane { frame: b }) => {
            dot(a.normal(), b.normal()) > 1. - 1e-10
                && dot(a.normal(), vector_sub(a.origin_mm, b.origin_mm)).abs() <= tolerance
        }
        (AnalyticGeometry::Line { a_mm, b_mm }, AnalyticGeometry::Line { a_mm: a, b_mm: b }) => {
            let delta = vector_sub(b_mm, a_mm);
            let length = dot(delta, delta).sqrt();
            let unit = delta.map(|coordinate| coordinate / length);
            [a, b].into_iter().all(|point| {
                let offset = vector_sub(point, a_mm);
                let along = dot(unit, offset);
                let across = vector_sub(offset, unit.map(|coordinate| coordinate * along));
                (-tolerance..=length + tolerance).contains(&along)
                    && dot(across, across) <= tolerance * tolerance
            })
        }
        (
            AnalyticGeometry::Point { position_mm: a },
            AnalyticGeometry::Point { position_mm: b },
        ) => {
            let delta = vector_sub(a, b);
            dot(delta, delta) <= tolerance * tolerance
        }
        _ => false,
    }
}
