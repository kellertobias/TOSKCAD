//! FDM aid placement support: the native aid inspections of this exact
//! evaluation, and the picked surface point and outward normal the aid form
//! places contacts with. Review and display data only; the canonical aid
//! source stays in the document.
use super::{Hit, Point, Scene, cross, dot, sub};
use multicad_core::{document::EntityId, project_file::ContentHash, references::AnalyticGeometry};
use std::sync::Arc;

/// An exact tab contact the native inspection found near the authored one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactContact {
    pub point_mm: Point,
    pub angle_deg: f64,
}
/// One aid as the native inspection measured it.
#[derive(Clone, Debug, PartialEq)]
pub struct AidReview {
    pub feature: EntityId,
    pub part: EntityId,
    pub target: EntityId,
    pub touches_target: bool,
    pub target_distance_mm: f64,
    /// Finished Parts colliding with or inside the fit clearance of the aid.
    pub crowded: Vec<(EntityId, f64)>,
    pub issues: Vec<String>,
    /// The findings that make the placement invalid; Apply is refused.
    pub contact_issues: Vec<String>,
    pub bounds_mm: [Point; 2],
    pub exact_contact: Option<ExactContact>,
}
pub(super) struct AidReviews {
    source: ContentHash,
    result: Result<Vec<AidReview>, String>,
}
/// A picked point on a face with its outward unit normal. On a planar face
/// the point and normal are exact (the analytic plane); otherwise they come
/// from the display triangle under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfacePoint {
    pub point_mm: Point,
    pub normal: Point,
    pub exact_plane: bool,
}

fn unit(v: Point) -> Option<Point> {
    let n = dot(v, v).sqrt();
    (n.is_finite() && n > 1e-12).then(|| v.map(|c| c / n))
}
/// Squared distance from `p` to triangle `abc`.
fn triangle_distance(p: Point, [a, b, c]: [Point; 3]) -> f64 {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let closest = if d1 <= 0. && d2 <= 0. {
        a
    } else {
        let bp = sub(p, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        let cp = sub(p, c);
        let (d5, d6) = (dot(ab, cp), dot(ac, cp));
        let along = |from: Point, dir: Point, t: f64| std::array::from_fn(|i| from[i] + dir[i] * t);
        let (vc, vb, va) = (d1 * d4 - d3 * d2, d5 * d2 - d1 * d6, d3 * d6 - d5 * d4);
        if d3 >= 0. && d4 <= d3 {
            b
        } else if vc <= 0. && d1 >= 0. && d3 <= 0. {
            along(a, ab, d1 / (d1 - d3))
        } else if d6 >= 0. && d5 <= d6 {
            c
        } else if vb <= 0. && d2 >= 0. && d6 <= 0. {
            along(a, ac, d2 / (d2 - d6))
        } else if va <= 0. && d4 - d3 >= 0. && d5 - d6 >= 0. {
            along(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)))
        } else {
            let denominator = 1. / (va + vb + vc);
            let (v, w) = (vb * denominator, vc * denominator);
            std::array::from_fn(|i| a[i] + ab[i] * v + ac[i] * w)
        }
    };
    let d = sub(p, closest);
    dot(d, d)
}

impl Scene {
    /// Attach the native aid inspections of this exact source evaluation.
    pub fn with_fdm_reviews(
        mut self,
        source: ContentHash,
        result: Result<Vec<AidReview>, String>,
    ) -> Result<Self, String> {
        if let Ok(list) = &result {
            let mut seen = std::collections::BTreeSet::new();
            if list.len() > 4096
                || list.iter().any(|r| {
                    r.feature.is_nil()
                        || !seen.insert(r.feature)
                        || r.bounds_mm.iter().flatten().any(|v| !v.is_finite())
                })
            {
                return Err("FDM aid reviews contain an invalid identity or extent".into());
            }
        }
        self.fdm_reviews = Some(Arc::new(AidReviews { source, result }));
        Ok(self)
    }
    /// The native review of one aid feature, if this full-model scene's
    /// evaluation carried aid reviews for its own source. `Some(Err)` is a
    /// failed inspection; a missing aid in a reviewed scene is `None`.
    pub fn fdm_review(&self, feature: EntityId) -> Option<Result<&AidReview, &str>> {
        let reviews = self.fdm_reviews.as_ref()?;
        if !self.is_full_model() || self.model_evaluation.as_ref()?.source_hash != reviews.source {
            return None;
        }
        match &reviews.result {
            Ok(list) => list.iter().find(|r| r.feature == feature).map(Ok),
            Err(error) => Some(Err(error.as_str())),
        }
    }
    /// The picked surface point and outward normal of a face hit, in world
    /// millimeters.
    pub fn surface_at(&self, hit: &Hit) -> Option<SurfacePoint> {
        let instance = self.instances.iter().find(|i| i.id == hit.instance)?;
        let geometry = self.geometries.get(&instance.geometry)?;
        let t = instance.transform;
        let local: Point = std::array::from_fn(|i| {
            (0..3)
                .map(|j| t.rotation[j][i] * (hit.point_mm[j] - t.translation[j]))
                .sum()
        });
        let ordinal = hit.key.ordinal;
        let mut best: Option<(f64, Point)> = None;
        for (triangle, face) in geometry.triangle_faces.iter().enumerate() {
            if *face != ordinal {
                continue;
            }
            let corners = geometry.triangle(triangle);
            let d = triangle_distance(local, corners);
            if best.is_none_or(|(b, _)| d < b)
                && let Some(n) = unit(cross(
                    sub(corners[1], corners[0]),
                    sub(corners[2], corners[0]),
                ))
            {
                best = Some((d, n));
            }
        }
        let (_, mesh_normal) = best?;
        let plane = geometry
            .topology
            .iter()
            .find(|e| e.key == hit.key)
            .and_then(|e| match e.geometry {
                Some(AnalyticGeometry::Plane { frame }) => Some(frame),
                _ => None,
            });
        let (point, normal, exact_plane) =
            match plane.and_then(|f| unit(cross(f.u, f.v)).map(|n| (f.origin_mm, n))) {
                Some((origin, n)) => {
                    let n = if dot(n, mesh_normal) < 0. {
                        n.map(|c| -c)
                    } else {
                        n
                    };
                    let off = dot(sub(local, origin), n);
                    (std::array::from_fn(|i| local[i] - n[i] * off), n, true)
                }
                None => (local, mesh_normal, false),
            };
        Some(SurfacePoint {
            point_mm: t.point(point),
            normal: unit(t.vector(normal))?,
            exact_plane,
        })
    }
}
