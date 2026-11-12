//! Interval evaluation of unchanged analytic source curves.
use super::*;
fn interpolate(ends: [I; 2], t: I) -> I {
    if t == I::point(0.) {
        ends[0]
    } else if t == I::point(1.) {
        ends[1]
    } else {
        ends[0] + (ends[1] - ends[0]) * t
    }
}
fn polar(radius: I, z: I, q: I, body: &Chamfer) -> Result<[I; 3], Error> {
    let theta = (z - q) / body.rate_interval()?;
    let out = [radius * theta.cos(), radius * theta.sin(), z];
    for v in out {
        v.checked()?;
    }
    Ok(out)
}
fn phase(body: &Chamfer, start: u32, q: I) -> I {
    q + I::point(f64::from(start)) * I::point(body.tool.spec().pitch_mm)
}
fn join(body: &Chamfer, band: usize, q: I, active: bool) -> Result<(I, I), Error> {
    let r = body.profile(band, q)?.radius;
    let z = if active {
        (r - I::point(body.entry_radius_mm)).positive() / body.cone_slope()?
    } else {
        I::point(0.)
    };
    Ok((r, z.checked()?))
}
fn endpoint_join(
    body: &Chamfer,
    band: usize,
    end: PhaseEndpoint,
    active: bool,
) -> Result<(I, I), Error> {
    if end.exact_contact {
        Ok((I::point(body.entry_radius_mm), I::point(0.)))
    } else {
        join(body, band, end.range(), active)
    }
}
pub(super) fn bounds(
    body: &Chamfer,
    source: SourceCurve,
    fraction: [f64; 2],
) -> Result<[I; 3], Error> {
    if fraction.into_iter().any(|t| !t.is_finite())
        || fraction[0] < 0.
        || fraction[1] > 1.
        || fraction[0] > fraction[1]
    {
        return Err(Error::Invalid);
    }
    let t = I::new(fraction[0], fraction[1]);
    let f = I::point(body.material_length_mm());
    match source {
        SourceCurve::Intersection {
            start,
            band,
            phase: ends,
            active,
        } => {
            let q = interpolate(ends.map(PhaseEndpoint::range), t);
            let (r, z) = join(body, band, q, active)?;
            polar(r, z, phase(body, start, q), body)
        }
        SourceCurve::EntryArc { start, phase: ends } => {
            let q = interpolate(ends.map(PhaseEndpoint::range), t);
            polar(
                I::point(body.entry_radius_mm),
                I::point(0.),
                phase(body, start, q),
                body,
            )
        }
        SourceCurve::EndProfile {
            start,
            band,
            phase: ends,
        } => {
            let q = interpolate(ends.map(PhaseEndpoint::range), t);
            polar(
                body.profile(band, q)?.radius,
                f,
                phase(body, start, q),
                body,
            )
        }
        SourceCurve::WallRail {
            start,
            band,
            phase: end,
            active,
        } => {
            let q = end.range();
            let (r, z) = endpoint_join(body, band, end, active)?;
            polar(r, interpolate([z, f], t), phase(body, start, q), body)
        }
        SourceCurve::ConeRail {
            start,
            band,
            phase: end,
        } => {
            let q = end.range();
            let (_, z) = endpoint_join(body, band, end, true)?;
            let z = interpolate([I::point(0.), z], t);
            let r = I::point(body.entry_radius_mm) + body.cone_slope()? * z;
            polar(r, z, phase(body, start, q), body)
        }
        SourceCurve::Circle { radius_mm, z_mm } => {
            if fraction == [0.; 2] || fraction == [1.; 2] {
                return Ok([I::point(radius_mm), I::point(0.), I::point(z_mm)]);
            }
            let theta = I::pi() * I::point(2.) * t;
            Ok([
                I::point(radius_mm) * theta.cos(),
                I::point(radius_mm) * theta.sin(),
                I::point(z_mm),
            ])
        }
        SourceCurve::NeckSeam {
            radius_mm,
            stations_mm,
        } => Ok([
            I::point(radius_mm),
            I::point(0.),
            interpolate(stations_mm.map(I::point), t),
        ]),
    }
}
/// Full corresponding-parameter bound when adjacent source supports share a
/// rail. Endpoint nearness alone cannot prove a long helical rail: radial and
/// lower-station discrepancies are retained before periodic-angle reduction.
pub(super) fn rail_binding_error(
    body: &Chamfer,
    a: SourceCurve,
    b: SourceCurve,
    angular_turns: i32,
) -> Result<f64, Error> {
    if a == b && angular_turns == 0 {
        return Ok(0.);
    }
    let fields = |curve| match curve {
        SourceCurve::WallRail {
            start,
            band,
            phase,
            active,
        } => Ok((false, start, band, phase, active)),
        SourceCurve::ConeRail { start, band, phase } => Ok((true, start, band, phase, true)),
        _ => Err(Error::Invalid),
    };
    let (cone_a, start_a, band_a, phase_a, active_a) = fields(a)?;
    let (cone_b, start_b, band_b, phase_b, active_b) = fields(b)?;
    if cone_a != cone_b {
        return Err(Error::Invalid);
    }
    let (r0, g0) = endpoint_join(body, band_a, phase_a, active_a)?;
    let (r1, g1) = endpoint_join(body, band_b, phase_b, active_b)?;
    let same_profile = band_a == band_b && phase_a == phase_b;
    let same_contact = phase_a.exact_contact && phase_b.exact_contact;
    let dg = if same_contact || (same_profile && active_a == active_b) {
        I::point(0.)
    } else {
        I::point((g1 - g0).magnitude())
    };
    let dr = if cone_a {
        body.cone_slope()?.abs() * dg
    } else if same_profile || same_contact {
        I::point(0.)
    } else {
        I::point((r1 - r0).magnitude())
    };
    let dq = if start_a == start_b && phase_a == phase_b {
        I::point(0.)
    } else {
        phase(body, start_b, phase_b.range()) - phase(body, start_a, phase_a.range())
    };
    let angle = (dq / body.rate_interval()?
        + I::pi() * I::point(2.) * I::point(f64::from(angular_turns)))
    .abs();
    let error = dr
        + dg
        + I::point(body.tool.spec().major_radius_mm) * (dg / body.rate_interval()?.abs() + angle);
    Ok(error.checked()?.hi)
}
