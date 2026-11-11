//! STEP exchange of an analytic spur outline with certified cubic flanks.
use super::{
    cubic_trim::CubicTrim,
    normal_sweep::{EndTrims, ProfileEdge},
    *,
};
use crate::spur_gear::{Edge, HubExtension, Profile};

/// Maximum geometric deviation of a STEP spur flank, in millimeters.
/// The writer also honors smaller caller tolerances.
pub const SPUR_STEP_MAX_ERROR_MM: f64 = 1e-7;

pub fn write_spur_gear(
    profile: &Profile,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_spur_gear(profile, origin, u, v, tolerance_mm))
}

pub(super) fn encode(
    file: &mut StepFile,
    profile: &Profile,
    placement: Placement,
    tolerance_mm: f64,
) -> Result<Entity, StepError> {
    if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let tolerance = tolerance_mm.min(SPUR_STEP_MAX_ERROR_MM);
    let outline = profile
        .outline()
        .iter()
        .map(|edge| match edge.edge {
            Edge::Planar(edge) => Ok(ProfileEdge::Analytic(edge)),
            Edge::Involute(flank) => CubicTrim::flank(flank, tolerance).map(ProfileEdge::Cubic),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let extensions = profile.hub_extensions();
    if extensions.iter().any(Option::is_some) {
        let trims = encode_hub(file, profile, extensions, &placement)?;
        return normal_sweep::encode_rings_with_trims(
            file,
            &mut std::iter::once(outline),
            profile.height_mm(),
            &[],
            placement,
            false,
            trims,
        );
    }
    let bore = profile.bore().map(|bore| vec![ProfileEdge::Analytic(bore)]);
    normal_sweep::encode_rings(
        file,
        &mut std::iter::once(outline).chain(bore),
        profile.height_mm(),
        &[],
        placement,
        false,
    )
}

pub(super) fn encode_hub(
    file: &mut StepFile,
    profile: &Profile,
    extensions: [Option<HubExtension>; 2],
    placement: &Placement,
) -> Result<EndTrims, StepError> {
    let trim = |file: &mut StepFile, span: [f64; 2], radius| {
        CylindricalTrim::new(
            file,
            span.map(|z| placement.point_f64([0.; 2], z)),
            placement.normal,
            placement.u,
            radius,
        )
    };
    let bore = profile
        .bore()
        .and_then(crate::normal_sweep::Edge::radius_mm)
        .map(|radius| trim(file, profile.axial_bounds(), radius))
        .transpose()?;
    let mut result = EndTrims::default();
    result
        .faces
        .extend(bore.as_ref().map(|trim| trim.wall(file, false)));
    for (end, extension) in extensions.into_iter().enumerate() {
        let upper = end == 1;
        let bore_bound = bore.as_ref().map(|trim| trim.cap_bound(file, upper, false));
        let Some(extension) = extension else {
            result.holes[end] = bore_bound;
            continue;
        };
        let levels = if upper {
            [extension.gear_z_mm, extension.end_z_mm]
        } else {
            [extension.end_z_mm, extension.gear_z_mm]
        };
        let cylinder = trim(file, levels, extension.radius_mm)?;
        // The hub circle on the tooth band is an inner cap boundary.
        // Its sense follows the gear cap even though the cylinder ends
        // on the opposite side of that cap.
        result.holes[end] = Some(circular_bound(file, cylinder.rims[1 - end], !upper, false));
        let cap = [Some(cylinder.cap_bound(file, upper, true)), bore_bound]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        result.faces.push(cylinder.wall(file, true));
        result.faces.push(file.face(
            &cap,
            cylinder.centers[end],
            placement.normal.map(|n| if upper { n } else { -n }),
            placement.u,
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
