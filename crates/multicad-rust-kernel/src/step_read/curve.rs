use super::*;

impl Shell<'_, '_> {
    pub(super) fn edge_geometry(&mut self, id: u32, vertices: [u32; 2]) -> Result<u32> {
        let curve = self.simple(id)?;
        if curve.name != "SURFACE_CURVE" {
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
            "CURVE_3D" => return Ok(curve_3d),
            "PCURVE_S1" => 0,
            "PCURVE_S2" => 1,
            _ => return Err(ReadError::Unsupported),
        };
        let &pcurve = associated.get(selected).ok_or(ReadError::Invalid)?;
        self.verify_planar_line_master(curve_3d, pcurve, vertices)?;
        Ok(curve_3d)
    }

    fn verify_planar_line_master(
        &mut self,
        curve: u32,
        pcurve: u32,
        vertices: [u32; 2],
    ) -> Result<()> {
        let [Value::String(_), surface, definition] = self.args(pcurve, "PCURVE")? else {
            return Err(ReadError::Invalid);
        };
        let [Value::String(_), placement] = self.args(reference(surface)?, "PLANE")? else {
            return Err(ReadError::Invalid);
        };
        let plane = self.placement(reference(placement)?)?;
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
        let ([x, y, _], [dx, dy, _], parameter_scale) = self.line(line, true)?;
        let (origin, direction, scale) = self.line(curve, false)?;
        let lifted_origin =
            std::array::from_fn(|i| plane.origin[i] + x * plane.u[i] + y * plane.v[i]);
        let lifted_direction = std::array::from_fn(|i| dx * plane.u[i] + dy * plane.v[i]);
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
