use super::*;
use std::f64::consts::TAU;

mod planar_circle;

struct LinePcurve {
    surface: u32,
    origin: Point,
    direction: Point,
    scale: f64,
    trim: Option<[f64; 2]>,
}

impl Shell<'_, '_> {
    pub(super) fn edge_geometry(&mut self, id: u32, vertices: [u32; 2]) -> Result<u32> {
        let curve = self.simple(id)?;
        let seam = curve.name == "SEAM_CURVE";
        if !seam && curve.name != "SURFACE_CURVE" {
            return Ok(id);
        }
        let [Value::String(_), curve_3d, associated, Value::Enum(master)] = curve.args.as_slice()
        else {
            return Err(ReadError::Invalid);
        };
        let curve_3d = reference(curve_3d)?;
        let associated = refs(associated)?;
        if !(1..=2).contains(&associated.len()) {
            return Err(ReadError::Invalid);
        }
        for &surface in &associated {
            if !matches!(
                self.simple(surface)?.name,
                "PCURVE" | "PLANE" | "CYLINDRICAL_SURFACE"
            ) {
                return Err(ReadError::Unsupported);
            }
        }
        let selected = match *master {
            // The associated data is supplementary for a 3D master curve.
            "CURVE_3D" => None,
            "PCURVE_S1" => Some(0),
            "PCURVE_S2" => Some(1),
            _ => return Err(ReadError::Unsupported),
        };
        if seam {
            let [a, b] = associated.as_slice() else {
                return Err(ReadError::Invalid);
            };
            let [a, b] = [self.line_pcurve(*a)?, self.line_pcurve(*b)?];
            let (_, radius) = self.cylinder(a.surface)?;
            if a.surface != b.surface
                || a.direction[0] != 0.
                || b.direction[0] != 0.
                || ((a.origin[0] - b.origin[0]).abs() - TAU).abs() * radius > self.tolerance
            {
                return Err(ReadError::Invalid);
            }
            for pcurve in [a, b] {
                self.verify_cylindrical_master(curve_3d, pcurve, vertices)?;
            }
        } else if let Some(selected) = selected {
            let &pcurve = associated.get(selected).ok_or(ReadError::Invalid)?;
            let (surface, curve) = self.pcurve_definition(pcurve)?;
            match self.simple(surface)?.name {
                "PLANE" if self.simple(curve_3d)?.name == "CIRCLE" => {
                    self.verify_planar_circle_master(curve_3d, surface, curve)?
                }
                "PLANE" => self.verify_planar_line_master(
                    curve_3d,
                    self.line_pcurve_geometry(surface, curve)?,
                    vertices,
                )?,
                "CYLINDRICAL_SURFACE" => self.verify_cylindrical_master(
                    curve_3d,
                    self.line_pcurve_geometry(surface, curve)?,
                    vertices,
                )?,
                _ => return Err(ReadError::Unsupported),
            }
        }
        Ok(curve_3d)
    }

    fn line_pcurve(&self, pcurve: u32) -> Result<LinePcurve> {
        let (surface, curve) = self.pcurve_definition(pcurve)?;
        self.line_pcurve_geometry(surface, curve)
    }

    fn pcurve_definition(&self, pcurve: u32) -> Result<(u32, u32)> {
        let [Value::String(_), surface, definition] = self.args(pcurve, "PCURVE")? else {
            return Err(ReadError::Invalid);
        };
        let surface = reference(surface)?;
        let [Value::String(_), items, context] =
            self.args(reference(definition)?, "DEFINITIONAL_REPRESENTATION")?
        else {
            return Err(ReadError::Invalid);
        };
        let context = self.components(reference(context)?)?;
        if !context.iter().any(|component| {
            component.name == "GEOMETRIC_REPRESENTATION_CONTEXT"
                && matches!(component.args.as_slice(), [Value::Number(2.)])
        }) {
            return Err(ReadError::Unsupported);
        }
        let [curve] = refs(items)?
            .try_into()
            .map_err(|_| ReadError::Unsupported)?;
        Ok((surface, curve))
    }

    fn line_pcurve_geometry(&self, surface: u32, line: u32) -> Result<LinePcurve> {
        if self.simple(line)?.name == "B_SPLINE_CURVE_WITH_KNOTS"
            && self.simple(surface)?.name == "PLANE"
        {
            return self.clamped_segment(surface, line);
        }
        let (origin, direction, scale) = self.line(line, true)?;
        Ok(LinePcurve {
            surface,
            origin,
            direction,
            scale,
            trim: None,
        })
    }

    fn clamped_segment(&self, surface: u32, curve: u32) -> Result<LinePcurve> {
        let [
            Value::String(_),
            Value::Number(1.),
            Value::List(points),
            Value::Enum("UNSPECIFIED"),
            Value::Enum("F"),
            Value::Enum("F"),
            Value::List(multiplicities),
            Value::List(knots),
            Value::Enum("PIECEWISE_BEZIER_KNOTS" | "UNSPECIFIED"),
        ] = self.args(curve, "B_SPLINE_CURVE_WITH_KNOTS")?
        else {
            return Err(ReadError::Unsupported);
        };
        let (
            [a, b],
            [Value::Number(2.), Value::Number(2.)],
            [Value::Number(start), Value::Number(end)],
        ) = (
            points.as_slice(),
            multiplicities.as_slice(),
            knots.as_slice(),
        )
        else {
            return Err(ReadError::Unsupported);
        };
        let span = end - start;
        if !span.is_finite() || span <= 0. {
            return Err(ReadError::Invalid);
        }
        let a = self.parameter_pair(reference(a)?, "CARTESIAN_POINT")?;
        let b = self.parameter_pair(reference(b)?, "CARTESIAN_POINT")?;
        // With degree one and two clamped knots, the complete spline is
        // exactly affine over its finite parameter interval.
        let delta = sub(b, a);
        let direction = unit(delta)?;
        let scale = length(delta) / span;
        if !scale.is_finite() || scale <= 0. {
            return Err(ReadError::Invalid);
        }
        let start_distance = (-start / span) * length(delta);
        let origin = std::array::from_fn(|i| direction[i].mul_add(start_distance, a[i]));
        Ok(LinePcurve {
            surface,
            origin,
            direction,
            scale,
            trim: Some([*start, *end]),
        })
    }

    fn verify_planar_line_master(
        &mut self,
        curve: u32,
        pcurve: LinePcurve,
        vertices: [u32; 2],
    ) -> Result<()> {
        let [Value::String(_), placement] = self.args(pcurve.surface, "PLANE")? else {
            return Err(ReadError::Invalid);
        };
        let plane = self.placement(reference(placement)?)?;
        let [x, y, _] = pcurve.origin;
        let [dx, dy, _] = pcurve.direction;
        let lifted_origin =
            std::array::from_fn(|i| plane.origin[i] + x * plane.u[i] + y * plane.v[i]);
        let lifted_direction = std::array::from_fn(|i| dx * plane.u[i] + dy * plane.v[i]);
        self.verify_linear_lift(
            curve,
            vertices,
            lifted_origin,
            lifted_direction,
            pcurve.scale,
            pcurve.trim,
        )
    }

    fn cylinder(&self, surface: u32) -> Result<(Plane, f64)> {
        let [Value::String(_), placement, Value::Number(radius)] =
            self.args(surface, "CYLINDRICAL_SURFACE")?
        else {
            return Err(ReadError::Invalid);
        };
        if *radius <= self.tolerance || *radius > 1e12 {
            return Err(ReadError::Invalid);
        }
        Ok((self.placement(reference(placement)?)?, *radius))
    }

    fn verify_cylindrical_master(
        &mut self,
        curve: u32,
        pcurve: LinePcurve,
        vertices: [u32; 2],
    ) -> Result<()> {
        let (plane, radius) = self.cylinder(pcurve.surface)?;
        let [angle, height, _] = pcurve.origin;
        let [dx, dy, _] = pcurve.direction;
        let (sin, cos) = angle.sin_cos();
        let radial = std::array::from_fn(|i| cos * plane.u[i] + sin * plane.v[i]);
        let center = std::array::from_fn(|i| plane.origin[i] + height * plane.normal[i]);
        if dx == 0. {
            let start = std::array::from_fn(|i| center[i] + radius * radial[i]);
            let direction = plane.normal.map(|coordinate| coordinate * dy);
            return self.verify_linear_lift(curve, vertices, start, direction, pcurve.scale, None);
        }
        if dy != 0. {
            return Err(ReadError::Unsupported);
        }
        let [Value::String(_), placement, Value::Number(actual_radius)] =
            self.args(curve, "CIRCLE")?
        else {
            return Err(ReadError::Invalid);
        };
        let actual = self.placement(reference(placement)?)?;
        let tangent = std::array::from_fn(|i| dx * (cos * plane.v[i] - sin * plane.u[i]));
        // Bounds the discrepancy for the entire revolution, including the
        // largest possible angular drift of the master curve's parameter.
        let drift = TAU * (pcurve.scale - 1.).abs();
        let bound = length(sub(center, actual.origin))
            + (radius - actual_radius).abs()
            + radius * (length(sub(radial, actual.u)) + length(sub(tangent, actual.v)) + drift);
        if bound > self.tolerance {
            return Err(ReadError::Invalid);
        }
        Ok(())
    }

    fn verify_linear_lift(
        &mut self,
        curve: u32,
        vertices: [u32; 2],
        lifted_origin: Point,
        lifted_direction: Point,
        parameter_scale: f64,
        trim: Option<[f64; 2]>,
    ) -> Result<()> {
        let (origin, direction, scale) = self.line(curve, false)?;
        if length(sub(lifted_origin, origin)) > self.tolerance
            || length(sub(lifted_direction, direction)) > 1e-10
            || (parameter_scale - scale).abs() > 1e-10 * parameter_scale.max(scale)
        {
            return Err(ReadError::Invalid);
        }
        // The permitted angular discrepancy must not accumulate into a
        // positional discrepancy over the actual finite edge interval.
        let offset = sub(lifted_origin, origin);
        let slope = sub(
            lifted_direction.map(|v| v * (parameter_scale / scale)),
            direction,
        );
        for vertex in vertices {
            let distance = dot(sub(self.vertex(vertex)?, origin), direction);
            if let Some([start, end]) = trim
                && (distance < start * scale - self.tolerance
                    || distance > end * scale + self.tolerance)
            {
                return Err(ReadError::Invalid);
            }
            if length(std::array::from_fn(|i| offset[i] + slope[i] * distance)) > self.tolerance {
                return Err(ReadError::Invalid);
            }
        }
        Ok(())
    }

    pub(super) fn line(&self, id: u32, parametric: bool) -> Result<(Point, Point, f64)> {
        let [Value::String(_), origin, vector] = self.args(id, "LINE")? else {
            return Err(ReadError::Invalid);
        };
        let [Value::String(_), direction, Value::Number(scale)] =
            self.args(reference(vector)?, "VECTOR")?
        else {
            return Err(ReadError::Invalid);
        };
        if !scale.is_finite() || *scale <= 0. {
            return Err(ReadError::Invalid);
        }
        let [origin, direction] = [reference(origin)?, reference(direction)?];
        let (origin, direction) = if parametric {
            let point = self.parameter_pair(origin, "CARTESIAN_POINT")?;
            let direction = self.parameter_pair(direction, "DIRECTION")?;
            (point, unit(direction)?)
        } else {
            (self.point(origin)?, self.direction(direction)?)
        };
        Ok((origin, direction, *scale))
    }

    fn parameter_pair(&self, id: u32, kind: &str) -> Result<Point> {
        let [Value::String(_), Value::List(coordinates)] = self.args(id, kind)? else {
            return Err(ReadError::Invalid);
        };
        let [Value::Number(x), Value::Number(y)] = coordinates.as_slice() else {
            return Err(ReadError::Invalid);
        };
        let point = [*x, *y, 0.];
        if point.iter().any(|x| !x.is_finite() || x.abs() > 1e12) {
            return Err(ReadError::Limit);
        }
        Ok(point)
    }
}
