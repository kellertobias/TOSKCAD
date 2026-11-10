//! Analytic references to finite points and straight edges in a sketch frame.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Coplanar,
    Project,
    Intersect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geometry {
    Point([f64; 2]),
    Line([[f64; 2]; 2]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// Parameter on the finite directed edge, from 0 at the start to 1 at the end.
    pub parameter: f64,
    pub crossing: Option<i8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Empty,
    Coincident,
    Hit {
        geometry: Geometry,
        contact: Option<Contact>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    OffPlane,
}

fn validate(points: &[[f64; 3]], tolerance: f64) -> Result<(), Error> {
    if !(1e-7..=1e-3).contains(&tolerance)
        || points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e8)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

pub fn vertex(point: [f64; 3], method: Method, tolerance: f64) -> Result<Outcome, Error> {
    validate(&[point], tolerance)?;
    let [u, v, depth] = point;
    if depth.abs() > tolerance {
        match method {
            Method::Coplanar => return Err(Error::OffPlane),
            Method::Intersect => return Ok(Outcome::Empty),
            Method::Project => {}
        }
    }
    Ok(Outcome::Hit {
        geometry: Geometry::Point([u, v]),
        contact: None,
    })
}

pub fn edge(ends: [[f64; 3]; 2], method: Method, tolerance: f64) -> Result<Outcome, Error> {
    validate(&ends, tolerance)?;
    let [a, b] = ends;
    let [[u0, v0, d0], [u1, v1, d1]] = ends;
    let [du, dv, depth_delta] = std::array::from_fn(|i| b[i] - a[i]);
    let length = du.hypot(dv).hypot(depth_delta);
    if length <= tolerance {
        return Err(Error::Invalid);
    }
    let on_plane = [d0.abs() <= tolerance, d1.abs() <= tolerance];
    match method {
        Method::Coplanar | Method::Project => {
            if method == Method::Coplanar && on_plane != [true; 2] {
                return Err(Error::OffPlane);
            }
            let geometry = if du.hypot(dv) <= tolerance {
                Geometry::Point([u0, v0])
            } else {
                Geometry::Line([[u0, v0], [u1, v1]])
            };
            Ok(Outcome::Hit {
                geometry,
                contact: None,
            })
        }
        Method::Intersect => {
            if on_plane == [true; 2] {
                return Ok(Outcome::Coincident);
            }
            let parameter = if (d0 < 0.) != (d1 < 0.) {
                -d0 / depth_delta
            } else if on_plane[0] {
                0.
            } else if on_plane[1] {
                1.
            } else {
                return Ok(Outcome::Empty);
            };
            let signed_direction = depth_delta / length;
            let crossing = (signed_direction.abs() > 1e-9).then_some(if signed_direction < 0. {
                -1
            } else {
                1
            });
            Ok(Outcome::Hit {
                geometry: Geometry::Point([du.mul_add(parameter, u0), dv.mul_add(parameter, v0)]),
                contact: Some(Contact {
                    parameter,
                    crossing,
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests;
