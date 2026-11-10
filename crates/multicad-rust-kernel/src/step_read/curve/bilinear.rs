use super::*;

impl Shell<'_, '_> {
    fn straight_spline_poles(&self, id: u32) -> Result<([Point; 2], [f64; 2])> {
        let ([a, b], range) = self.clamped_line_definition(id)?;
        Ok(([self.point(a)?, self.point(b)?], range))
    }

    pub(in crate::step_read) fn straight_spline_curve(
        &mut self,
        id: u32,
        vertices: [u32; 2],
        forward: bool,
    ) -> Result<Curve> {
        let (mut poles, _) = self.straight_spline_poles(id)?;
        if !forward {
            poles.reverse();
        }
        let ends = [self.vertex(vertices[0])?, self.vertex(vertices[1])?];
        if length(sub(ends[0], ends[1])) <= self.tolerance
            || poles
                .into_iter()
                .zip(ends)
                .any(|(pole, end)| length(sub(pole, end)) > self.tolerance)
        {
            return Err(ReadError::Invalid);
        }
        Ok(Curve::Line)
    }

    pub(super) fn verify_bilinear_line_master(
        &mut self,
        curve: u32,
        surface: u32,
        pcurve: u32,
        vertices: [u32; 2],
    ) -> Result<()> {
        let patch = nonplanar_loft::bilinear_surface(self, surface)?;
        let (origin, direction, scale) = self.line(pcurve, true)?;
        let moving = match (direction[0] == 0., direction[1] == 0.) {
            (false, true) => 0,
            (true, false) => 1,
            _ => return Err(ReadError::Unsupported),
        };
        let fixed = 1 - moving;
        let at_end = if origin[fixed] == patch.limits[fixed][0] {
            false
        } else if origin[fixed] == patch.limits[fixed][1] {
            true
        } else {
            return Err(ReadError::Unsupported);
        };
        let indices = [[[0, 1], [3, 2]], [[0, 3], [1, 2]]][moving][usize::from(at_end)];
        let [a, b] = indices.map(|i| patch.corners[i]);
        let [low, high] = patch.limits[moving];
        let span = high - low;
        let position = (origin[moving] - low) / span;
        let delta = sub(b, a);
        let lifted_origin = std::array::from_fn(|i| delta[i].mul_add(position, a[i]));
        let velocity = delta.map(|x| x * (direction[moving] * scale / span));
        if !span.is_finite()
            || lifted_origin
                .into_iter()
                .chain(velocity)
                .any(|x| !x.is_finite())
        {
            return Err(ReadError::Invalid);
        }
        if self.simple(curve)?.name == "B_SPLINE_CURVE_WITH_KNOTS" {
            let (poles, range) = self.straight_spline_poles(curve)?;
            for (pole, parameter) in poles.into_iter().zip(range) {
                let expected =
                    std::array::from_fn(|i| velocity[i].mul_add(parameter, lifted_origin[i]));
                let deviation = length(sub(pole, expected));
                if !deviation.is_finite() || deviation > self.tolerance {
                    return Err(ReadError::Invalid);
                }
            }
            return Ok(());
        }
        self.verify_linear_lift(
            curve,
            vertices,
            lifted_origin,
            unit(velocity)?,
            length(velocity),
            None,
        )
    }
}
