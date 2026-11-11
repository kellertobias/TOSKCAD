use super::*;
fn norm(v: Point) -> f64 {
    v[0].hypot(v[1]).hypot(v[2])
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

impl RoundedCylinderBody {
    /// Euclidean projection onto the exact convex meridian, rotated about Z.
    /// No display mesh participates in either the distance or occupied test.
    pub(crate) fn point_projection(
        &self,
        frame: PlaneFrame,
        point: Point,
        tolerance: f64,
    ) -> Result<(Point, f64, bool), GeometryError> {
        if point.into_iter().any(|v| !v.is_finite()) {
            return Err(failure(
                GeometryErrorKind::Input,
                "Point query must be finite",
            ));
        }
        let unit = |v: Point| {
            let n = norm(v);
            v.map(|x| x / n)
        };
        let u = unit(frame.u);
        let n = unit(frame.normal());
        let v = [
            n[1] * u[2] - n[2] * u[1],
            n[2] * u[0] - n[0] * u[2],
            n[0] * u[1] - n[1] * u[0],
        ];
        let r = self.geometry.radius_mm();
        let h = self.geometry.height_mm();
        let magnitude = point
            .into_iter()
            .chain(frame.origin)
            .map(f64::abs)
            .fold(r.max(h).max(1.), f64::max);
        let uncertainty = norm(sub(frame.u, u)) * r
            + norm(sub(frame.v, v)) * r
            + norm(sub(frame.normal(), n)) * h
            + 64. * f64::EPSILON * magnitude;
        if !uncertainty.is_finite() || uncertainty > tolerance / 8. {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rounded-cylinder point query cannot certify its world chart",
            ));
        }
        let delta = sub(point, frame.origin);
        let local = [dot(delta, u), dot(delta, v), dot(delta, n)];
        let rho = local[0].hypot(local[1]);
        let z = local[2];
        let [a, b] = self.geometry.fillet_radii_mm().map(|f| f.unwrap_or(0.));
        let mut best = [0.; 2];
        let mut distance = f64::INFINITY;
        let mut visit = |candidate: [f64; 2]| {
            let d = (rho - candidate[0]).hypot(z - candidate[1]);
            if d < distance {
                distance = d;
                best = candidate;
            }
        };
        visit([rho.min(r - a), 0.]);
        visit([rho.min(r - b), h]);
        visit([r, z.clamp(a, h - b)]);
        for role in [Role::StartFillet, Role::EndFillet] {
            if let Some(Surface::Toroid {
                major_radius_mm: major,
                minor_radius_mm: f,
                center_z_mm: center,
                meridian_rad: span,
            }) = self.geometry.surface(role)
            {
                let phase = (z - center).atan2(rho - major);
                for phase in [
                    Some(span[0]),
                    Some(span[1]),
                    (phase >= span[0] && phase <= span[1]).then_some(phase),
                ]
                .into_iter()
                .flatten()
                {
                    visit([major + f * phase.cos(), center + f * phase.sin()]);
                }
            }
        }
        let inside = self.geometry.contains(local, 0.);
        let clearance = if inside { distance } else { 0. };
        let projected = if inside {
            point
        } else {
            let radial = if rho == 0. {
                [0., 0.]
            } else {
                [(local[0] / rho) * best[0], (local[1] / rho) * best[0]]
            };
            std::array::from_fn(|i| {
                frame.origin[i] + u[i] * radial[0] + v[i] * radial[1] + n[i] * best[1]
            })
        };
        let measured = norm(sub(point, projected));
        if !measured.is_finite()
            || (measured - if inside { 0. } else { distance }).abs() > tolerance / 4.
        {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rounded-cylinder point witness exceeds its numerical budget",
            ));
        }
        Ok((projected, measured, inside && clearance > uncertainty))
    }
}
