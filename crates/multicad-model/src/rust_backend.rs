//! Canonical model providers for the bounded Rust solid backend.
use crate::{ModelError, compiler::error};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityData, EntityId},
    model::{
        self, Extent, ExtrudeParameters, PrimitiveParameters, ResolvedExtent,
        ResolvedExtrudeControls, ResolvedTermination, Termination,
    },
    parameters::{EvaluatedParameters, Unit},
    references::Origin,
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform, Solid,
    rust_kernel::{
        Circle, Error as ExactError, Rational,
        cylinder::AxialCylinder,
        parse_length_mm,
        polygon::{PolygonError, PolygonPrism, signed_area},
        prism::Prism,
    },
};

pub(crate) fn validate_document(
    document: &Document,
    scope: Option<&std::collections::BTreeSet<EntityId>>,
) -> Result<(), ModelError> {
    for entity in document.entities() {
        if scope.is_some_and(|needed| !needed.contains(&entity.id)) {
            continue;
        }
        match &entity.data {
            EntityData::Feature {
                definition,
                suppressed: false,
                ..
            } if !matches!(
                (definition.type_id.as_str(), definition.schema_version),
                (model::EXTRUDE_TYPE, model::EXTRUDE_SCHEMA)
                    | (model::REVOLVE_TYPE, model::REVOLVE_SCHEMA)
                    | (model::PRIMITIVE_TYPE, model::PRIMITIVE_SCHEMA)
                    | (model::BOOLEAN_TYPE, model::BOOLEAN_SCHEMA)
                    | (model::TRANSFORM_TYPE, model::TRANSFORM_SCHEMA)
                    | (model::ALIGN_TYPE, model::ALIGN_SCHEMA)
                    | (model::SLAB_TYPE, 1)
                    | (model::STAIR_TYPE, 1)
                    | (model::CONNECTION_RECEIVER_TYPE, 1)
                    | (
                        model::WALL_TYPE,
                        model::WALL_SCHEMA | model::LAYERED_WALL_SCHEMA
                    )
                    | (model::THREAD_TYPE, model::THREAD_SCHEMA)
                    | (model::MECHANICAL_PART_TYPE, model::MECHANICAL_PART_SCHEMA)
                    | (model::EDGE_TREATMENT_TYPE, model::EDGE_TREATMENT_SCHEMA)
                    | (
                        model::PANEL_TYPE,
                        model::PANEL_SCHEMA | model::RECIPE_PANEL_SCHEMA
                    )
                    | (model::PANEL_JOINT_CUT_TYPE, model::PANEL_JOINT_CUT_SCHEMA)
                    | (model::LOOSE_TENON_TYPE, model::LOOSE_TENON_SCHEMA)
                    | (model::MACHINING_CUT_TYPE, model::MACHINING_CUT_SCHEMA)
                    | (model::SHEET_TYPE, model::SHEET_SCHEMA)
                    | (model::HARDWARE_TYPE, model::HARDWARE_SCHEMA)
                    | (
                        model::SHEET_PROFILE_CUT_TYPE,
                        model::SHEET_PROFILE_CUT_SCHEMA
                    )
                    | (model::MEMBER_TYPE, model::MEMBER_SCHEMA)
                    | (model::imports::STEP_BODY_TYPE, 1)
            ) =>
            {
                return Err(error(
                    Some(entity.id),
                    format!(
                        "Rust backend has no provider for {} schema {}",
                        definition.type_id, definition.schema_version
                    ),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

// Preserve decimal or simple fraction source, including one bound parameter.
// Do not reconstruct a fractional authored dimension from its f64 result.
fn exact_length(
    dimension: &Dimension,
    parameters: &EvaluatedParameters,
) -> Result<Rational, String> {
    fn expression(text: &str, unit: &str) -> Result<Rational, String> {
        let text = text.trim().strip_prefix('=').unwrap_or(text.trim()).trim();
        parse_length_mm(text)
            .or_else(|_| parse_length_mm(&format!("{text} {unit}")))
            .map_err(|e| {
                format!("Rust backend needs an exact decimal or simple fractional length: {e:?}")
            })
    }
    match dimension {
        Dimension::Literal { value, unit } => expression(&value.to_string(), unit),
        Dimension::Expression { expression: source } => match source.bindings.as_slice() {
            [] => expression(&source.text, &source.unit),
            [binding]
                if source
                    .text
                    .trim()
                    .trim_start_matches('=')
                    .trim()
                    .strip_prefix('$')
                    == Some(binding.alias.as_str()) =>
            {
                let Some(EntityData::Parameter {
                    expression: text,
                    unit,
                    inputs,
                    ..
                }) = parameters
                    .document
                    .get(binding.target)
                    .map(|entry| &entry.data)
                else {
                    return Err("Rust backend could not resolve the bound length parameter".into());
                };
                if !inputs.is_empty() {
                    return Err(
                        "Rust backend requires a direct decimal or fractional parameter".into(),
                    );
                }
                expression(text, unit)
            }
            _ => Err("Rust backend does not yet evaluate compound length formulas".into()),
        },
    }
}

fn coordinate(value: f64) -> Result<Rational, String> {
    // A solved sketch exposes floating-point coordinates. Use their round-trip
    // decimal representation and require exact agreement of the profile joins.
    parse_length_mm(&format!("{value} mm"))
        .map_err(|e| format!("Sketch coordinate exceeds the Rust exact geometry range: {e:?}"))
}

fn profile_circle(center: [f64; 2], radius: f64) -> Result<Circle, String> {
    let [x, y] = center;
    Circle::new([coordinate(x)?, coordinate(y)?], coordinate(radius)?)
        .map_err(|e| format!("Rust profile circle failed: {e:?}"))
}

struct StraightSweep {
    height: Rational,
    start_offset: f64,
    reverse: bool,
}

fn straight_sweep(
    source: &ExtrudeParameters,
    controls: &ResolvedExtrudeControls,
    parameters: &EvaluatedParameters,
) -> Result<StraightSweep, String> {
    let [u, v, normal] = controls.direction;
    if u != 0. || v != 0. || normal.abs() != 1. || controls.taper_rad != 0. {
        return Err("Rust straight extrusion requires a normal sweep without taper".into());
    }
    let range_error = |e| format!("Rust extrusion extent exceeds the exact length range: {e:?}");
    let length = |dimension| exact_length(dimension, parameters);
    let zero = Rational::new(0, 1).expect("zero is representable");
    let (height, backward) = match &source.extent {
        Extent::OneSided {
            end: Termination::Distance { distance },
        } => {
            let signed = length(distance)?;
            let height = if signed < zero {
                zero.checked_sub(signed).map_err(range_error)?
            } else {
                signed
            };
            (height, zero)
        }
        Extent::Symmetric { total } => {
            let total = length(total)?;
            let two = Rational::new(2, 1).expect("two is representable");
            (total, total.checked_div(two).map_err(range_error)?)
        }
        Extent::TwoSided {
            forward: Termination::Distance { distance: forward },
            backward: Termination::Distance { distance: backward },
        } => {
            let forward = length(forward)?;
            let backward = length(backward)?;
            (
                forward.checked_add(backward).map_err(range_error)?,
                backward,
            )
        }
        _ => return Err("Rust straight extrusion requires distance terminations".into()),
    };
    let reverse = normal < 0.;
    let start_offset = match length(&source.start_offset) {
        Ok(offset) => {
            let shifted = if reverse {
                offset.checked_add(backward)
            } else {
                offset.checked_sub(backward)
            };
            shifted.map_err(range_error)?.as_f64()
        }
        // Preserve start offsets that use the full parameter evaluator.
        Err(_) => controls.start_offset_mm - normal * backward.as_f64(),
    };
    Ok(StraightSweep {
        height,
        start_offset,
        reverse,
    })
}

fn circular_prism(
    id: EntityId,
    profile: &CurvedProfile,
    center_uv: [f64; 2],
    radius_mm: f64,
    origin: &Origin,
    sweep: StraightSweep,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let fail = |message: String| error(Some(id), message);
    let radius = coordinate(radius_mm).map_err(fail)?;
    let normal = profile.frame.normal();
    let center = profile.frame.point(center_uv);
    let frame = PlaneFrame {
        origin: std::array::from_fn(|i| center[i] + normal[i] * sweep.start_offset),
        u: profile.frame.u,
        v: if sweep.reverse {
            profile.frame.v.map(|axis| -axis)
        } else {
            profile.frame.v
        },
    };
    // Retain exact world coordinates for upward XY cylinders so the existing
    // exact through-cut can use them. Other sweeps use a rigid placement of
    // the same analytic axial body.
    let zero = Rational::new(0, 1).expect("zero is representable");
    let [x, y, bottom] = if frame.u[2] == 0. && frame.v[2] == 0. && frame.normal()[2] > 0. {
        match frame.origin.map(coordinate) {
            [Ok(x), Ok(y), Ok(z)] => [x, y, z],
            _ => [zero; 3],
        }
    } else {
        [zero; 3]
    };
    let circle = Circle::new([x, y], radius)
        .map_err(|e| fail(format!("Rust circular extrusion profile failed: {e:?}")))?;
    let cylinder = AxialCylinder::new(id.uuid_bits(), circle, bottom, sweep.height)
        .map_err(|e| fail(format!("Rust circular extrusion failed: {e:?}")))?;
    Solid::from_axial_cylinder_sweep(
        cylinder,
        frame,
        center_uv,
        sweep.reverse,
        origin.clone(),
        policy,
    )
    .map_err(|e| error(Some(id), e))
}

fn straight_prism(
    id: EntityId,
    profile: &CurvedProfile,
    origins: &[Origin],
    sweep: StraightSweep,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let fail = |message: String| error(Some(id), message);
    let mut frame = profile.frame;
    let normal = frame.normal();
    frame.origin = std::array::from_fn(|i| frame.origin[i] + normal[i] * sweep.start_offset);
    let straight = || {
        profile
            .holes
            .iter()
            .flatten()
            .all(|edge| matches!(edge, CurvedEdge2d::Line { .. }))
    };
    let linear = || {
        // Derived sketch coordinates can exhaust rational intermediate
        // products even when their planar geometry is well-conditioned.
        let displacement = sweep.height.as_f64() * if sweep.reverse { -1. } else { 1. };
        let mut profile = profile.clone();
        profile.frame = frame;
        Solid::rust_extrude_linear_named(
            &profile,
            normal.map(|value| value * displacement),
            0.,
            id,
            origins,
            policy,
        )
        .map_err(|cause| error(Some(id), cause))
    };
    let mut rings = Vec::with_capacity(1 + profile.holes.len());
    let mut circles = vec![];
    let mut circle_origins = vec![];
    let mut side_origins = Vec::with_capacity(origins.len());
    let mut reversed = Vec::with_capacity(origins.len());
    let mut offset = 0;
    for (index, ring) in std::iter::once(&profile.outer)
        .chain(&profile.holes)
        .enumerate()
    {
        let ring_origins = origins
            .get(offset..offset + ring.len())
            .ok_or_else(|| fail("Rust extrusion is missing authored boundary identities".into()))?;
        offset += ring.len();
        if index > 0
            && let [CurvedEdge2d::Circle { center, radius_mm }] = ring.as_slice()
        {
            circles.push(profile_circle(*center, *radius_mm).map_err(fail)?);
            circle_origins.extend_from_slice(ring_origins);
            continue;
        }
        let mut points = Vec::with_capacity(ring.len());
        for (edge, next) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let (CurvedEdge2d::Line { a, b }, CurvedEdge2d::Line { a: next, .. }) = (edge, next)
            else {
                return Err(fail(
                    "Rust polygon extrusion requires straight segments or whole circular inner loops"
                        .into(),
                ));
            };
            if b != next {
                return Err(fail(
                    "Rust polygon extrusion requires closed boundary joins".into(),
                ));
            }
            points.push([
                coordinate(a[0]).map_err(fail)?,
                coordinate(a[1]).map_err(fail)?,
            ]);
        }
        let area = match signed_area(&points) {
            Ok(area) => area,
            Err(PolygonError::Arithmetic(ExactError::OutOfRange)) if straight() => return linear(),
            Err(cause) => return Err(fail(format!("Rust polygon winding failed: {cause:?}"))),
        };
        let reverse = (area.ratio().0 < 0) != (index > 0);
        if reverse {
            // Keep the first vertex and reverse the edges that start there.
            // Their authored directions are retained separately for attachment frames.
            points[1..].reverse();
            side_origins.extend(ring_origins.iter().rev().cloned());
        } else {
            side_origins.extend_from_slice(ring_origins);
        }
        reversed.extend(std::iter::repeat_n(reverse, ring.len()));
        rings.push(points);
    }
    if offset != origins.len() {
        return Err(fail(
            "Rust extrusion has unmatched authored boundary identities".into(),
        ));
    }
    reversed.resize(origins.len(), false);
    side_origins.extend(circle_origins);
    let polygon = match PolygonPrism::with_circular_holes(rings, circles, sweep.height) {
        Ok(polygon) => polygon,
        Err(PolygonError::Arithmetic(ExactError::OutOfRange)) if straight() => return linear(),
        Err(cause) => return Err(fail(format!("Rust polygon extrusion failed: {cause:?}"))),
    };
    Solid::from_polygon_prism_sweep(
        polygon,
        frame,
        sweep.reverse,
        id,
        &side_origins,
        &reversed,
        policy,
    )
    .map_err(|e| error(Some(id), e))
}

fn rounded_dimensions(profile: &CurvedProfile) -> Result<(Rational, Rational), String> {
    let unsupported = || {
        "Rust extrusion requires three straight sides and a right semicircle at the XY origin"
            .to_string()
    };
    if profile.frame != PlaneFrame::default() || profile.outer.len() != 4 {
        return Err(unsupported());
    }
    let Some((center, lower, upper)) = profile.outer.iter().find_map(|edge| match *edge {
        CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise,
        } => Some(if clockwise {
            (center, b, a)
        } else {
            (center, a, b)
        }),
        _ => None,
    }) else {
        return Err(unsupported());
    };
    let [length, radius] = center;
    let (length_exact, radius_exact) = (coordinate(length)?, coordinate(radius)?);
    let width = radius_exact
        .checked_add(radius_exact)
        .map_err(|e| format!("Rounded profile width exceeds exact geometry range: {e:?}"))?
        .as_f64();
    if lower != [length, 0.] || upper != [length, width] {
        return Err(unsupported());
    }
    for (from, to) in [
        ([0., 0.], lower),
        (upper, [0., width]),
        ([0., width], [0., 0.]),
    ] {
        if !profile.outer.iter().any(|edge| {
            matches!(*edge,
            CurvedEdge2d::Line { a, b } if (a == from && b == to) || (b == from && a == to))
        }) {
            return Err(unsupported());
        }
    }
    Ok((length_exact, radius_exact))
}

fn sweep_distances(
    id: EntityId,
    controls: &ResolvedExtrudeControls,
) -> Result<(f64, f64), ModelError> {
    use ResolvedTermination::Distance;

    let (length, backward) = match controls.extent {
        ResolvedExtent::OneSided {
            end: Distance { distance_mm },
        } => (distance_mm, 0.),
        ResolvedExtent::Symmetric { total_mm } if controls.taper_rad == 0. => {
            (total_mm, total_mm / 2.)
        }
        ResolvedExtent::TwoSided {
            forward: Distance { distance_mm: front },
            backward: Distance { distance_mm: back },
        } if controls.taper_rad == 0. => (front + back, back),
        ResolvedExtent::Symmetric { .. }
        | ResolvedExtent::TwoSided {
            forward: Distance { .. },
            backward: Distance { .. },
        } => {
            return Err(error(
                Some(id),
                "Rust symmetric and two-sided draft requires an intermediate sweep section",
            ));
        }
        _ => {
            return Err(error(
                Some(id),
                "Rust directed and tapered extrusion requires distance terminations",
            ));
        }
    };
    Ok((length, backward))
}

fn linear_sweep(
    id: EntityId,
    controls: &ResolvedExtrudeControls,
    mut profile: CurvedProfile,
    origins: &[Origin],
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let (length, backward) = sweep_distances(id, controls)?;
    let normal = profile.frame.normal();
    let [u, v, n] = controls.direction;
    let direction: [f64; 3] =
        std::array::from_fn(|i| profile.frame.u[i] * u + profile.frame.v[i] * v + normal[i] * n);
    profile.frame.origin = std::array::from_fn(|i| {
        profile.frame.origin[i] + normal[i] * controls.start_offset_mm - direction[i] * backward
    });
    Solid::rust_extrude_linear_named(
        &profile,
        direction.map(|component| component * length),
        controls.taper_rad,
        id,
        origins,
        policy,
    )
    .map_err(|cause| error(Some(id), cause))
}

pub(crate) fn extrude(
    id: EntityId,
    source: &ExtrudeParameters,
    controls: &ResolvedExtrudeControls,
    mut profile: CurvedProfile,
    origins: &[Origin],
    parameters: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let fail = |message: String| error(Some(id), message);
    if controls.taper_rad != 0. || controls.direction[0] != 0. || controls.direction[1] != 0. {
        return linear_sweep(id, controls, profile, origins, policy);
    }
    if !has_partial_arcs(&profile)
        && (matches!(profile.outer.as_slice(), [CurvedEdge2d::Circle { .. }])
            || profile
                .outer
                .iter()
                .all(|edge| matches!(edge, CurvedEdge2d::Line { .. })))
    {
        let sweep = straight_sweep(source, controls, parameters).map_err(fail)?;
        return normal_profile(id, &profile, origins, sweep, policy);
    }
    if let Extent::OneSided {
        end: Termination::Distance { distance },
    } = &source.extent
        && controls.direction == [0., 0., 1.]
        && controls.start_offset_mm == 0.
        && let Ok(height) = exact_length(distance, parameters)
        && height.as_f64() > 0.
        && let Ok((length, radius)) = rounded_dimensions(&profile)
        && profile
            .holes
            .iter()
            .all(|hole| matches!(hole.as_slice(), [CurvedEdge2d::Circle { .. }]))
    {
        let mut prism = Prism::extrude(id.uuid_bits(), length, radius, height)
            .map_err(|e| fail(format!("Rust extrusion failed: {e:?}")))?;
        for (index, hole) in profile.holes.iter().enumerate() {
            let [CurvedEdge2d::Circle { center, radius_mm }] = hole.as_slice() else {
                unreachable!("validated circular inner loops");
            };
            let origin = origins.get(profile.outer.len() + index).ok_or_else(|| {
                fail("Rust extrusion is missing the authored inner-loop identity".into())
            })?;
            let circle = profile_circle(*center, *radius_mm).map_err(fail)?;
            prism = prism
                .with_inner_loop(origin.source.uuid_bits(), circle)
                .map_err(|e| fail(format!("Rust inner loop failed: {e:?}")))?;
        }
        return Solid::from_rust_prism(prism, policy).map_err(|e| error(Some(id), e));
    }
    let (length, backward) = sweep_distances(id, controls)?;
    let sign = controls.direction[2];
    let normal = profile.frame.normal();
    profile.frame.origin = std::array::from_fn(|i| {
        profile.frame.origin[i] + normal[i] * (controls.start_offset_mm - sign * backward)
    });
    Solid::rust_extrude_normal_profile_named(&profile, sign * length, id, origins, policy)
        .map_err(|cause| error(Some(id), cause))
}

pub(crate) fn stock_profile(
    id: EntityId,
    profile: &CurvedProfile,
    origins: &[Origin],
    height_mm: f64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    if has_partial_arcs(profile) {
        return Solid::rust_extrude_normal_profile_named(profile, height_mm, id, origins, policy)
            .map_err(|cause| error(Some(id), cause));
    }
    let sweep = StraightSweep {
        height: coordinate(height_mm.abs()).map_err(|cause| error(Some(id), cause))?,
        start_offset: 0.,
        reverse: height_mm < 0.,
    };
    normal_profile(id, profile, origins, sweep, policy)
}

fn has_partial_arcs(profile: &CurvedProfile) -> bool {
    profile
        .outer
        .iter()
        .chain(profile.holes.iter().flatten())
        .any(|edge| matches!(edge, CurvedEdge2d::Arc { .. }))
}

fn normal_profile(
    id: EntityId,
    profile: &CurvedProfile,
    origins: &[Origin],
    sweep: StraightSweep,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let [CurvedEdge2d::Circle { center, radius_mm }] = profile.outer.as_slice() else {
        return straight_prism(id, profile, origins, sweep, policy);
    };
    match (profile.holes.as_slice(), origins) {
        ([], [origin]) => circular_prism(id, profile, *center, *radius_mm, origin, sweep, policy),
        ([hole], [outer, inner])
            if let [
                CurvedEdge2d::Circle {
                    center: bore_center,
                    radius_mm: bore_radius,
                },
            ] = hole.as_slice() =>
        {
            let [dx, dy] = std::array::from_fn(|i| bore_center[i] - center[i]);
            if dx.hypot(dy) > policy.linear_tolerance_mm {
                return Err(error(
                    Some(id),
                    "Rust annular extrusion requires concentric circles",
                ));
            }
            let normal = profile.frame.normal();
            let start = PlaneFrame {
                origin: std::array::from_fn(|i| {
                    profile.frame.origin[i] + normal[i] * sweep.start_offset
                }),
                ..profile.frame
            };
            let sign = if sweep.reverse { -1. } else { 1. };
            Solid::rust_annulus_from_sketch(
                [*bore_radius, *radius_mm],
                sign * sweep.height.as_f64(),
                id,
                start,
                *center,
                [inner.clone(), outer.clone()],
                policy,
            )
            .map_err(|cause| error(Some(id), cause))
        }
        _ => Err(error(
            Some(id),
            "Rust circular extrusion requires one circle or a concentric pair with authored boundary identities",
        )),
    }
}

pub(crate) fn primitive(
    id: EntityId,
    source: &PrimitiveParameters,
    parameters: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let fail = |message: String| error(Some(id), message);
    if let PrimitiveParameters::Box { size, frame } = source {
        let resolved = frame.resolve(parameters).map_err(|e| error(Some(id), e))?;
        let [length, width, height] = size.each_ref().map(|d| primitive_length(d, parameters));
        let placement = PlaneFrame {
            origin: resolved.origin,
            u: frame.u,
            v: frame.v,
        };
        let exact = [
            length.map_err(fail)?,
            width.map_err(fail)?,
            height.map_err(fail)?,
        ];
        let build = |size| Solid::rust_box(id.uuid_bits(), size, placement, policy);
        return build(exact)
            .or_else(|cause| {
                let compact = std::array::from_fn(|i| {
                    if matches!(size[i], Dimension::Literal { .. }) {
                        compact_literal_mm(exact[i])
                    } else {
                        exact[i]
                    }
                });
                if compact == exact {
                    return Err(cause);
                }
                build(compact).map_err(|_| cause)
            })
            .map_err(|e| error(Some(id), e));
    }
    let PrimitiveParameters::Cylinder {
        radius,
        height,
        frame,
    } = source
    else {
        return Err(fail(
            "Rust backend supports only box and axial cylinder primitives".into(),
        ));
    };
    let resolved = frame.resolve(parameters).map_err(|e| error(Some(id), e))?;
    let placement = PlaneFrame {
        origin: resolved.origin,
        u: frame.u,
        v: frame.v,
    };
    let normal = placement.normal();
    let axial = placement.u[2] == 0. && placement.v[2] == 0. && normal[2] > 0.;
    // Upward XY cylinders retain exact world coordinates for the axial
    // Boolean path. Other orientations place the same exact local solid.
    let zero = Rational::new(0, 1).expect("zero is representable");
    let [x, y, bottom] = if axial {
        let [x, y, z] = [0, 1, 2].map(|i| {
            exact_length(&frame.origin[i], parameters).or_else(|_| coordinate(resolved.origin[i]))
        });
        [x.map_err(fail)?, y.map_err(fail)?, z.map_err(fail)?]
    } else {
        [zero; 3]
    };
    let circle = Circle::new([x, y], primitive_length(radius, parameters).map_err(fail)?)
        .map_err(|e| fail(format!("Rust cylinder circle failed: {e:?}")))?;
    let cylinder = AxialCylinder::new(
        id.uuid_bits(),
        circle,
        bottom,
        primitive_length(height, parameters).map_err(fail)?,
    )
    .map_err(|e| fail(format!("Rust cylinder failed: {e:?}")))?;
    let solid = Solid::from_axial_cylinder(
        cylinder,
        if axial {
            PlaneFrame {
                origin: [x, y, bottom].map(Rational::as_f64),
                ..placement
            }
        } else {
            PlaneFrame::default()
        },
        policy,
    )
    .map_err(|e| error(Some(id), e))?;
    if axial {
        return Ok(solid);
    }
    solid
        .transformed(RigidTransform {
            rotation: std::array::from_fn(|i| [placement.u[i], placement.v[i], normal[i]]),
            translation_mm: placement.origin,
        })
        .map_err(|e| error(Some(id), e))
}

fn primitive_length(
    dimension: &Dimension,
    parameters: &EvaluatedParameters,
) -> Result<Rational, String> {
    exact_length(dimension, parameters).or_else(|_| {
        let value = dimension
            .value_in(Unit::Millimeter, Some(parameters))
            .map_err(|cause| cause.to_string())?;
        coordinate(value).map(compact_literal_mm)
    })
}

fn compact_literal_mm(exact: Rational) -> Rational {
    // A generated floating-point literal can have a long decimal tail.
    // Bound the retry to roundoff rather than the coarser modeling tolerance.
    let value = exact.as_f64();
    let budget = 2. * f64::EPSILON * value.abs();
    (0..=15)
        .filter_map(|digits| parse_length_mm(&format!("{value:.digits$} mm")).ok())
        .find(|candidate| (candidate.as_f64() - value).abs() <= budget)
        .unwrap_or(exact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_sweep_keeps_parameter_offsets_and_rejects_unsupported_controls() {
        use multicad_core::parameters::{BoundExpression, evaluate};

        let document = Document::new("Straight extrusion controls").unwrap();
        let values = evaluate(&document).unwrap();
        let mut source = ExtrudeParameters {
            start_offset: Dimension::Expression {
                expression: BoundExpression::bind(&document, "=1 mm + 2 mm", "mm", None).unwrap(),
            },
            extent: Extent::Symmetric {
                total: model::length(4.),
            },
            ..Default::default()
        };
        let controls = source.resolve_controls(&values).unwrap();
        let sweep = straight_sweep(&source, &controls, &values).unwrap();
        assert_eq!(sweep.height.ratio(), (4, 1));
        assert_eq!(sweep.start_offset, 1.);
        assert!(!sweep.reverse);

        source.direction.elevation = model::angle(45.);
        let controls = source.resolve_controls(&values).unwrap();
        let unsupported = straight_sweep(&source, &controls, &values).err().unwrap();
        assert!(unsupported.contains("normal sweep"));

        source.direction = Default::default();
        source.extent = Extent::OneSided {
            end: Termination::ToPlane {
                plane: EntityId::new(),
                offset: model::length(0.),
            },
        };
        let controls = source.resolve_controls(&values).unwrap();
        let unsupported = straight_sweep(&source, &controls, &values).err().unwrap();
        assert!(unsupported.contains("distance terminations"));
    }

    #[test]
    fn straight_prism_refuses_open_joins_and_partial_curves() {
        let line = |a, b| CurvedEdge2d::Line { a, b };
        let id = EntityId::new();
        let origins = std::array::from_fn::<_, 4, _>(|_| Origin {
            source: EntityId::new(),
            role: "profile.edge".into(),
        });
        let mut profile = CurvedProfile {
            frame: PlaneFrame::default(),
            outer: vec![
                line([0., 0.], [5., 0.]),
                line([5., 0.], [0., 5.]),
                line([0., 5.], [0., 0.01]),
            ],
            holes: vec![],
        };
        let build = |profile: &CurvedProfile, origins: &[Origin]| {
            straight_prism(
                id,
                profile,
                origins,
                StraightSweep {
                    height: Rational::new(2, 1).unwrap(),
                    start_offset: 0.,
                    reverse: false,
                },
                GeometryPolicy::default(),
            )
            .err()
            .unwrap()
        };
        assert!(
            build(&profile, &origins[..3])
                .message
                .contains("closed boundary joins")
        );
        profile.outer[2] = line([0., 5.], [0., 0.]);
        profile.holes.push(vec![CurvedEdge2d::Arc {
            center: [1., 1.],
            a: [1.5, 1.],
            b: [1., 1.5],
            clockwise: false,
        }]);
        assert!(
            build(&profile, &origins)
                .message
                .contains("straight segments")
        );
    }
}
