//! Authored polynomial spline sections retain source curves and exact basis spans.
use super::*;
use crate::{CurvedEdge2d, CurvedProfile, GeometryPolicy, NativeRegionQueries};
use multicad_core::{
    project_file::ContentHash,
    references::ReferenceFrame,
    sketch_regions::{Edge as RegionEdge, ExactCurve, RegionQueries},
};
use multicad_rust_kernel::spline_sweep::{ConstructionError, Cubic};
use std::rc::Rc;

fn failure(kind: GeometryErrorKind, message: impl std::fmt::Display) -> GeometryError {
    super::super::error("spline extrusion", kind, message.to_string())
}
fn map_error(error: ConstructionError) -> GeometryError {
    failure(
        match error {
            ConstructionError::Invalid => GeometryErrorKind::Input,
            ConstructionError::Unresolved => GeometryErrorKind::Unsupported,
            ConstructionError::Limit => GeometryErrorKind::Limit,
            ConstructionError::Cancelled => GeometryErrorKind::Cancelled,
        },
        format!("Authored spline section could not be certified: {error:?}"),
    )
}
fn cancelled(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), GeometryError> {
    if interrupt.is_some_and(|interrupt| interrupt()) {
        Err(map_error(ConstructionError::Cancelled))
    } else {
        Ok(())
    }
}
fn exact_curve(edge: &CurvedEdge2d) -> ExactCurve {
    match edge {
        CurvedEdge2d::Line { a, b } => ExactCurve::Line { a: *a, b: *b },
        CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise,
        } => ExactCurve::Arc {
            center: *center,
            a: *a,
            b: *b,
            clockwise: *clockwise,
        },
        CurvedEdge2d::Circle { center, radius_mm } => ExactCurve::Circle {
            center: *center,
            radius_mm: *radius_mm,
        },
        CurvedEdge2d::BSpline(spline) => ExactCurve::BSpline {
            poles: spline.poles.clone(),
            basis: spline.basis(),
        },
    }
}
struct AuthoredEdge {
    edge: Edge,
    origins: Vec<Origin>,
    anchor: Option<ReferenceFrame>,
    fragment: bool,
}
fn aliases(feature: EntityId, origin: &Origin) -> Vec<Origin> {
    vec![
        Origin {
            source: feature,
            role: format!("side/{}/{}", origin.source, origin.role),
        },
        origin.clone(),
    ]
}
fn finite_trim(feature: EntityId, origin: &Origin, points: &[[f64; 2]]) -> Origin {
    let mut bytes = origin.source.uuid_bits().to_le_bytes().to_vec();
    bytes.extend((origin.role.len() as u32).to_le_bytes());
    bytes.extend(origin.role.as_bytes());
    bytes.extend((points.len() as u32).to_le_bytes());
    for coordinate in points.iter().flatten() {
        bytes.extend(if *coordinate == 0. {
            0_f64.to_le_bytes()
        } else {
            coordinate.to_le_bytes()
        });
    }
    Origin {
        source: feature,
        role: format!("spline.trim.v1/{}", ContentHash::of(&bytes)),
    }
}

impl Solid {
    pub fn rust_extrude_spline_profile_named(
        profile: &CurvedProfile,
        height_mm: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::rust_extrude_spline_profile_with_interrupt(
            profile, height_mm, feature, origins, policy, None,
        )
    }
    pub fn rust_extrude_spline_profile_with_interrupt(
        profile: &CurvedProfile,
        height_mm: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        profile.frame.checked(policy)?;
        policy.length(height_mm.abs())?;
        if profile.outer.len() > 1024 {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Authored spline extrusion exceeds its certified source edge budget",
            ));
        }
        if profile.outer.is_empty()
            || profile.holes.len() > 1
            || profile
                .holes
                .iter()
                .any(|hole| !matches!(hole.as_slice(), [CurvedEdge2d::Circle { .. }]))
        {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Authored spline extrusion requires one exterior and at most one circular hole",
            ));
        }
        super::super::construction::validate_origins(
            feature,
            origins,
            profile.outer.len() + profile.holes.len(),
        )?;
        for edge in profile.outer.iter().chain(profile.holes.iter().flatten()) {
            cancelled(interrupt)?;
            edge.domain(policy)?;
            if let CurvedEdge2d::BSpline(spline) = edge {
                if spline.degree > 3 {
                    return Err(failure(
                        GeometryErrorKind::Unsupported,
                        "Spline extrusion currently supports degree one through three",
                    ));
                }
                if spline.degree > 1
                    && spline
                        .weights
                        .as_ref()
                        .is_some_and(|weights| weights.windows(2).any(|pair| pair[0] != pair[1]))
                {
                    return Err(failure(
                        GeometryErrorKind::Unsupported,
                        "This rational spline profile cannot yet be extruded",
                    ));
                }
            }
        }
        let negative = height_mm < 0.;
        let reflect = |[x, y]: [f64; 2]| [x, if negative { -y } else { y }];
        let source: Vec<_> = profile
            .outer
            .iter()
            .zip(origins)
            .map(|(edge, origin)| RegionEdge {
                authored: origin.source,
                reversed: false,
                fragment: None,
                curve: exact_curve(edge),
            })
            .collect();
        let queries = NativeRegionQueries::analytic(policy)?;
        let control = multicad_core::sketch_regions::QueryControl::new(interrupt);
        let checkpoint = || crate::sketch_queries::query_checkpoint(&control);
        let query_error = |error| {
            control
                .failure()
                .map(crate::sketch_queries::query_control_error)
                .unwrap_or_else(|| failure(GeometryErrorKind::Unsupported, error))
        };

        for (index, edge) in source.iter().enumerate() {
            cancelled(interrupt)?;
            let hits = queries
                .self_intersections_with_control(&edge.curve, policy.linear_tolerance_mm, &control)
                .map_err(query_error)?;
            if hits.overlap || !hits.points.is_empty() {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "Authored spline outline intersects itself",
                ));
            }
            for (other, earlier) in source[..index].iter().enumerate() {
                cancelled(interrupt)?;
                let mut joins = Vec::new();
                if other + 1 == index {
                    let curve = &profile.outer[other];
                    let domain = curve.domain(policy)?;
                    joins.push(curve.evaluate(domain[1], policy)?.point);
                }
                if other == 0 && index + 1 == source.len() {
                    let curve = &profile.outer[0];
                    let domain = curve.domain(policy)?;
                    joins.push(curve.evaluate(domain[0], policy)?.point);
                }
                let hits = queries
                    .intersections_with_control(
                        &edge.curve,
                        &earlier.curve,
                        policy.linear_tolerance_mm,
                        &control,
                    )
                    .map_err(query_error)?;
                if hits.overlap
                    || hits.points.iter().any(|point| {
                        !joins.iter().any(|join| {
                            (point[0] - join[0]).hypot(point[1] - join[1])
                                <= policy.linear_tolerance_mm * 0.5
                        })
                    })
                {
                    return Err(failure(
                        GeometryErrorKind::Input,
                        "Authored spline outline has crossing or overlapping source curves",
                    ));
                }
            }
        }
        let area =
            crate::sketch_queries::spline_regions::signed_area(&source, policy, Some(&checkpoint))
                .map_err(query_error)?;
        let reverse = (area > 0.) == negative;
        let hole = profile.holes.first().map(|hole| {
            let [CurvedEdge2d::Circle { center, radius_mm }] = hole.as_slice() else {
                unreachable!("validated circular hole");
            };
            AnalyticEdge::Circle {
                center: reflect(*center),
                radius_mm: *radius_mm,
                clockwise: true,
            }
        });
        for depth in 2..=8 {
            cancelled(interrupt)?;
            let mut edges = Vec::new();
            for (curve, origin) in profile.outer.iter().zip(origins) {
                cancelled(interrupt)?;
                match curve {
                    CurvedEdge2d::BSpline(spline) if spline.degree > 1 => {
                        for patch in crate::spline_edits::regions::polynomial_patches(
                            spline,
                            depth,
                            policy,
                            Some(&checkpoint),
                        )? {
                            let cubic = Cubic::new(
                                patch.poles.into_iter().map(reflect).collect(),
                                vec![0., 1.],
                                false,
                            )
                            .map_err(|cause| {
                                failure(
                                    GeometryErrorKind::Unsupported,
                                    format!("Cannot retain an authored spline span: {cause:?}"),
                                )
                            })?;
                            edges.push(AuthoredEdge {
                                edge: Edge::Cubic(cubic),
                                origins: aliases(feature, origin),
                                anchor: None,
                                fragment: true,
                            });
                        }
                    }
                    CurvedEdge2d::BSpline(spline) => {
                        for pair in spline.poles.windows(2) {
                            let [a, b] = [reflect(pair[0]), reflect(pair[1])];
                            let delta = [b[0] - a[0], b[1] - a[1]];
                            let length = delta[0].hypot(delta[1]);
                            policy.length(length)?;
                            edges.push(AuthoredEdge {
                                edge: Edge::Analytic(AnalyticEdge::Line { a, b }),
                                fragment: spline.poles.len() > 2,
                                origins: aliases(feature, origin),
                                anchor: Some(ReferenceFrame {
                                    origin_mm: [a[0], a[1], 0.],
                                    u: [delta[0] / length, delta[1] / length, 0.],
                                    v: [0., 0., 1.],
                                }),
                            });
                        }
                    }
                    CurvedEdge2d::Line { a, b } => {
                        let [a, b] = [reflect(*a), reflect(*b)];
                        let delta = [b[0] - a[0], b[1] - a[1]];
                        let length = delta[0].hypot(delta[1]);
                        policy.length(length)?;
                        edges.push(AuthoredEdge {
                            edge: Edge::Analytic(AnalyticEdge::Line { a, b }),
                            origins: aliases(feature, origin),
                            fragment: false,
                            anchor: Some(ReferenceFrame {
                                origin_mm: [a[0], a[1], 0.],
                                u: [delta[0] / length, delta[1] / length, 0.],
                                v: [0., 0., 1.],
                            }),
                        });
                    }
                    CurvedEdge2d::Arc {
                        center,
                        a,
                        b,
                        clockwise,
                    } => edges.push(AuthoredEdge {
                        edge: Edge::Analytic(AnalyticEdge::Arc {
                            center: reflect(*center),
                            a: reflect(*a),
                            b: reflect(*b),
                            clockwise: *clockwise != negative,
                        }),
                        origins: aliases(feature, origin),
                        fragment: false,
                        anchor: None,
                    }),
                    CurvedEdge2d::Circle { .. } => {
                        return Err(failure(
                            GeometryErrorKind::Unsupported,
                            "A full circular exterior cannot share a spline outline",
                        ));
                    }
                }
            }
            if edges.len() > 1024 {
                return Err(failure(
                    GeometryErrorKind::Limit,
                    "Authored spline extrusion exceeds its certified span budget",
                ));
            }
            if reverse {
                edges.reverse();
                for item in &mut edges {
                    item.edge = match &item.edge {
                        Edge::Analytic(edge) => Edge::Analytic(edge.reversed()),
                        Edge::Cubic(cubic) => Edge::Cubic(
                            Cubic::new(
                                cubic.poles().to_vec(),
                                cubic.knots().to_vec(),
                                !cubic.reversed(),
                            )
                            .expect("retained source cubic"),
                        ),
                    };
                }
            }
            for item in &mut edges {
                if item.fragment {
                    let points = match &item.edge {
                        Edge::Cubic(cubic) => {
                            let mut points = cubic.poles().to_vec();
                            if cubic.reversed() {
                                points.reverse();
                            }
                            points.into_iter().map(reflect).collect::<Vec<_>>()
                        }
                        Edge::Analytic(edge) => {
                            vec![reflect(edge.point(0.)), reflect(edge.point(1.))]
                        }
                    };
                    item.origins
                        .push(finite_trim(feature, &item.origins[1], &points));
                }
            }
            let outline = edges.iter().map(|edge| edge.edge.clone()).collect();
            let pure_interrupt = || checkpoint().is_err();
            let result = Sweep::new_with_interrupt(
                outline,
                hole,
                height_mm.abs(),
                policy.linear_tolerance_mm,
                Some(&pure_interrupt),
            );
            if let Some(cause) = control.failure() {
                return Err(crate::sketch_queries::query_control_error(cause));
            }
            let sweep = match result {
                Ok(sweep) => sweep,
                Err(ConstructionError::Unresolved) if depth < 8 => continue,
                Err(cause) => return Err(map_error(cause)),
            };
            let frame = PlaneFrame {
                v: profile
                    .frame
                    .v
                    .map(|value| if negative { -value } else { value }),
                ..profile.frame
            };
            let bounds = sweep.bounds();
            frame.checked_metric_extent(
                std::array::from_fn(|i| bounds[0][i].abs().max(bounds[1][i].abs())),
                policy,
            )?;
            let mut body = SplineSweepBody::new_controlled(
                feature,
                sweep,
                frame,
                policy.linear_tolerance_mm,
                Some(&checkpoint),
            )?;
            body.wall_origins = edges
                .iter()
                .map(|edge| edge.origins.clone())
                .chain(
                    profile
                        .holes
                        .first()
                        .map(|_| aliases(feature, &origins[profile.outer.len()])),
                )
                .collect();
            body.authored = Some(AuthoredFrames {
                reverse: negative,
                // The local L1 conversion enclosure is tolerance/128.
                // Double it for the admitted near-unit frame's spatial
                // operator bound before reserving the world chord budget.
                source_error_mm: policy.linear_tolerance_mm / 64.,
                walls: edges
                    .into_iter()
                    .map(|edge| edge.anchor)
                    .chain(hole.map(|_| None))
                    .collect(),
            });
            let bounds = body.bounds(frame);
            policy.point(bounds.min)?;
            policy.point(bounds.max)?;
            checkpoint()?;
            return Ok(Self {
                body: SolidBody::SplineSweep(Rc::new(body), frame),
                policy,
                provenance: vec![],
                rust_origins: vec![],
                retained_placement: None,
            });
        }
        unreachable!("final refinement returns its typed failure")
    }
}
