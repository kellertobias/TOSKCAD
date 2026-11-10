use super::*;
use std::f64::consts::TAU;

struct LinePcurve {
    surface: u32,
    origin: Point,
    direction: Point,
    scale: f64,
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
            let pcurve = self.line_pcurve(pcurve)?;
            match self.simple(pcurve.surface)?.name {
                "PLANE" => self.verify_planar_line_master(curve_3d, pcurve, vertices)?,
                "CYLINDRICAL_SURFACE" => {
                    self.verify_cylindrical_master(curve_3d, pcurve, vertices)?
                }
                _ => return Err(ReadError::Unsupported),
            }
        }
        Ok(curve_3d)
    }

    fn line_pcurve(&self, pcurve: u32) -> Result<LinePcurve> {
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
        let [line] = refs(items)?
            .try_into()
            .map_err(|_| ReadError::Unsupported)?;
        let (origin, direction, scale) = self.line(line, true)?;
        Ok(LinePcurve {
            surface,
            origin,
            direction,
            scale,
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
            return self.verify_linear_lift(curve, vertices, start, direction, pcurve.scale);
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
