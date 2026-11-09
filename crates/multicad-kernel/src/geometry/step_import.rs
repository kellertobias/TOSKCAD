//! Rebuild an independently verified STEP extrusion as exact Rust geometry.
use super::*;
use multicad_core::{project_file::ContentHash, references::AnalyticGeometry};
use multicad_rust_kernel::step_read::{
    self, CircleEdge, ImportProfile, ImportSolid, ReadError, Readback,
};

type Point = [f64; 3];
type Segment = [Point; 2];
const IMPORT: &str = "STEP import";

fn invalid(message: impl Into<String>) -> GeometryError {
    error(IMPORT, GeometryErrorKind::InvalidResult, message)
}

fn convert(value: f64, tolerance: f64) -> Result<Rational, GeometryError> {
    // Select the first continued-fraction convergent within the error
    // budget. This avoids treating floating-point projection noise as
    // authored precision and preserves ordinary fractional dimensions.
    let mut remainder = value.abs();
    let mut previous = [0_i128, 1];
    let mut current = [1_i128, 0];
    for _ in 0..32 {
        if !remainder.is_finite() || remainder > i64::MAX as f64 {
            break;
        }
        let whole = remainder.floor();
        let next = std::array::from_fn(|i| whole as i128 * current[i] + previous[i]);
        let [Ok(n), Ok(d)] = next.map(i64::try_from) else {
            break;
        };
        let Ok(candidate) = Rational::new(if value.is_sign_negative() { -n } else { n }, d) else {
            break;
        };
        if (candidate.as_f64() - value).abs() <= tolerance / 32. {
            return Ok(candidate);
        }
        previous = current;
        current = next;
        remainder = 1. / (remainder - whole);
    }
    Err(invalid(
        "STEP dimension cannot be represented within the geometry tolerance",
    ))
}

fn feature_origin(
    feature: EntityId,
    kind: &str,
    coordinates: impl IntoIterator<Item = f64>,
) -> Origin {
    let bytes: Vec<_> = coordinates
        .into_iter()
        .flat_map(|value| {
            let value = if value == 0. { 0. } else { value };
            value.to_le_bytes()
        })
        .collect();
    let hash = ContentHash::of(&bytes);
    Origin {
        source: feature,
        role: format!("step.{kind}/{hash}"),
    }
}

impl Solid {
    /// Import a verified, supported, single analytic STEP solid in millimeters.
    pub fn import_rust_step(
        input: &str,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        let tolerance = policy.linear_tolerance_mm;
        let (readback, imported) = step_read::read_import_solid(input, tolerance).map_err(|cause| {
            let (kind, message) = match cause {
                ReadError::Invalid => (GeometryErrorKind::InvalidResult, "Invalid STEP geometry"),
                ReadError::Limit => (GeometryErrorKind::Limit, "STEP exceeds the Rust import budget"),
                ReadError::Unsupported => (
                    GeometryErrorKind::Unsupported,
                    "Rust STEP import supports one millimeter extrusion with straight or circular trims",
                ),
            };
            error(IMPORT, kind, message)
        })?;
        let solid = Self::from_imported_extrusion(imported, feature, policy)?;
        verify(&solid, &readback, feature, tolerance)?;
        Ok(solid)
    }

    fn from_imported_extrusion(
        imported: ImportSolid,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let ImportSolid {
            origin_mm,
            u,
            v,
            height_mm,
            profile,
        } = imported;
        let frame = PlaneFrame {
            origin: origin_mm,
            u,
            v,
        };
        frame.checked(policy)?;
        let dimension = |value| convert(value, policy.linear_tolerance_mm);
        let height = dimension(height_mm)?;
        match profile {
            ImportProfile::Cylinder { radius_mm } => {
                let zero = Rational::new(0, 1).expect("zero is representable");
                let circle = Circle::new([zero; 2], dimension(radius_mm)?)
                    .map_err(|cause| invalid(format!("Invalid STEP circle: {cause:?}")))?;
                let cylinder = AxialCylinder::new(feature.uuid_bits(), circle, zero, height)
                    .map_err(|cause| invalid(format!("Invalid STEP cylinder: {cause:?}")))?;
                let source = Self::from_axial_cylinder(cylinder, PlaneFrame::default(), policy)?;
                let normal = frame.normal();
                source.transformed(RigidTransform {
                    rotation: std::array::from_fn(|i| [u[i], v[i], normal[i]]),
                    translation_mm: origin_mm,
                })
            }
            ImportProfile::Polygon {
                rings,
                circular_holes,
            } => {
                let exact_rings = rings
                    .iter()
                    .map(|ring| {
                        ring.iter()
                            .map(|&[x, y]| Ok([dimension(x)?, dimension(y)?]))
                            .collect()
                    })
                    .collect::<Result<Vec<_>, GeometryError>>()?;
                let holes = circular_holes
                    .iter()
                    .map(|&([x, y], radius)| {
                        Circle::new([dimension(x)?, dimension(y)?], dimension(radius)?)
                            .map_err(|cause| invalid(format!("Invalid STEP hole: {cause:?}")))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let prism = PolygonPrism::with_circular_holes(exact_rings, holes, height)
                    .map_err(|cause| invalid(format!("Invalid STEP polygon: {cause:?}")))?;
                let mut origins: Vec<_> = rings
                    .iter()
                    .flat_map(|ring| {
                        ring.iter()
                            .zip(ring.iter().cycle().skip(1))
                            .map(|(&a, &b)| {
                                let mut points = [frame.point(a), frame.point(b)];
                                points.sort_unstable_by(|a, b| {
                                    a.iter()
                                        .zip(b)
                                        .find_map(|(a, b)| (a != b).then(|| a.total_cmp(b)))
                                        .unwrap_or(std::cmp::Ordering::Equal)
                                });
                                feature_origin(feature, "edge", points.into_iter().flatten())
                            })
                    })
                    .collect();
                origins.extend(circular_holes.iter().map(|&(center, radius)| {
                    feature_origin(
                        feature,
                        "hole",
                        frame.point(center).into_iter().chain([radius]),
                    )
                }));
                let reversed = vec![false; origins.len()];
                Self::from_polygon_prism(prism, frame, feature, &origins, &reversed, policy)
            }
        }
    }
}

fn max_error(a: Point, b: Point) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0., f64::max)
}

fn match_circles(expected: &[CircleEdge], actual: &[CircleEdge], tolerance: f64) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    let mut matched = vec![false; actual.len()];
    expected.iter().all(|rim| {
        let hit = actual.iter().enumerate().find(|(i, candidate)| {
            !matched[*i]
                && max_error(rim.center_mm, candidate.center_mm) <= tolerance
                && (rim.radius_mm - candidate.radius_mm).abs() <= tolerance
                && cross(rim.normal, candidate.normal)
                    .iter()
                    .all(|v| v.abs() * rim.radius_mm <= tolerance)
        });
        if let Some((i, _)) = hit {
            matched[i] = true;
            true
        } else {
            false
        }
    })
}

fn match_lines(expected: &[Segment], actual: &[Segment], tolerance: f64) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    let mut indexed: Vec<_> = actual
        .iter()
        .enumerate()
        .flat_map(|(i, &[a, b])| [(i, [a, b]), (i, [b, a])])
        .collect();
    indexed.sort_unstable_by(|(_, a), (_, b)| a[0][0].total_cmp(&b[0][0]));
    let mut matched = vec![false; actual.len()];
    expected.iter().all(|&[a, b]| {
        let start = indexed.partition_point(|(_, points)| points[0][0] < a[0] - tolerance);
        let hit = indexed[start..]
            .iter()
            .take_while(|(_, points)| points[0][0] <= a[0] + tolerance)
            .filter_map(|&(index, [p, q])| {
                let error = max_error(a, p).max(max_error(b, q));
                (!matched[index] && error <= tolerance).then_some((index, error))
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b));
        if let Some((index, _)) = hit {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn verify(
    solid: &Solid,
    readback: &Readback,
    feature: EntityId,
    tolerance: f64,
) -> Result<(), GeometryError> {
    let bounds = solid.bounds()?;
    let [x, y, z] = std::array::from_fn(|i| bounds.max[i] - bounds.min[i]);
    let volume_tolerance =
        (readback.volume_mm3 * 1e-8).max(tolerance * 2. * (x * y + y * z + z * x));
    if max_error(bounds.min, readback.bounds_mm[0]) > tolerance * 2.
        || max_error(bounds.max, readback.bounds_mm[1]) > tolerance * 2.
        || (solid.volume_mm3()? - readback.volume_mm3).abs() > volume_tolerance
    {
        return Err(invalid(
            "Rebuilt STEP extrusion differs from the imported bounds or volume",
        ));
    }
    let mut lines = Vec::new();
    let mut circles = Vec::new();
    let mut faces = 0;
    for element in solid.topology_snapshot(feature, 0)?.elements {
        use multicad_core::references::TopologyKind;
        match (element.key.kind, element.geometry) {
            (TopologyKind::Face, _) => faces += 1,
            (TopologyKind::Edge, Some(AnalyticGeometry::Line { a_mm, b_mm })) => {
                lines.push([a_mm, b_mm])
            }
            (
                TopologyKind::Edge,
                Some(AnalyticGeometry::CircularArc {
                    frame,
                    radius_mm,
                    sweep_rad,
                    ..
                }),
            ) if (sweep_rad.abs() - std::f64::consts::TAU).abs() < 1e-9 => {
                circles.push(CircleEdge {
                    center_mm: frame.origin_mm,
                    normal: frame.normal(),
                    radius_mm,
                });
            }
            (TopologyKind::Edge, _) => {
                return Err(invalid("Rebuilt STEP extrusion has an unexpected edge"));
            }
            (TopologyKind::Vertex, _) => {}
        }
    }
    if faces != readback.face_count
        || !match_lines(&readback.edges_mm, &lines, tolerance * 2.)
        || !match_circles(&readback.circles, &circles, tolerance * 2.)
    {
        return Err(invalid(
            "Rebuilt STEP extrusion differs from the imported analytic topology",
        ));
    }
    Ok(())
}
