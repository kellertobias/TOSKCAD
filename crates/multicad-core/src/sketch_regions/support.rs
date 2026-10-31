use super::*;
use crate::{manufacturing::PlanarEdge, sketch::Geometry};

impl SketchSupport {
    /// Combine solved ordinary sketch strokes with the exact finite support.
    /// Native face holes and areas outside the face are never material cells.
    pub fn regions(
        &self,
        geometry: &[Geometry],
        construction: &BTreeSet<EntityId>,
        queries: &dyn RegionQueries,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Extraction, QueryError> {
        let tolerance = queries.tolerance_mm();
        self.contours
            .validate(tolerance)
            .map_err(|message| QueryError { message })?;
        if self.edges.len() != self.contours.loops.len() {
            return Err(QueryError {
                message: "Supporting face edge receipt differs from its wires".into(),
            });
        }
        let mut loops = vec![];
        for (wire, ids) in self.contours.loops.iter().zip(&self.edges) {
            if ids.len() != wire.edges.len() {
                return Err(QueryError {
                    message: "Supporting wire edge identities are incomplete".into(),
                });
            }
            let edges: Vec<_> = wire
                .edges
                .iter()
                .zip(ids)
                .map(|(curve, id)| Edge {
                    authored: *id,
                    reversed: false,
                    fragment: None,
                    curve: match *curve {
                        PlanarEdge::Line { a, b } => ExactCurve::Line { a, b },
                        PlanarEdge::Arc {
                            a,
                            center,
                            sweep_rad,
                            ..
                        } if (sweep_rad.abs() - TAU).abs() < 1e-9 => ExactCurve::Circle {
                            center,
                            radius_mm: distance(a, center),
                        },
                        PlanarEdge::Arc {
                            a,
                            b,
                            center,
                            sweep_rad,
                        } => ExactCurve::Arc {
                            a,
                            b,
                            center,
                            clockwise: sweep_rad < 0.,
                        },
                    },
                })
                .collect();
            loops.push((wire.outer, edges));
        }
        let mut edges: Vec<_> = loops
            .iter()
            .flat_map(|(_, edges)| edges.iter().cloned())
            .collect();
        let mut authored = vec![];
        for g in geometry.iter().filter(|g| !construction.contains(&g.id())) {
            let curve = match g {
                Geometry::Line { a, b, .. } => ExactCurve::Line { a: *a, b: *b },
                Geometry::Arc {
                    a,
                    b,
                    center,
                    clockwise,
                    ..
                } => ExactCurve::Arc {
                    a: *a,
                    b: *b,
                    center: *center,
                    clockwise: *clockwise,
                },
                Geometry::Circle {
                    center, radius_mm, ..
                } => ExactCurve::Circle {
                    center: *center,
                    radius_mm: *radius_mm,
                },
                Geometry::BSpline { poles, basis, .. } => ExactCurve::BSpline {
                    poles: poles.clone(),
                    basis: basis.clone(),
                },
                Geometry::Rectangle { .. } => {
                    return Err(QueryError {
                        message: "Region evaluation requires individual exact sketch curves".into(),
                    });
                }
            };
            authored.push(Edge {
                authored: g.id(),
                reversed: false,
                fragment: None,
                curve,
            });
        }
        // Existing face sketches may already contain a copied/projected whole
        // perimeter edge. Keep its authored identity instead of adding the
        // same finite edge twice. Partial overlaps and authored duplicates
        // still require repair; no stroke is trimmed or silently discarded.
        edges.retain(|support| {
            !authored
                .iter()
                .any(|edge| same_curve(&support.curve, &edge.curve, tolerance))
        });
        edges.extend(authored);
        let mut result = extract_cells(&edges, tolerance, queries, interrupt)?;
        let mut retained = vec![];
        for region in result.regions {
            check_interrupt(interrupt)?;
            let point = arrangement::interior_point(&region, queries, interrupt)?;
            let mut material = false;
            for (outer, edges) in &loops {
                let inside =
                    queries.classify_boundary(edges, point, tolerance)? == PointLocation::Inside;
                if *outer {
                    material |= inside;
                } else if inside {
                    material = false;
                    break;
                }
            }
            if material {
                retained.push(region);
            }
        }
        result.regions = retained;
        let mut intents = BTreeSet::new();
        if result.regions.iter().any(|r| !intents.insert(r.intent())) {
            return Err(QueryError {
                message:
                    "Several material cells share a boundary identity; repair the ambiguous profile"
                        .into(),
            });
        }
        Ok(result)
    }
}

fn same_curve(a: &ExactCurve, b: &ExactCurve, tolerance: f64) -> bool {
    let same = |a: &ExactCurve, b: &ExactCurve| match (a, b) {
        (ExactCurve::Line { a, b }, ExactCurve::Line { a: c, b: d }) => {
            distance(*a, *c) <= tolerance && distance(*b, *d) <= tolerance
        }
        (
            ExactCurve::Circle { center, radius_mm },
            ExactCurve::Circle {
                center: c,
                radius_mm: r,
            },
        ) => distance(*center, *c) <= tolerance && (radius_mm - r).abs() <= tolerance,
        (
            ExactCurve::Arc {
                center,
                a,
                b,
                clockwise,
            },
            ExactCurve::Arc {
                center: c,
                a: d,
                b: e,
                clockwise: w,
            },
        ) => {
            clockwise == w
                && distance(*center, *c) <= tolerance
                && distance(*a, *d) <= tolerance
                && distance(*b, *e) <= tolerance
        }
        _ => false,
    };
    same(a, b) || same(a, &b.reversed())
}
