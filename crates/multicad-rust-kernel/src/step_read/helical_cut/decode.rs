use super::*;
use crate::step::helical_trim::{Spline, Surface, SurfaceAxis};

#[derive(Clone, Copy)]
pub(super) struct Edge {
    pub vertices: [u32; 2],
    pub geometry: u32,
}

pub(super) fn edge(shell: &Shell<'_, '_>, id: u32) -> Result<Edge> {
    let [Value::String(_), start, end, geometry, Value::Enum("T")] =
        shell.args(id, "EDGE_CURVE")?
    else {
        return Err(ReadError::Invalid);
    };
    Ok(Edge {
        vertices: [reference(start)?, reference(end)?],
        geometry: reference(geometry)?,
    })
}

pub(super) fn wrapper<const N: usize>(
    shell: &Shell<'_, '_>,
    edge: Edge,
    kind: &str,
    master: &str,
    surfaces: [u32; N],
) -> Result<(u32, [u32; N])> {
    let [Value::String(_), geometry, pcurves, Value::Enum(sense)] =
        shell.args(edge.geometry, kind)?
    else {
        return Err(ReadError::Invalid);
    };
    require(*sense == master)?;
    let ids: [u32; N] = refs(pcurves)?.try_into().map_err(|_| ReadError::Invalid)?;
    require(ids.iter().copied().collect::<BTreeSet<_>>().len() == N)?;
    let mut curves = [0; N];
    for ((curve, id), expected) in curves.iter_mut().zip(ids).zip(surfaces) {
        let (surface, item) = shell.pcurve_definition(id)?;
        require(surface == expected)?;
        *curve = item;
    }
    Ok((reference(geometry)?, curves))
}

pub(in crate::step_read) struct Curve<const N: usize> {
    pub axis: SurfaceAxis,
    pub points: Vec<[f64; N]>,
}

pub(in crate::step_read) fn curve<const N: usize>(
    shell: &Shell<'_, '_>,
    id: u32,
) -> Result<Curve<N>> {
    let [
        Value::String(_),
        degree,
        points,
        Value::Enum("UNSPECIFIED"),
        Value::Enum("F"),
        Value::Enum("F"),
        multiplicities,
        knots,
        Value::Enum("UNSPECIFIED" | "PIECEWISE_BEZIER_KNOTS"),
    ] = shell.args(id, "B_SPLINE_CURVE_WITH_KNOTS")?
    else {
        return Err(ReadError::Invalid);
    };
    let axis = spline_axis(degree, multiplicities, knots)?;
    let points = refs(points)?
        .into_iter()
        .map(|id| {
            let [Value::String(_), coordinates] = shell.args(id, "CARTESIAN_POINT")? else {
                return Err(ReadError::Invalid);
            };
            let Value::List(coordinates) = coordinates else {
                return Err(ReadError::Invalid);
            };
            let values: &[Value<'_>; N] = coordinates
                .as_slice()
                .try_into()
                .map_err(|_| ReadError::Invalid)?;
            let mut point = [0.; N];
            for (destination, value) in point.iter_mut().zip(values) {
                let Value::Number(number) = *value else {
                    return Err(ReadError::Invalid);
                };
                if !number.is_finite() || number.abs() > 1e12 {
                    return Err(ReadError::Limit);
                }
                *destination = number;
            }
            Ok(point)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Curve { axis, points })
}

fn spline_axis(
    degree: &Value<'_>,
    multiplicities: &Value<'_>,
    knots: &Value<'_>,
) -> Result<SurfaceAxis> {
    let (Value::List(multiplicities), Value::List(knots)) = (multiplicities, knots) else {
        return Err(ReadError::Invalid);
    };
    let integer = |value: &Value<'_>| {
        let Value::Number(value) = *value else {
            return Err(ReadError::Invalid);
        };
        if !(1. ..=12.).contains(&value) || value.fract() != 0. {
            return Err(ReadError::Invalid);
        }
        Ok(value as usize)
    };
    let degree = integer(degree)?;
    require((2..=1025).contains(&knots.len()) && knots.len() == multiplicities.len())?;
    let knots = knots
        .iter()
        .map(|value| match *value {
            Value::Number(number) => Ok(number),
            _ => Err(ReadError::Invalid),
        })
        .collect::<Result<Vec<_>>>()?;
    let multiplicities = multiplicities
        .iter()
        .map(integer)
        .collect::<Result<Vec<_>>>()?;
    require(knots.windows(2).all(|pair| pair[0] < pair[1]))?;
    Ok(SurfaceAxis {
        degree,
        knots,
        multiplicities,
    })
}

pub(super) fn surface(shell: &Shell<'_, '_>, id: u32, frame: Frame) -> Result<Surface> {
    let [
        Value::String(_),
        degree_u,
        degree_v,
        grid,
        Value::Enum("UNSPECIFIED"),
        Value::Enum("F"),
        Value::Enum("F"),
        Value::Enum("F"),
        mult_u,
        mult_v,
        knots_u,
        knots_v,
        Value::Enum("UNSPECIFIED" | "PIECEWISE_BEZIER_KNOTS"),
    ] = shell.args(id, "B_SPLINE_SURFACE_WITH_KNOTS")?
    else {
        return Err(ReadError::Invalid);
    };
    let axes = [
        spline_axis(degree_u, mult_u, knots_u)?,
        spline_axis(degree_v, mult_v, knots_v)?,
    ];
    let Value::List(rows) = grid else {
        return Err(ReadError::Invalid);
    };
    let count = |axis: &SurfaceAxis| {
        axis.multiplicities
            .iter()
            .sum::<usize>()
            .checked_sub(axis.degree + 1)
            .filter(|&count| count > 0)
            .ok_or(ReadError::Invalid)
    };
    let [a, b] = [count(&axes[0])?, count(&axes[1])?];
    if a > MAX_STEP_ENTITIES / b {
        return Err(ReadError::Limit);
    }
    require(rows.len() == a)?;
    let mut poles_mm = Vec::with_capacity(a * b);
    for row in rows {
        let ids = refs(row)?;
        require(ids.len() == b)?;
        for id in ids {
            poles_mm.push(local(frame, shell.point(id)?));
        }
    }
    Ok(Surface { axes, poles_mm })
}

impl Curve<3> {
    pub(super) fn into_spline(self, frame: Frame) -> Spline {
        Spline {
            degree: self.axis.degree,
            knots: self.axis.knots,
            multiplicities: self.axis.multiplicities,
            poles_mm: self
                .points
                .into_iter()
                .map(|point| local(frame, point))
                .collect(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Affine {
    pub domain: [f64; 2],
    pub endpoints: [[f64; 2]; 2],
}

impl Affine {
    pub(super) fn decode(shell: &Shell<'_, '_>, id: u32) -> Result<Self> {
        let Curve { axis, points } = curve(shell, id)?;
        require(axis.degree == 1 && axis.multiplicities == [2, 2])?;
        let domain = axis.knots.try_into().map_err(|_| ReadError::Invalid)?;
        let endpoints = points.try_into().map_err(|_| ReadError::Invalid)?;
        Ok(Self { domain, endpoints })
    }
}

pub(super) fn plane(shell: &Shell<'_, '_>, id: u32) -> Result<Frame> {
    let [Value::String(_), placement] = shell.args(id, "PLANE")? else {
        return Err(ReadError::Invalid);
    };
    shell.placement(reference(placement)?).map(Frame::from)
}

pub(super) fn circle(shell: &Shell<'_, '_>, id: u32) -> Result<(Frame, f64)> {
    let [Value::String(_), placement, Value::Number(radius)] = shell.args(id, "CIRCLE")? else {
        return Err(ReadError::Invalid);
    };
    Ok((
        shell.placement(reference(placement)?).map(Frame::from)?,
        *radius,
    ))
}
