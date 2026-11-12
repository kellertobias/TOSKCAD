//! Rebuild independently verified STEP boundaries as analytic Rust solids.
use super::*;
use crate::{CurvedEdge2d, CurvedProfile};
use multicad_core::{
    project_file::ContentHash,
    references::{AnalyticGeometry, TopologyElement, TopologyKind},
};
use multicad_rust_kernel::normal_sweep::Edge as SweepEdge;
use multicad_rust_kernel::orthogonal_pockets::OrthogonalPocket;
use multicad_rust_kernel::step_read::{
    self, ArcEdge, CircleEdge, ImportProfile, ImportSolid, ReadError, Readback, SemicircleEdge,
};
use multicad_rust_kernel::trim_curve::{Ellipse, EllipseArc};
use multicad_rust_kernel::{Error as RationalError, polygon::PolygonError};
use std::f64::consts::{PI, TAU};
mod compound;
mod involute;

type Point = [f64; 3];
type Segment = [Point; 2];
const IMPORT: &str = "STEP import";

fn invalid(message: impl Into<String>) -> GeometryError {
    error(IMPORT, GeometryErrorKind::InvalidResult, message)
}

fn convert(value: f64, tolerance: f64) -> Result<Rational, GeometryError> {
    rational_coordinate(value, tolerance).ok_or_else(|| {
        invalid("STEP dimension cannot be represented within the geometry tolerance")
    })
}

fn import_cylinder(
    radius_mm: f64,
    height_mm: f64,
    frame: PlaneFrame,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    let height = convert(height_mm, policy.linear_tolerance_mm)?;
    let zero = Rational::new(0, 1).expect("zero is representable");
    let radius = convert(radius_mm, policy.linear_tolerance_mm)?;
    let circle = Circle::new([zero; 2], radius)
        .map_err(|cause| invalid(format!("Invalid STEP circle: {cause:?}")))?;
    let cylinder = AxialCylinder::new(feature.uuid_bits(), circle, zero, height)
        .map_err(|cause| invalid(format!("Invalid STEP cylinder: {cause:?}")))?;
    let source = Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), policy)?;
    let normal = frame.normal();
    source.transformed(RigidTransform {
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], normal[i]]),
        translation_mm: frame.origin,
    })
}

pub(super) fn feature_origin(
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

pub(super) fn spline_control_key(poles: &[Point], knots: &[f64]) -> Vec<u64> {
    let oriented = |reverse: bool| {
        let last = knots.len() - 1;
        let width = knots[last] - knots[0];
        (0..poles.len())
            .flat_map(|i| poles[if reverse { poles.len() - 1 - i } else { i }])
            .chain((0..knots.len()).map(|i| {
                if reverse {
                    (knots[last] - knots[last - i]) / width
                } else {
                    (knots[i] - knots[0]) / width
                }
            }))
            .map(|value| if value == 0. { 0 } else { value.to_bits() })
            .collect::<Vec<_>>()
    };
    oriented(false).min(oriented(true))
}

fn straight_origin(feature: EntityId, frame: PlaneFrame, a: [f64; 2], b: [f64; 2]) -> Origin {
    let mut points = [frame.point(a), frame.point(b)];
    points.sort_unstable_by(compare_points);
    feature_origin(feature, "edge", points.into_iter().flatten())
}

fn compare_points(a: &Point, b: &Point) -> std::cmp::Ordering {
    a.iter()
        .zip(b)
        .find_map(|(a, b)| (a != b).then(|| a.total_cmp(b)))
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn pocket_origins(
    feature: EntityId,
    frame: PlaneFrame,
    pockets: &[OrthogonalPocket],
) -> Vec<Origin> {
    pockets
        .iter()
        .map(|pocket| {
            let center = crate::rust_prism_topology::position(frame, pocket.center_mm);
            feature_origin(
                feature,
                "pocket",
                center
                    .into_iter()
                    .chain([pocket.radius_mm, pocket.depth_mm]),
            )
        })
        .collect()
}

impl Solid {
    pub(super) fn read_rust_step_file(
        path: &Path,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        use std::io::Read;
        // Match the independent Part21 parser's 64 MiB input ceiling. Check
        // the open file, then bound the read as well so concurrent growth
        // cannot turn a checked input into an unbounded allocation.
        const MAX_BYTES: u64 = 64 * 1024 * 1024;
        let io_error =
            |cause: std::io::Error| error("read STEP", GeometryErrorKind::Input, cause.to_string());
        let file = std::fs::File::open(path).map_err(io_error)?;
        let exceeded = || {
            error(
                "read STEP",
                GeometryErrorKind::Limit,
                "STEP file exceeds the Rust import byte budget",
            )
        };
        if file.metadata().map_err(io_error)?.len() > MAX_BYTES {
            return Err(exceeded());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(exceeded());
        }
        let input = std::str::from_utf8(&bytes).map_err(|_| {
            error(
                "read STEP",
                GeometryErrorKind::Input,
                "STEP input must be UTF-8 text",
            )
        })?;
        Self::import_rust_step(input, EntityId::new(), policy)?.without_authored_provenance()
    }

    /// Import a verified analytic STEP solid or independent solid group in millimeters.
    pub fn import_rust_step(
        input: &str,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if feature.is_nil() {
            return Err(invalid("STEP import needs a source identity"));
        }
        let tolerance = policy.linear_tolerance_mm;
        let mut members = step_read::read_import_compound(input, tolerance).map_err(|cause| {
            let (kind, message) = match cause {
                ReadError::Invalid => (GeometryErrorKind::InvalidResult, "Invalid STEP geometry"),
                ReadError::Limit => (
                    GeometryErrorKind::Limit,
                    "STEP exceeds the Rust import budget",
                ),
                ReadError::Unsupported => (
                    GeometryErrorKind::Unsupported,
                    "Rust STEP import requires a supported analytic solid in millimeters",
                ),
            };
            error(IMPORT, kind, message)
        })?;
        if members.len() == 1 {
            let (readback, imported) = members.pop().expect("one member");
            return Self::verified_step_member(&readback, imported, feature, policy);
        }
        compound::rebuild(members, feature, policy)
    }

    fn verified_step_member(
        readback: &Readback,
        imported: ImportSolid,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let orthogonal = matches!(
            &imported.profile,
            ImportProfile::Orthogonal { .. }
                | ImportProfile::PocketedOrthogonal { .. }
                | ImportProfile::SlottedOrthogonal { .. }
        );
        let solid = Self::from_imported_step(imported, feature, policy)?;
        verify(
            &solid,
            readback,
            feature,
            policy.linear_tolerance_mm,
            orthogonal,
        )?;
        Ok(solid)
    }

    fn from_imported_step(
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
        match profile {
            ImportProfile::RoundedThreadBore {
                spec,
                stock_radius_mm,
            } => Self::import_rounded_thread_bore(spec, stock_radius_mm, frame, feature, policy),
            ImportProfile::RoundedThread { spec } => {
                Self::import_rounded_thread(spec, frame, feature, policy)
            }
            ImportProfile::NormalRelief { relief } => {
                Self::from_imported_normal_relief(relief, frame, feature, policy)
            }
            ImportProfile::HelicalThread {
                spec,
                stock_radius_mm,
            } => Self::import_helical_thread(spec, stock_radius_mm, frame, feature, policy),
            ImportProfile::ThreadEndRelief {
                tool_spec,
                material_length_mm,
                neck_radius_mm,
            } => Self::import_thread_end_relief(
                tool_spec,
                material_length_mm,
                neck_radius_mm,
                frame,
                feature,
                policy,
            ),
            ImportProfile::ThreadedShaft {
                spec,
                stock_radius_mm,
                stock_length_mm,
            } => Self::import_threaded_shaft(
                spec,
                stock_radius_mm,
                stock_length_mm,
                frame,
                feature,
                policy,
            ),
            ImportProfile::HelicalReceiver {
                spec,
                stock_radius_mm,
                stock_length_mm,
            } => Self::import_helical_receiver(
                spec,
                stock_radius_mm,
                stock_length_mm,
                frame,
                feature,
                policy,
            ),
            ImportProfile::HelicalNut {
                spec,
                across_flats_mm,
            } => Self::import_helical_nut(spec, across_flats_mm, frame, feature, policy),
            ImportProfile::HelicalScrew { spec, screw } => {
                Self::import_helical_screw(spec, screw, frame, feature, policy)
            }
            ImportProfile::Sphere { radius_mm } => {
                Self::rust_sphere_named(radius_mm, frame, feature, policy)
            }
            ImportProfile::Torus {
                major_radius_mm,
                minor_radius_mm,
            } => Self::rust_torus_named(
                [major_radius_mm, minor_radius_mm],
                frame,
                feature,
                Origin {
                    source: feature,
                    role: "import.torus.profile".into(),
                },
                policy,
            ),
            ImportProfile::RoundedCylinder {
                radius_mm,
                fillet_radii_mm,
            } => {
                let geometry = multicad_rust_kernel::rounded_cylinder::RoundedCylinder::new(
                    radius_mm,
                    height_mm,
                    fillet_radii_mm,
                    policy.minimum_feature_mm,
                    policy.maximum_coordinate_mm,
                )
                .map_err(|_| invalid("rounded cylinder exceeds import policy"))?;
                Self::import_rounded_cylinder(geometry, frame, feature, policy)
            }
            ImportProfile::SplineSweep { sweep } => {
                let body = SplineSweepBody::new(feature, sweep, frame, policy.linear_tolerance_mm)?;
                Ok(Self {
                    body: SolidBody::SplineSweep(Rc::new(body), frame),
                    policy,
                    provenance: vec![],
                    rust_origins: vec![],
                    retained_placement: None,
                })
            }
            ImportProfile::NormalSweep { sweep } => {
                Self::from_imported_normal_sweep(&sweep, frame, feature, policy)
            }
            ImportProfile::NonplanarLoft { geometry } => {
                Self::from_imported_nonplanar_loft(geometry, feature, frame, policy)
            }
            ImportProfile::AxialProfileStack { stack } => {
                Self::from_imported_axial_stack(stack, frame, feature, policy)
            }
            ImportProfile::WarpedUnion { shell } => {
                Self::from_imported_warped_union(shell, frame, feature, policy)
            }
            ImportProfile::PlanarBoundary { shell } => {
                Self::from_imported_planar_boundary(shell, feature, policy)
            }
            ImportProfile::CylindricalRevolution { meridian }
            | ImportProfile::ConicalRevolution { meridian } => {
                let axis = frame.normal();
                let profile_frame = PlaneFrame { v: axis, ..frame };
                let mut outer = Vec::with_capacity(meridian.len());
                let mut origins = Vec::with_capacity(meridian.len());
                for (&a, &b) in meridian.iter().zip(meridian.iter().cycle().skip(1)) {
                    outer.push(CurvedEdge2d::Line { a, b });
                    origins.push(straight_origin(feature, profile_frame, a, b));
                }
                Self::rust_revolve_curved_profile_named(
                    &CurvedProfile {
                        frame: PlaneFrame {
                            v: [0., 0., 1.],
                            ..PlaneFrame::default()
                        },
                        outer,
                        holes: vec![],
                    },
                    crate::RevolutionAxis {
                        origin_mm: [0.; 3],
                        direction: [0., 0., 1.],
                    },
                    TAU,
                    feature,
                    &origins,
                    policy,
                )?
                .transformed(RigidTransform {
                    rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], axis[i]]),
                    translation_mm: frame.origin,
                })
            }
            ImportProfile::PiercedNormalSweep { sweep } => {
                let stock =
                    Self::from_imported_normal_sweep(sweep.sweep(), frame, feature, policy)?;
                let origins = sweep
                    .openings()
                    .iter()
                    .map(|opening| {
                        opening
                            .spec()
                            .outline
                            .iter()
                            .map(|edge| {
                                let coordinates = [0., 0.5, 1.].into_iter().flat_map(|t| {
                                    crate::rust_prism_topology::position(
                                        frame,
                                        opening.point(edge.point(t), false),
                                    )
                                });
                                feature_origin(feature, "sweep-opening", coordinates)
                            })
                            .collect()
                    })
                    .collect();
                stock.with_certified_sweep_openings(sweep, origins)
            }
            ImportProfile::Orthogonal { solid } => {
                Self::from_imported_orthogonal(solid, frame, feature, policy)
            }
            ImportProfile::PocketedOrthogonal { solid, pockets } => {
                let origins = pocket_origins(feature, frame, &pockets);
                Self::from_imported_orthogonal(solid, frame, feature, policy)?
                    .with_orthogonal_pockets(pockets, origins)
            }
            ImportProfile::DrilledOrthogonal {
                solid,
                pockets,
                through,
            } => {
                let circle_origins = pocket_origins(feature, frame, &pockets);
                let through_origins = through
                    .iter()
                    .map(|bore| {
                        let mut centers = bore
                            .centers_mm
                            .map(|point| crate::rust_prism_topology::position(frame, point));
                        centers.sort_unstable_by(compare_points);
                        feature_origin(
                            feature,
                            "through-bore",
                            centers.into_iter().flatten().chain([bore.radius_mm]),
                        )
                    })
                    .collect();
                Self::from_imported_orthogonal(solid, frame, feature, policy)?
                    .with_orthogonal_round_cuts(pockets, circle_origins, through, through_origins)
            }
            ImportProfile::SlottedOrthogonal {
                solid,
                pockets,
                slots,
            } => {
                let circle_origins = pocket_origins(feature, frame, &pockets);
                let slot_origins = slots
                    .iter()
                    .map(|slot| {
                        let mut centers = slot
                            .centers_mm
                            .map(|point| crate::rust_prism_topology::position(frame, point));
                        centers.sort_unstable_by(compare_points);
                        feature_origin(
                            feature,
                            "slot",
                            centers
                                .into_iter()
                                .flatten()
                                .chain([slot.radius_mm, slot.depth_mm]),
                        )
                    })
                    .collect();
                Self::from_imported_orthogonal(solid, frame, feature, policy)?
                    .with_orthogonal_pockets(pockets, circle_origins)?
                    .with_orthogonal_slots(slots, slot_origins)
            }
            ImportProfile::Stadium { profile } => {
                let corners = profile.corners();
                let centers = profile.centers();
                let origins = std::array::from_fn(|index| {
                    let a = corners[index];
                    let b = corners[(index + 1) % 4];
                    if index.is_multiple_of(2) {
                        return straight_origin(feature, frame, a, b);
                    }
                    let center = frame.point(centers[usize::from(index == 1)]);
                    feature_origin(
                        feature,
                        "semicircle",
                        center
                            .into_iter()
                            .chain(frame.point(a))
                            .chain(frame.point(b)),
                    )
                });
                Self::rust_extrude_rounded_slot(
                    frame,
                    centers,
                    profile.radius_mm(),
                    height_mm,
                    feature,
                    origins,
                    policy,
                )
            }
            ImportProfile::LinearSweep { base, top } => {
                Self::from_imported_linear_sweep(base, top, height_mm, frame, feature, policy)
            }
            ImportProfile::TrimmedSweep {
                rings,
                direction,
                travels,
            } => {
                let edges = rings
                    .iter()
                    .flat_map(|ring| {
                        ring.iter()
                            .zip(ring.iter().cycle().skip(1))
                            .map(|(&a, &b)| (straight_origin(feature, frame, a, b), false))
                    })
                    .collect();
                let sweep = multicad_rust_kernel::trimmed_sweep::TrimmedSweep::new(
                    rings,
                    direction,
                    travels,
                    policy.minimum_feature_mm,
                )
                .map_err(|cause| {
                    invalid(format!(
                        "Invalid STEP plane-terminated extrusion: {cause:?}"
                    ))
                })?;
                for &point in sweep.rings().iter().flatten() {
                    policy.length(sweep.travel(true, point))?;
                }
                Self::from_trimmed_linear_sweep(sweep, frame, None, feature, edges, policy)
            }
            ImportProfile::Annulus {
                inner_radius_mm,
                outer_radius_mm,
            } => {
                let rims = [("hole", inner_radius_mm), ("edge", outer_radius_mm)];
                Self::rust_annulus_from_sketch(
                    [inner_radius_mm, outer_radius_mm],
                    height_mm,
                    feature,
                    frame,
                    [0.; 2],
                    rims.map(|(kind, radius)| {
                        feature_origin(feature, kind, frame.origin.into_iter().chain([radius]))
                    }),
                    policy,
                )
            }
            ImportProfile::Cylinder { radius_mm } => {
                import_cylinder(radius_mm, height_mm, frame, feature, policy)
            }
            ImportProfile::HelicalTubeCut {
                stock_radius_mm,
                radius_mm,
                pitch_mm,
                path_height_mm,
                start_height_mm,
                start_angle_rad,
            } => import_cylinder(stock_radius_mm, height_mm, frame, feature, policy)?
                .rust_cut_helical_tube(
                    HelicalTube {
                        radius_mm,
                        pitch_mm,
                        path_height_mm,
                        start_height_mm,
                        start_angle_rad,
                    },
                    feature,
                    policy,
                ),
            ImportProfile::Polygon {
                rings,
                circular_holes,
            } => Self::from_imported_polygon(
                &rings,
                &circular_holes,
                height_mm,
                frame,
                feature,
                policy,
                true,
            ),
            ImportProfile::ChamferedPolygon {
                rings,
                circular_holes,
                chamfers,
            } => {
                let stock = Self::from_imported_polygon(
                    &rings,
                    &circular_holes,
                    height_mm,
                    frame,
                    feature,
                    policy,
                    false,
                )?;
                let origins = chamfers
                    .iter()
                    .map(|chamfer| {
                        let (center, radius) = circular_holes[chamfer.hole];
                        (
                            (chamfer.hole, chamfer.end),
                            feature_origin(
                                feature,
                                "bore-chamfer",
                                frame.point(center).into_iter().chain([
                                    radius,
                                    chamfer.size_mm,
                                    f64::from(chamfer.end),
                                ]),
                            ),
                        )
                    })
                    .collect();
                stock.with_polygon_bore_treatments(&chamfers, origins, policy)
            }
            ImportProfile::CounterboredPolygon {
                rings,
                circular_holes,
                counterbores,
                source_faces,
            } => {
                let stock = Self::from_imported_polygon(
                    &rings,
                    &circular_holes,
                    height_mm,
                    frame,
                    feature,
                    policy,
                    false,
                )?;
                let origins = counterbores
                    .iter()
                    .map(|cut| {
                        let (center, radius) = circular_holes[cut.hole];
                        feature_origin(
                            feature,
                            "counterbore",
                            frame.point(center).into_iter().chain([
                                radius,
                                cut.radius_mm,
                                cut.depth_mm,
                                f64::from(cut.end),
                            ]),
                        )
                    })
                    .collect::<Vec<_>>();
                stock.with_polygon_counterbores(&counterbores, &origins, policy, Some(source_faces))
            }
            ImportProfile::PocketedPolygon {
                rings,
                circular_holes,
                pockets,
            } => {
                let stock = Self::from_imported_polygon(
                    &rings,
                    &circular_holes,
                    height_mm,
                    frame,
                    feature,
                    policy,
                    false,
                )?;
                let origins = pockets
                    .iter()
                    .map(|pocket| {
                        feature_origin(
                            feature,
                            "pocket",
                            frame.point(pocket.center_mm).into_iter().chain([
                                pocket.radius_mm,
                                pocket.depth_mm,
                                f64::from(pocket.end),
                            ]),
                        )
                    })
                    .collect();
                stock.with_polygon_pockets(pockets, origins)
            }
        }
    }

    fn from_imported_normal_sweep(
        sweep: &multicad_rust_kernel::normal_sweep::Sweep,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let mut origins = Vec::new();
        let mut wires = sweep.rings().iter().map(|ring| {
            ring.iter()
                .map(|&edge| {
                    let (curve, origin) = match edge {
                        SweepEdge::Line { a, b } => (
                            CurvedEdge2d::Line { a, b },
                            straight_origin(feature, frame, a, b),
                        ),
                        SweepEdge::Arc {
                            center,
                            a,
                            b,
                            clockwise,
                        } => {
                            let mut endpoints = [frame.point(a), frame.point(b)];
                            endpoints.sort_unstable_by(compare_points);
                            let origin = feature_origin(
                                feature,
                                "arc",
                                endpoints
                                    .into_iter()
                                    .flatten()
                                    .chain(frame.point(edge.point(0.5))),
                            );
                            (
                                CurvedEdge2d::Arc {
                                    center,
                                    a,
                                    b,
                                    clockwise,
                                },
                                origin,
                            )
                        }
                        SweepEdge::Circle {
                            center, radius_mm, ..
                        } => {
                            let origin = feature_origin(
                                feature,
                                "circle",
                                frame.point(center).into_iter().chain([radius_mm]),
                            );
                            (CurvedEdge2d::Circle { center, radius_mm }, origin)
                        }
                    };
                    origins.push(origin);
                    curve
                })
                .collect()
        });
        let outer = wires
            .next()
            .ok_or_else(|| invalid("STEP normal sweep has no outer ring"))?;
        let holes = wires.collect();
        Self::rust_extrude_normal_profile_named(
            &CurvedProfile {
                frame,
                outer,
                holes,
            },
            sweep.height_mm(),
            feature,
            &origins,
            policy,
        )
    }

    fn from_imported_linear_sweep(
        base: Vec<Vec<[f64; 2]>>,
        top: Vec<Vec<[f64; 2]>>,
        height_mm: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let edges = base
            .iter()
            .flat_map(|ring| {
                ring.iter()
                    .zip(ring.iter().cycle().skip(1))
                    .map(|(&a, &b)| (straight_origin(feature, frame, a, b), false))
            })
            .collect();
        let sweep = multicad_rust_kernel::linear_sweep::Sweep::with_profile_budget(
            base,
            top,
            height_mm,
            multicad_rust_kernel::linear_sweep::MAX_PROFILE_VERTICES,
        )
        .map_err(|cause| invalid(format!("Invalid STEP linear sweep: {cause:?}")))?;
        Self::from_linear_sweep(sweep, frame, feature, edges, [0.; 2], false, policy)
    }

    fn from_imported_polygon(
        rings: &[Vec<[f64; 2]>],
        circular_holes: &[([f64; 2], f64)],
        height_mm: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
        allow_linear_fallback: bool,
    ) -> Result<Self, GeometryError> {
        let dimension = |value| convert(value, policy.linear_tolerance_mm);
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
        let prism =
            match PolygonPrism::with_circular_holes(exact_rings, holes, dimension(height_mm)?) {
                Ok(prism) => prism,
                Err(
                    PolygonError::Arithmetic(RationalError::OutOfRange) | PolygonError::VertexLimit,
                ) if allow_linear_fallback && circular_holes.is_empty() => {
                    // The independent reader proved a straight planar sweep.
                    // Preserve all coordinates when rational products overflow
                    // or this wire exceeds the smaller exact polygon budget.
                    // The bounded sweep and full readback proof still govern admission.
                    return Self::from_imported_linear_sweep(
                        rings.to_vec(),
                        rings.to_vec(),
                        height_mm,
                        frame,
                        feature,
                        policy,
                    );
                }
                Err(cause) => return Err(invalid(format!("Invalid STEP polygon: {cause:?}"))),
            };
        let mut origins: Vec<_> = rings
            .iter()
            .flat_map(|ring| {
                ring.iter()
                    .zip(ring.iter().cycle().skip(1))
                    .map(|(&a, &b)| straight_origin(feature, frame, a, b))
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

fn max_error(a: Point, b: Point) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0., f64::max)
}

fn match_circles(expected: &[CircleEdge], actual: &[CircleEdge], tolerance: f64) -> bool {
    match_round_edges(expected, actual, tolerance, circle_error)
}

fn circle_error(rim: &CircleEdge, candidate: &CircleEdge) -> f64 {
    let normal = max_error(rim.normal, candidate.normal)
        .min(max_error(rim.normal, candidate.normal.map(|axis| -axis)));
    max_error(rim.center_mm, candidate.center_mm)
        .max((rim.radius_mm - candidate.radius_mm).abs())
        .max(normal * rim.radius_mm)
}

fn match_semicircles(
    expected: &[SemicircleEdge],
    actual: &[SemicircleEdge],
    tolerance: f64,
) -> bool {
    let circle = |rim: &SemicircleEdge| CircleEdge {
        center_mm: rim.center_mm,
        normal: rim.normal,
        radius_mm: rim.radius_mm,
    };
    match_round_edges(expected, actual, tolerance, |rim, candidate| {
        circle_error(&circle(rim), &circle(candidate))
            .max(max_error(rim.radial_mid, candidate.radial_mid) * rim.radius_mm)
    })
}

fn match_arcs(expected: &[ArcEdge], actual: &[ArcEdge], tolerance: f64) -> bool {
    let circle = |rim: &ArcEdge| CircleEdge {
        center_mm: rim.center_mm,
        normal: rim.normal,
        radius_mm: rim.radius_mm,
    };
    let mid = |rim: &ArcEdge| {
        let tangent = cross(rim.normal, rim.radial_start);
        let (s, c) = (rim.sweep_rad * 0.5).sin_cos();
        std::array::from_fn(|i| c.mul_add(rim.radial_start[i], s * tangent[i]))
    };
    match_round_edges(expected, actual, tolerance, |rim, candidate| {
        let paired = max_error(rim.radial_start, candidate.radial_start)
            .max(max_error(rim.radial_end, candidate.radial_end));
        let swapped = max_error(rim.radial_start, candidate.radial_end)
            .max(max_error(rim.radial_end, candidate.radial_start));
        circle_error(&circle(rim), &circle(candidate))
            .max(paired.min(swapped) * rim.radius_mm)
            .max(max_error(mid(rim), mid(candidate)) * rim.radius_mm)
    })
}

fn match_ellipses(expected: &[EllipseArc], actual: &[EllipseArc], tolerance: f64) -> bool {
    let midpoint = |arc: EllipseArc| {
        arc.ellipse()
            .point_at(arc.start_rad() + arc.sweep_rad() * 0.5)
    };
    let axis_error = |a: Point, b: Point| max_error(a, b).min(max_error(a, b.map(|c| -c)));
    match_round_edges(expected, actual, tolerance, |&a, &b| {
        let [ellipse, other] = [a, b].map(EllipseArc::ellipse);
        let [major, minor] = ellipse.semiaxes_mm();
        let [other_major, other_minor] = other.semiaxes_mm();
        let [a0, a1] = a.endpoints();
        let [b0, b1] = b.endpoints();
        let ends = max_error(a0, b0)
            .max(max_error(a1, b1))
            .min(max_error(a0, b1).max(max_error(a1, b0)));
        max_error(ellipse.center_mm(), other.center_mm())
            .max((major - other_major).abs())
            .max((minor - other_minor).abs())
            .max(axis_error(ellipse.major_axis(), other.major_axis()) * major)
            .max(axis_error(ellipse.normal(), other.normal()) * major)
            .max((a.sweep_rad().abs() - b.sweep_rad().abs()).abs() * major)
            .max(max_error(midpoint(a), midpoint(b)))
            .max(ends)
    })
}

fn match_round_edges<T, U>(
    expected: &[T],
    actual: &[U],
    tolerance: f64,
    deviation: impl Fn(&T, &U) -> f64,
) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    let mut matched = vec![false; actual.len()];
    expected.iter().all(|rim| {
        let hit = actual
            .iter()
            .enumerate()
            .filter(|(i, _)| !matched[*i])
            .filter_map(|(i, candidate)| {
                let error = deviation(rim, candidate);
                (error <= tolerance).then_some((i, error))
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b));
        if let Some((i, _)) = hit {
            matched[i] = true;
            true
        } else {
            false
        }
    })
}

fn same_segments(
    actual: &[Segment],
    expected: &[Segment],
    bounds: [[f64; 3]; 2],
    tolerance: f64,
) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    // Index both directions by the largest spatial extent. This avoids an
    // all-pairs comparison without relying on input order or edge winding.
    let axis = (0..3)
        .max_by(|&a, &b| (bounds[1][a] - bounds[0][a]).total_cmp(&(bounds[1][b] - bounds[0][b])))
        .expect("three axes");
    let mut ordered: Vec<_> = actual
        .iter()
        .copied()
        .enumerate()
        .flat_map(|(i, [a, b])| [(i, [a, b]), (i, [b, a])])
        .collect();
    ordered.sort_unstable_by(|a, b| a.1[0][axis].total_cmp(&b.1[0][axis]));
    let mut used = vec![false; actual.len()];
    expected.iter().all(|edge| {
        let x = edge[0][axis];
        let first = ordered.partition_point(|(_, p)| p[0][axis] < x - tolerance);
        let hit = ordered[first..]
            .iter()
            .take_while(|(_, p)| p[0][axis] <= x + tolerance)
            .filter(|(i, _)| !used[*i])
            .filter_map(|(i, p)| {
                let error = p
                    .iter()
                    .flatten()
                    .zip(edge.iter().flatten())
                    .map(|(a, b)| (a - b).abs())
                    .fold(0., f64::max);
                (error <= tolerance).then_some((*i, error))
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b));
        if let Some((i, _)) = hit {
            used[i] = true;
            true
        } else {
            false
        }
    })
}

fn verify_step_metrics(
    solid: &Solid,
    readback: &Readback,
    tolerance: f64,
) -> Result<(), GeometryError> {
    let bounds = solid.bounds()?;
    solid.policy.point(bounds.min)?;
    solid.policy.point(bounds.max)?;
    verify_shell_metrics(bounds, solid.volume_mm3()?, readback, tolerance)
}

fn verify_shell_metrics(
    bounds: Bounds,
    volume: f64,
    readback: &Readback,
    tolerance: f64,
) -> Result<(), GeometryError> {
    let [x, y, z] = std::array::from_fn(|i| bounds.max[i] - bounds.min[i]);
    let volume_tolerance =
        (readback.volume_mm3 * 1e-8).max(tolerance * 2. * (x * y + y * z + z * x));
    if max_error(bounds.min, readback.bounds_mm[0]) > tolerance * 2.
        || max_error(bounds.max, readback.bounds_mm[1]) > tolerance * 2.
        || (volume - readback.volume_mm3).abs() > volume_tolerance
    {
        return Err(invalid(
            "Rebuilt STEP solid differs from the imported bounds or volume",
        ));
    }
    Ok(())
}

fn verify(
    solid: &Solid,
    readback: &Readback,
    feature: EntityId,
    tolerance: f64,
    orthogonal: bool,
) -> Result<(), GeometryError> {
    verify_step_metrics(solid, readback, tolerance)?;
    if solid.rounded_thread_bore_body().is_some()
        || solid.rounded_thread_body().is_some()
        || solid.helical_body().is_some()
        || solid.helical_tube_cut_body().is_some()
    {
        // Typed helical imports are produced only after complete analytic
        // control-net/support and oriented edge-incidence proofs. Their exact
        // rebuilt feature can partition the same physical boundary differently.
        return Ok(());
    }
    if orthogonal
        && readback.circles.is_empty()
        && readback.semicircles.is_empty()
        && readback.arcs.is_empty()
        && readback.ellipses.is_empty()
    {
        // The pure reader has already proved equality of every oriented
        // boundary patch. The rebuilt shell consolidates coplanar source
        // faces and collinear edges, so record counts can differ.
        return Ok(());
    }
    let topology = solid.topology_snapshot(feature, 0)?;
    if !compare_topology(
        &topology.elements,
        readback,
        tolerance * 2.,
        orthogonal,
        solid.spline_sweep_body(),
    ) {
        return Err(invalid(
            "Rebuilt STEP extrusion differs from the imported analytic topology",
        ));
    }
    Ok(())
}

impl Solid {
    /// Compare an independently read STEP shell with the exact topology being
    /// published. Compare finite trims and bound spline deviation from each exact involute.
    pub fn step_topology_matches(
        topology: &TopologySnapshot,
        readback: &Readback,
        tolerance: f64,
    ) -> bool {
        tolerance.is_finite()
            && tolerance > 0.
            && compare_topology(&topology.elements, readback, tolerance, false, None)
    }
}

fn compare_topology<'a>(
    elements: impl IntoIterator<Item = &'a TopologyElement>,
    readback: &Readback,
    tolerance: f64,
    allow_consolidation: bool,
    spline_body: Option<(&SplineSweepBody, PlaneFrame)>,
) -> bool {
    let mut lines = Vec::new();
    let mut circles = Vec::new();
    let mut semicircles = Vec::new();
    let mut arcs = Vec::new();
    let mut ellipses = Vec::new();
    let mut splines = Vec::new();
    let mut faces = 0;
    let mut retained_edges = 0;
    for element in elements {
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
            ) if (sweep_rad.abs() - TAU).abs() < 1e-9 => {
                circles.push(CircleEdge {
                    center_mm: frame.origin_mm,
                    normal: frame.normal(),
                    radius_mm,
                });
            }
            (
                TopologyKind::Edge,
                Some(AnalyticGeometry::CircularArc {
                    frame,
                    radius_mm,
                    start_rad,
                    sweep_rad,
                }),
            ) if (sweep_rad.abs() - PI).abs() < 1e-9 => {
                let (sin, cos) = (start_rad + sweep_rad * 0.5).sin_cos();
                semicircles.push(SemicircleEdge {
                    center_mm: frame.origin_mm,
                    normal: frame.normal(),
                    radius_mm,
                    radial_mid: std::array::from_fn(|i| cos * frame.u[i] + sin * frame.v[i]),
                });
            }
            (
                TopologyKind::Edge,
                Some(AnalyticGeometry::CircularArc {
                    frame,
                    radius_mm,
                    start_rad,
                    sweep_rad,
                }),
            ) => {
                let radial = |angle: f64| {
                    let (s, c) = angle.sin_cos();
                    std::array::from_fn(|i| c.mul_add(frame.u[i], s * frame.v[i]))
                };
                let (start, end) = if sweep_rad > 0. {
                    (start_rad, start_rad + sweep_rad)
                } else {
                    (start_rad + sweep_rad, start_rad)
                };
                arcs.push(ArcEdge {
                    center_mm: frame.origin_mm,
                    normal: frame.normal(),
                    radius_mm,
                    radial_start: radial(start),
                    radial_end: radial(end),
                    sweep_rad: sweep_rad.abs(),
                });
            }
            (
                TopologyKind::Edge,
                Some(AnalyticGeometry::EllipticArc {
                    frame,
                    semiaxes_mm,
                    start_rad,
                    sweep_rad,
                }),
            ) => {
                let Ok(ellipse) =
                    Ellipse::new(frame.origin_mm, frame.normal(), frame.u, semiaxes_mm)
                else {
                    return false;
                };
                let Ok(arc) = EllipseArc::new(ellipse, start_rad, sweep_rad) else {
                    return false;
                };
                ellipses.push(arc);
            }
            (
                TopologyKind::Edge,
                Some(AnalyticGeometry::InvoluteArc {
                    frame,
                    base_radius_mm,
                    parameters,
                }),
            ) => splines.push(involute::Flank {
                frame,
                radius_mm: base_radius_mm,
                parameters,
            }),
            (TopologyKind::Edge, None) if spline_body.is_some() => retained_edges += 1,
            (TopologyKind::Edge, _) => {
                return false;
            }
            (TopologyKind::Vertex, _) => {}
        }
    }
    (allow_consolidation
        || (faces == readback.face_count
            && same_segments(&readback.edges_mm, &lines, readback.bounds_mm, tolerance)))
        && match_circles(&readback.circles, &circles, tolerance)
        && match_semicircles(&readback.semicircles, &semicircles, tolerance)
        && match_arcs(&readback.arcs, &arcs, tolerance)
        && match_ellipses(&readback.ellipses, &ellipses, tolerance)
        && if let Some((body, frame)) = spline_body {
            retained_edges == readback.splines.len()
                && splines.is_empty()
                && body.matches_splines(frame, &readback.splines, tolerance)
        } else {
            involute::matches(&splines, &readback.splines, tolerance)
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_edge_match_is_unordered_and_cannot_reuse_or_move_a_boundary() {
        let bounds = [[0., 0., 0.], [8., 4., 2.]];
        let a = [[0., 0., 0.], [8., 0., 0.]];
        let b = [[8., 0., 0.], [8., 4., 0.]];
        let reversed_with_noise = [[8. + 1e-8, 0., 0.], [0., 0., 0.]];
        assert!(same_segments(
            &[b, reversed_with_noise],
            &[a, b],
            bounds,
            1e-7
        ));
        assert!(!same_segments(
            &[b, reversed_with_noise],
            &[a, a],
            bounds,
            1e-7
        ));
        assert!(!same_segments(&[b], &[a, b], bounds, 1e-7));
        let moved = [[8., 0., 0.], [8., 3., 0.]];
        assert!(!same_segments(&[a, moved], &[a, b], bounds, 1e-7));
    }

    #[test]
    fn step_edge_match_prefers_the_closest_of_nearby_boundaries() {
        let bounds = [[0., 0., 0.], [8., 4., 2.]];
        let edge = |x| [[x, 0., 0.], [x, 4., 0.]];
        let [a, b, c] = [edge(1.), edge(1.0075), edge(1.015)];
        assert!(same_segments(&[a, b, c], &[b, c, a], bounds, 0.01));
    }

    #[test]
    fn step_circle_match_rejects_shifted_bores_and_compares_unsigned_axes() {
        let expected = CircleEdge {
            center_mm: [2., 4., 6.],
            normal: [0., 1., 0.],
            radius_mm: 1.25,
        };
        let reversed = CircleEdge {
            normal: [0., -1., 0.],
            ..expected
        };
        assert!(match_circles(&[reversed], &[expected], 1e-7));
        for changed in [
            CircleEdge {
                center_mm: [2., 4., 7.],
                ..expected
            },
            CircleEdge {
                normal: [0., 0., 1.],
                ..expected
            },
            CircleEdge {
                radius_mm: 1.5,
                ..expected
            },
        ] {
            assert!(!match_circles(&[changed], &[expected], 1e-7));
        }
    }

    #[test]
    fn step_semicircle_match_distinguishes_opposite_halves() {
        let expected = SemicircleEdge {
            center_mm: [2., 4., 6.],
            normal: [0., 0., 1.],
            radius_mm: 1.25,
            radial_mid: [0., 1., 0.],
        };
        let opposite = SemicircleEdge {
            radial_mid: [0., -1., 0.],
            ..expected
        };
        assert!(match_semicircles(&[expected], &[expected], 1e-7));
        assert!(!match_semicircles(&[expected], &[opposite], 1e-7));
        assert!(match_semicircles(
            &[expected, opposite],
            &[opposite, expected],
            1e-7
        ));
        assert!(!match_semicircles(
            &[expected, opposite],
            &[opposite, opposite],
            1e-7
        ));
    }
}
