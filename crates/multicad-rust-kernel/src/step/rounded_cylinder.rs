//! Plane/cylinder/quarter-toroid BREP with shared trim and periodic seam edges.
use super::*;
use crate::rounded_cylinder::RoundedCylinder;
use std::f64::consts::TAU;

pub fn write_rounded_cylinder(
    geometry: &RoundedCylinder,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_rounded_cylinder(geometry, origin, u, v, tolerance_mm))
}

pub(super) fn encode(
    file: &mut StepFile,
    g: &RoundedCylinder,
    p: Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let norm = |[x, y, z]: Point| x.hypot(y).hypot(z);
    let unit = |v: Point| {
        let n = norm(v);
        v.map(|x| x / n)
    };
    let normal = unit(p.normal);
    let v = unit(cross(normal, unit(p.u)));
    let u = cross(v, normal);
    let residual = norm(std::array::from_fn(|i| p.u[i] - u[i])) * g.radius_mm()
        + norm(std::array::from_fn(|i| p.v[i] - v[i])) * g.radius_mm()
        + norm(std::array::from_fn(|i| p.normal[i] - normal[i])) * g.height_mm();
    let magnitude =
        p.origin.into_iter().map(f64::abs).fold(0., f64::max) + g.radius_mm() + g.height_mm();
    let [a, b] = g.fillet_radii_mm().map(|f| f.unwrap_or(0.));
    if [g.radius_mm() - a, g.radius_mm() - b, g.height_mm() - a - b]
        .into_iter()
        .chain(g.fillet_radii_mm().into_iter().flatten())
        .any(|length| length < 10. * tolerance)
    {
        return Err(StepError::Unsupported);
    }
    let mut rims = BTreeMap::new();
    for rim in g.rims() {
        rims.insert(
            rim.ordinal,
            CircularRim::new(
                file,
                p.point_f64([0.; 2], rim.z_mm),
                p.normal,
                p.u,
                rim.radius_mm,
            )?,
        );
    }
    // Check representability inside the append transaction: any failed rim
    // construction or numerical proof rolls back all freshly encoded entities.
    if magnitude > 1e12 || residual + 32. * f64::EPSILON * magnitude > tolerance {
        return Err(StepError::InvalidPlacement);
    }
    let cap_start = p.point_f64([0.; 2], 0.);
    let cap_end = p.point_f64([0.; 2], g.height_mm());
    let reconstructed_height = dot(std::array::from_fn(|i| cap_end[i] - cap_start[i]), normal);
    RoundedCylinder::new(
        g.radius_mm(),
        reconstructed_height,
        g.fillet_radii_mm(),
        (10. * tolerance).max(1e-6),
        1e7,
    )
    .map_err(|_| StepError::Unsupported)?;
    let cap_key = |i: usize| {
        if g.fillet_radii_mm()[i].is_some() {
            2 + 2 * i as u32
        } else {
            i as u32
        }
    };
    let wall_key = |i: usize| {
        if g.fillet_radii_mm()[i].is_some() {
            3 + 2 * i as u32
        } else {
            i as u32
        }
    };
    let mut faces = Vec::new();
    for i in 0..2 {
        let rim = &rims[&cap_key(i)];
        let up = i == 1;
        let bound = circular_bound(file, rim.edge, up, true);
        faces.push(file.face(
            &[bound],
            rim.center,
            p.normal.map(|n| if up { n } else { -n }),
            p.u,
        ));
    }
    let trim = CylindricalTrim::from_rims(file, &rims[&wall_key(0)], &rims[&wall_key(1)])?;
    faces.push(trim.wall(file, true));
    for (i, f) in g.fillet_radii_mm().into_iter().enumerate() {
        if let Some(f) = f {
            let major_radius_mm = g.radius_mm() - f;
            let center_z_mm = if i == 0 { f } else { g.height_mm() - f };
            let [low, high] = if i == 0 {
                [&rims[&cap_key(i)], &rims[&wall_key(i)]]
            } else {
                [&rims[&wall_key(i)], &rims[&cap_key(i)]]
            };
            let center = file.point(p.point_f64([0.; 2], center_z_mm));
            let axis = file.axis(center, p.normal, p.u);
            let surface = file.entity(format_args!(
                "TOROIDAL_SURFACE('',{axis},{major_radius_mm:.16E},{f:.16E})"
            ));
            let circle_center = file.point(p.point_f64([major_radius_mm, 0.], center_z_mm));
            let circle_axis = file.axis(circle_center, p.v.map(|x| -x), p.u);
            let circle = file.entity(format_args!("CIRCLE('',{circle_axis},{f:.16E})"));
            let context=file.entity(format_args!("(GEOMETRIC_REPRESENTATION_CONTEXT(2)PARAMETRIC_REPRESENTATION_CONTEXT()REPRESENTATION_CONTEXT('2D SPACE',''))"));
            let pcurves = [0., TAU].map(|u| {
                let point = file.entity(format_args!("CARTESIAN_POINT('',({u:.16E},0.))"));
                let direction = file.entity(format_args!("DIRECTION('',(0.,1.))"));
                let vector = file.entity(format_args!("VECTOR('',{direction},1.)"));
                let line = file.entity(format_args!("LINE('',{point},{vector})"));
                let representation = file.entity(format_args!(
                    "DEFINITIONAL_REPRESENTATION('',({line}),{context})"
                ));
                file.entity(format_args!("PCURVE('',{surface},{representation})"))
            });
            let [first, last] = pcurves;
            let curve = file.entity(format_args!(
                "SEAM_CURVE('',{circle},({first},{last}),.PCURVE_S1.)"
            ));
            let [start, end] = [low.vertex.vertex, high.vertex.vertex];
            let seam = file.entity(format_args!("EDGE_CURVE('',{start},{end},{curve},.T.)"));
            faces.push(periodic_wall(
                file,
                [low.edge, high.edge],
                seam,
                surface,
                true,
            ));
        }
    }
    Ok(file.solid(&faces))
}

#[cfg(test)]
mod tests;
