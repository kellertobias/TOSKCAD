//! Occupied queries in a retained common rectilinear chart.
//!
//! A placed body retains local geometry and a construction frame. An exact
//! forward witness restores that representation instead of interpreting a
//! rounded inverse as a newly authored local displacement. Imported bodies
//! additionally retain the strict reader's bounded canonicalization contract;
//! only complete rectangular cavity contact can reuse that contract. This is
//! deliberately separate from arbitrary world-space planar clipping.
use super::*;

impl Solid {
    pub(crate) fn rust_factored_orthogonal_overlap(
        &self,
        other: &Self,
    ) -> Option<Result<f64, GeometryError>> {
        self.factored_orthogonal_overlap(other)
            .or_else(|| other.factored_orthogonal_overlap(self))
    }

    fn factored_orthogonal_overlap(&self, other: &Self) -> Option<Result<f64, GeometryError>> {
        let (stock, frame) = self.orthogonal_body()?;
        if stock.has_curved_cuts() {
            return None;
        }
        let (probe, probe_frame) = match other.drawing_rectilinear_cells() {
            Ok(Some(value)) => value,
            Ok(None) => return None,
            Err(error) => return Some(Err(error)),
        };
        if probe.len() != 1 {
            return None;
        }
        // Cardinal placement must not hide a retained dimension before an
        // imported contact certificate sees the rounded world corners.
        if cardinal(probe_frame) {
            let probe_axes = [probe_frame.u, probe_frame.v, probe_frame.normal()];
            for (local_axis, axis) in probe_axes.iter().enumerate() {
                for (world_axis, &direction) in axis.iter().enumerate() {
                    if direction == 0. {
                        continue;
                    }
                    for endpoint in [probe[0].min[local_axis], probe[0].max[local_axis]] {
                        if exact_sum(probe_frame.origin[world_axis], direction * endpoint).is_none()
                        {
                            return Some(Err(endpoint_precision()));
                        }
                    }
                }
            }
        }
        if imported_cavity_contact(stock, frame, probe[0], probe_frame) {
            return Some(Ok(0.));
        }
        // Inverting a placed origin needlessly loses a literal grid coordinate.
        // Recover only an exact forward witness, never a nearest grid point.
        // Multiple witnesses mean world arithmetic erased a distinction.
        // Both associations occur in authored PrimitiveFrame origins and the
        // retained topology placement; neither permits an epsilon fit.
        let axes = [frame.u, frame.v, frame.normal()];
        let delta = subtract(probe_frame.origin, frame.origin);
        let cofactors = [
            cross(axes[1], axes[2]),
            cross(axes[2], axes[0]),
            cross(axes[0], axes[1]),
        ];
        let determinant = dot(axes[0], cofactors[0]);
        if !determinant.is_finite() || determinant <= 0. {
            return None;
        }
        let estimate = cofactors.map(|axis| dot(axis, delta) / determinant);
        let corners: [Point; 8] = std::array::from_fn(|bits| {
            transform_point(
                probe_frame,
                std::array::from_fn(|i| {
                    if bits & (1 << i) == 0 {
                        probe[0].min[i]
                    } else {
                        probe[0].max[i]
                    }
                }),
            )
        });
        let magnitude = evaluation_magnitude(probe_frame, probe[0])
            .max(frame.origin.into_iter().map(f64::abs).fold(1., f64::max));
        let local = corners.map(|point| {
            cofactors.map(|axis| dot(axis, subtract(point, frame.origin)) / determinant)
        });
        let radius = 64. * f64::EPSILON * magnitude;
        // Strict separation of an outward error enclosure proves empty
        // material without reconstructing any grid coordinate. It covers
        // genuinely interior probes as well as ordinary external clearance.
        let enclosure = Box3 {
            min: std::array::from_fn(|i| {
                local.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min) - radius
            }),
            max: std::array::from_fn(|i| {
                local.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max) + radius
            }),
        };
        if stock.geometry.cells().iter().all(|cell| {
            (0..3).any(|i| enclosure.max[i] < cell.min[i] || enclosure.min[i] > cell.max[i])
        }) {
            return Some(Ok(0.));
        }
        if frame.u != probe_frame.u || frame.v != probe_frame.v {
            return None;
        }
        let candidates: [Vec<f64>; 3] = std::array::from_fn(|axis| {
            let mut values: Vec<_> = stock
                .geometry
                .cells()
                .iter()
                .flat_map(|cell| [cell.min[axis], cell.max[axis]])
                .filter(|value| (*value - estimate[axis]).abs() <= radius)
                .collect();
            values.sort_unstable_by(f64::total_cmp);
            values.dedup_by(|a, b| *a == *b);
            values
        });
        let mut witness = None;
        let mut work = 0;
        for &x in &candidates[0] {
            for &y in &candidates[1] {
                for &z in &candidates[2] {
                    work += 1;
                    if work > orthogonal::MAX_CELLS {
                        return None;
                    }
                    let point = [x, y, z];
                    let literal = std::array::from_fn(|i| {
                        frame.origin[i] + frame.u[i] * x + frame.v[i] * y + frame.normal()[i] * z
                    });
                    if (transform_point(frame, point) == probe_frame.origin
                        || literal == probe_frame.origin)
                        && witness.replace(point).is_some()
                    {
                        return None;
                    }
                }
            }
        }
        let origin = witness?;
        let min = std::array::from_fn::<_, 3, _>(|i| exact_sum(origin[i], probe[0].min[i]));
        let max = std::array::from_fn::<_, 3, _>(|i| exact_sum(origin[i], probe[0].max[i]));
        if min.iter().chain(&max).any(Option::is_none) {
            return Some(Err(endpoint_precision()));
        }
        let probe = Box3 {
            min: min.map(Option::unwrap),
            max: max.map(Option::unwrap),
        };
        Some(
            orthogonal::intersection_volume(stock.geometry.cells(), &[probe])
                .map_err(operation_error),
        )
    }
}

// The STEP reader already declared a bounded canonicalization. Only a full
// rectangular inward shell/probe correspondence may reuse that certificate;
// arbitrary near contacts continue through the strict planar query.
fn imported_cavity_contact(
    stock: &OrthogonalBody,
    frame: PlaneFrame,
    probe: Box3,
    probe_frame: PlaneFrame,
) -> bool {
    let Some(reconstruction) = stock.imported_roundoff_mm else {
        return false;
    };
    if stock.boundary.shell_count < 2 {
        return false;
    }
    let corners: [Point; 8] = std::array::from_fn(|i| {
        transform_point(
            probe_frame,
            std::array::from_fn(|axis| {
                if i & (1 << axis) == 0 {
                    probe.min[axis]
                } else {
                    probe.max[axis]
                }
            }),
        )
    });
    let scale = evaluation_magnitude(probe_frame, probe)
        .max(evaluation_magnitude(frame, stock.geometry.bounds()));
    // Proper placement transports the original reconstruction error. The
    // second term bounds current affine point evaluation, not user tolerance.
    let error = reconstruction * (1. + 1e-9) + 16. * f64::EPSILON * scale;
    let extent = (0..3)
        .map(|i| probe.max[i] - probe.min[i])
        .fold(0., f64::max);
    let axes = [probe_frame.u, probe_frame.v, probe_frame.normal()];
    let mut matches: [Vec<&BoundaryFace>; 6] = std::array::from_fn(|_| Vec::new());
    for face in &stock.boundary.faces {
        let [ring] = face.rings.as_slice() else {
            continue;
        };
        if ring.len() != 4 {
            continue;
        }
        let mut indices = BTreeSet::new();
        for &point in ring {
            let world = transform_point(frame, point);
            let hits: Vec<_> = corners
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    let delta = subtract(world, **p);
                    dot(delta, delta).sqrt() <= error
                })
                .map(|(i, _)| i)
                .collect();
            if hits.len() != 1 || !indices.insert(hits[0]) {
                break;
            }
        }
        if indices.len() != 4 {
            continue;
        }
        let normal = transform_vector(frame, face.normal);
        for axis in 0..3 {
            for side in 0..2 {
                if indices
                    .iter()
                    .all(|&i| usize::from(i & (1 << axis) != 0) == side)
                {
                    // A cavity's material normal points into the empty probe.
                    let expected = axes[axis].map(|x| if side == 0 { x } else { -x });
                    let delta = subtract(normal, expected);
                    if dot(delta, delta).sqrt() * extent <= error {
                        matches[2 * axis + side].push(face);
                    }
                }
            }
        }
    }
    if matches.iter().any(|faces| faces.len() != 1) {
        return false;
    }
    // All six physical faces must share the same eight exact source vertices.
    let vertices: BTreeSet<_> = matches
        .iter()
        .flat_map(|faces| faces[0].rings[0].iter().copied().map(key))
        .collect();
    vertices.len() == 8
        && vertices.iter().all(|point| {
            matches
                .iter()
                .filter(|faces| faces[0].rings[0].iter().any(|&p| key(p) == *point))
                .count()
                == 3
        })
}

// Bounds absolute evaluation terms, including cancellation at distant anchors.
fn evaluation_magnitude(frame: PlaneFrame, bounds: Box3) -> f64 {
    let axes = [frame.u, frame.v, frame.normal()];
    (0..3)
        .map(|i| {
            frame.origin[i].abs()
                + (0..3)
                    .map(|j| axes[j][i].abs() * bounds.min[j].abs().max(bounds.max[j].abs()))
                    .sum::<f64>()
        })
        .fold(1., f64::max)
}

fn exact_sum(a: f64, b: f64) -> Option<f64> {
    let sum = a + b;
    let remainder = sum - a;
    let error = (a - (sum - remainder)) + (b - remainder);
    (sum.is_finite() && error == 0.).then_some(sum)
}
fn endpoint_precision() -> GeometryError {
    invalid("Factored rectilinear endpoint addition lost a retained coordinate")
}
