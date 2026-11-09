//! Canonical model providers for the bounded Rust solid backend.
use crate::{ModelError, compiler::error};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityData, EntityId},
    model::{
        self, Extent, ExtrudeParameters, PrimitiveParameters, ResolvedExtrudeControls, Termination,
    },
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder, parse_length_mm, prism::Prism},
};

pub(crate) fn validate_document(document: &Document) -> Result<(), ModelError> {
    for entity in document.entities() {
        match &entity.data {
            EntityData::Instance { .. } => {
                return Err(error(
                    Some(entity.id),
                    "Rust solid occurrences are unavailable",
                ));
            }
            EntityData::Assembly { instances } if !instances.is_empty() => {
                return Err(error(
                    Some(entity.id),
                    "Rust solid assemblies are unavailable",
                ));
            }
            EntityData::Feature {
                definition,
                suppressed: false,
                ..
            } if !matches!(
                (definition.type_id.as_str(), definition.schema_version),
                (model::EXTRUDE_TYPE, model::EXTRUDE_SCHEMA)
                    | (model::PRIMITIVE_TYPE, model::PRIMITIVE_SCHEMA)
                    | (model::BOOLEAN_TYPE, model::BOOLEAN_SCHEMA)
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

pub(crate) fn extrude(
    id: EntityId,
    source: &ExtrudeParameters,
    controls: &ResolvedExtrudeControls,
    profile: &CurvedProfile,
    origins: &[Origin],
    parameters: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let fail = |message: String| error(Some(id), message);
    let Extent::OneSided {
        end: Termination::Distance { distance },
    } = &source.extent
    else {
        return Err(fail("Rust extrusion requires a one-sided distance".into()));
    };
    if controls.direction != [0., 0., 1.]
        || controls.start_offset_mm != 0.
        || controls.taper_rad != 0.
    {
        return Err(fail(
            "Rust extrusion requires a positive normal sweep without offset or taper".into(),
        ));
    }
    let height = exact_length(distance, parameters).map_err(fail)?;
    let (length, radius) = rounded_dimensions(profile).map_err(fail)?;
    let mut prism = Prism::extrude(id.uuid_bits(), length, radius, height)
        .map_err(|e| fail(format!("Rust extrusion failed: {e:?}")))?;
    for (index, hole) in profile.holes.iter().enumerate() {
        let [
            CurvedEdge2d::Circle {
                center: [x, y],
                radius_mm,
            },
        ] = hole.as_slice()
        else {
            return Err(fail(
                "Rust extrusion supports only whole circular inner loops".into(),
            ));
        };
        let origin = origins.get(profile.outer.len() + index).ok_or_else(|| {
            fail("Rust extrusion is missing the authored inner-loop identity".into())
        })?;
        let circle = Circle::new(
            [coordinate(*x).map_err(fail)?, coordinate(*y).map_err(fail)?],
            coordinate(*radius_mm).map_err(fail)?,
        )
        .map_err(|e| fail(format!("Rust profile circle failed: {e:?}")))?;
        prism = prism
            .with_inner_loop(origin.source.uuid_bits(), circle)
            .map_err(|e| fail(format!("Rust inner loop failed: {e:?}")))?;
    }
    Solid::from_rust_prism(prism, policy).map_err(|e| error(Some(id), e))
}

pub(crate) fn primitive(
    id: EntityId,
    source: &PrimitiveParameters,
    parameters: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let fail = |message: String| error(Some(id), message);
    if let PrimitiveParameters::Box { size, frame } = source {
        frame.validate().map_err(|e| error(Some(id), e))?;
        let [x, y, z] = frame.origin.each_ref().map(|d| exact_length(d, parameters));
        let [length, width, height] = size.each_ref().map(|d| exact_length(d, parameters));
        let placement = PlaneFrame {
            origin: [x.map_err(fail)?, y.map_err(fail)?, z.map_err(fail)?].map(Rational::as_f64),
            u: frame.u,
            v: frame.v,
        };
        return Solid::rust_box(
            id.uuid_bits(),
            [
                length.map_err(fail)?,
                width.map_err(fail)?,
                height.map_err(fail)?,
            ],
            placement,
            policy,
        )
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
    frame.validate().map_err(|e| error(Some(id), e))?;
    let [x, y, bottom] =
        [&frame.origin[0], &frame.origin[1], &frame.origin[2]].map(|d| exact_length(d, parameters));
    let circle = Circle::new(
        [x.map_err(fail)?, y.map_err(fail)?],
        exact_length(radius, parameters).map_err(fail)?,
    )
    .map_err(|e| fail(format!("Rust cylinder circle failed: {e:?}")))?;
    let cylinder = AxialCylinder::new(
        id.uuid_bits(),
        circle,
        bottom.map_err(fail)?,
        exact_length(height, parameters).map_err(fail)?,
    )
    .map_err(|e| fail(format!("Rust cylinder failed: {e:?}")))?;
    let [x, y] = circle.center().map(Rational::as_f64);
    let placement = PlaneFrame {
        origin: [x, y, cylinder.bottom().as_f64()],
        u: frame.u,
        v: frame.v,
    };
    Solid::from_axial_cylinder(cylinder, placement, policy).map_err(|e| error(Some(id), e))
}
