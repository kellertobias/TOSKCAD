//! Physical flat washer stacks on the two certified through-bore mouths.
use super::*;
use multicad_kernel::GeometryPolicy;

pub(super) struct Inputs<'a, 'solid> {
    pub placed: &'a [Hardware<'solid>],
    pub axis: ReferenceFrame,
    pub bore: rust_through::ThroughReceiverProof,
    pub grip: f64,
    pub head: [f64; 2],
    pub nut: [f64; 2],
    pub shaft: f64,
    pub policy: GeometryPolicy,
    pub connection: EntityId,
}

fn fail(input: &Inputs<'_, '_>, message: &str) -> ModelError {
    error(Some(input.connection), message)
}

fn dimensions(row: &Hardware<'_>, input: &Inputs<'_, '_>) -> Result<[f64; 3], ModelError> {
    let ResolvedMechanicalPart::Washer {
        bore_radius_mm,
        outside_radius_mm,
        thickness_mm,
        ..
    } = row.part
    else {
        return Err(fail(
            input,
            "A washer role must contain actual flat washer geometry",
        ));
    };
    let (ring, actual) = row.solid.rust_annulus_data().ok_or_else(|| {
        fail(
            input,
            "A washer role has no retained circular ring material",
        )
    })?;
    let dimensions = [bore_radius_mm, outside_radius_mm, thickness_mm];
    if dimensions
        != [
            ring.inner_radius_mm(),
            ring.outer_radius_mm(),
            ring.height_mm(),
        ]
    {
        return Err(fail(
            input,
            "Actual washer material differs from its current definition",
        ));
    }
    // A generated Part can still have a placement. Check its actual retained
    // chart against the current source mount, at the full physical extent.
    let expected = placed_frame(&row.part, row.pose);
    let distance = |a: [f64; 3], b: [f64; 3]| (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]);
    let normal = actual.normal();
    let residual = distance(actual.origin, expected.origin_mm)
        + outside_radius_mm * (distance(actual.u, expected.u) + distance(actual.v, expected.v))
        + thickness_mm * distance(normal, expected.normal());
    let tilt = cross(normal, input.axis.normal());
    let tilt = dot(tilt, tilt).sqrt();
    if !residual.is_finite()
        || residual > input.policy.linear_tolerance_mm / 8.
        || tilt * outside_radius_mm > input.policy.linear_tolerance_mm / 8.
        || dot(normal, input.axis.normal()) <= 0.
    {
        return Err(fail(
            input,
            "Actual washer placement cannot prove a flat bearing plane",
        ));
    }
    Ok(dimensions)
}

fn land(input: &Inputs<'_, '_>, a: [f64; 2], b: [f64; 2]) -> Result<(), ModelError> {
    let margin = input
        .policy
        .minimum_feature_mm
        .max(16. * input.policy.linear_tolerance_mm);
    // This complete annular land fits both adjacent materials. The reserve
    // exceeds the admitted frame and reconstruction errors on both sides.
    if a[1].min(b[1]) - a[0].max(b[0]) < 5. * margin {
        return Err(fail(
            input,
            "Bearing components have no complete overlapping flat annular land",
        ));
    }
    Ok(())
}

pub(super) fn prove(input: Inputs<'_, '_>) -> Result<[f64; 2], ModelError> {
    let mut sums = [0.; 2];
    for row in input.placed {
        let side = match row.role {
            Role::HeadWasher => 0,
            Role::NutWasher => 1,
            _ => continue,
        };
        sums[side] += dimensions(row, &input)?[2];
    }
    if sums.into_iter().any(|sum| !sum.is_finite()) {
        return Err(fail(
            &input,
            "Washer stack dimensions exceed the numerical budget",
        ));
    }
    let stock = [input.bore.radius_mm, input.bore.protected_radius_mm];
    let margin = input
        .policy
        .minimum_feature_mm
        .max(16. * input.policy.linear_tolerance_mm);
    for (role, mut station, mut previous) in [
        (Role::HeadWasher, -sums[0], input.head),
        (Role::NutWasher, input.grip, stock),
    ] {
        for row in input.placed.iter().filter(|row| row.role == role) {
            let [inner, outer, height] = dimensions(row, &input)?;
            let (_, actual) = row
                .solid
                .rust_annulus_data()
                .expect("checked retained ring");
            at(
                input.axis,
                ReferenceFrame {
                    origin_mm: actual.origin,
                    u: actual.u,
                    v: actual.v,
                },
                station,
                input.policy.linear_tolerance_mm / 8.,
                input.connection,
            )?;
            // A head washer must clear the neck blend as well as the plain
            // shank and thread crests. Nut washers only surround the shaft.
            let clearance = if role == Role::HeadWasher {
                input.head[0]
            } else {
                input.shaft
            };
            if inner - clearance < margin {
                return Err(fail(
                    &input,
                    "Washer bore interferes with the shaft or head neck",
                ));
            }
            land(&input, previous, [inner, outer])?;
            previous = [inner, outer];
            station += height;
        }
        land(
            &input,
            previous,
            if role == Role::HeadWasher {
                stock
            } else {
                input.nut
            },
        )?;
    }
    Ok(sums)
}
