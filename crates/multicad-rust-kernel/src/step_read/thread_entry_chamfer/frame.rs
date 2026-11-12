//! Directed complete-support bounds of the actual STEP axis semantics.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Frame {
    pub origin: [I; 3],
    pub axes: [[I; 3]; 3],
}
fn arithmetic(value: I) -> Result<I> {
    value.checked().map_err(|_| ReadError::Unsupported)
}
fn dot(a: [I; 3], b: [I; 3]) -> I {
    a.into_iter()
        .zip(b)
        .fold(I::point(0.), |sum, (a, b)| sum + a * b)
}
fn cross(a: [I; 3], b: [I; 3]) -> [I; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(super) fn unit(vector: [I; 3]) -> Result<[I; 3]> {
    let scale = vector.into_iter().map(I::magnitude).fold(0_f64, f64::max);
    if !scale.is_finite() || scale == 0. {
        return Err(ReadError::Invalid);
    }
    // Direct endpoint division avoids overflowing a reciprocal for a valid
    // direction stored at a very small, otherwise irrelevant common scale.
    let scaled = vector.map(|v| I::new((v.lo / scale).next_down(), (v.hi / scale).next_up()));
    let squared = arithmetic(dot(scaled, scaled))?;
    if squared.lo <= 0. {
        return Err(ReadError::Unsupported);
    }
    let norm = squared.sqrt();
    let result = scaled.map(|v| v / norm);
    for x in result {
        arithmetic(x)?;
    }
    Ok(result)
}
pub(super) fn direction(shell: &Shell<'_, '_>, id: u32) -> Result<[I; 3]> {
    let [Value::String(_), Value::List(values)] = shell.args(id, "DIRECTION")? else {
        return Err(ReadError::Invalid);
    };
    let [Value::Number(x), Value::Number(y), Value::Number(z)] = values.as_slice() else {
        return Err(ReadError::Invalid);
    };
    require([x, y, z].into_iter().all(|x| x.is_finite()))?;
    Ok([*x, *y, *z].map(I::point))
}
impl Frame {
    pub fn read(shell: &Shell<'_, '_>, id: u32) -> Result<Self> {
        let [Value::String(_), origin, normal, reference_axis] =
            shell.args(id, "AXIS2_PLACEMENT_3D")?
        else {
            return Err(ReadError::Invalid);
        };
        let n = unit(direction(shell, reference(normal)?)?)?;
        let v = unit(cross(
            n,
            unit(direction(shell, reference(reference_axis)?)?)?,
        ))?;
        let u = cross(v, n);
        Ok(Self {
            origin: shell.point(reference(origin)?)?.map(I::point),
            axes: [u, v, n],
        })
    }
    pub fn expected([origin, u, v]: [Point; 3]) -> Self {
        Self {
            origin: origin.map(I::point),
            axes: [
                u.map(I::point),
                v.map(I::point),
                super::super::cross(u, v).map(I::point),
            ],
        }
    }
    pub fn point(&self, point: [I; 3]) -> Result<[I; 3]> {
        let result = std::array::from_fn(|i| {
            self.origin[i]
                + self.axes[0][i] * point[0]
                + self.axes[1][i] * point[1]
                + self.axes[2][i] * point[2]
        });
        for value in result {
            arithmetic(value)?;
        }
        Ok(result)
    }
    /// Origin/basis differences are bounded before the complete physical
    /// extents are applied, so large remote supports cannot hide cancellation.
    pub fn support_error(&self, other: &Self, extents: Point) -> Result<f64> {
        let mut sum = I::point(0.);
        for i in 0..3 {
            sum = sum + (self.origin[i] - other.origin[i]).abs();
            for (axis, extent) in extents.into_iter().enumerate() {
                sum = sum + (self.axes[axis][i] - other.axes[axis][i]).abs() * I::point(extent);
            }
        }
        Ok(arithmetic(sum)?.hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_axis_records_and_uncertain_norms_fail_closed_without_losing_small_scaled_directions() {
        assert!(matches!(unit([I::point(0.); 3]), Err(ReadError::Invalid)));
        assert!(matches!(
            unit([I::new(-1., 1.); 3]),
            Err(ReadError::Unsupported)
        ));
        assert!(matches!(
            unit([I::point(f64::INFINITY); 3]),
            Err(ReadError::Invalid)
        ));
        for data in ["#1=DIRECTION('');", "#1=DIRECTION('',(1.,0.));"] {
            with_shell(data, |s| {
                assert!(matches!(direction(s, 1), Err(ReadError::Invalid)))
            });
        }
        with_shell("#1=AXIS2_PLACEMENT_3D('');", |s| {
            assert!(matches!(Frame::read(s, 1), Err(ReadError::Invalid)))
        });
        with_shell(
            "#1=CARTESIAN_POINT('',(0.,0.,0.));#2=DIRECTION('',(0.,0.,1.E-320));#3=DIRECTION('',(1.E-320,0.,0.));#4=AXIS2_PLACEMENT_3D('',#1,#2,#3);",
            |s| {
                let actual = Frame::read(s, 4).unwrap();
                assert!(
                    actual
                        .support_error(
                            &Frame::expected([[0.; 3], [1., 0., 0.], [0., 1., 0.]]),
                            [1.; 3]
                        )
                        .unwrap()
                        < 1e-12
                );
            },
        );
        with_shell(
            "#1=CARTESIAN_POINT('',(0.,0.,0.));#2=DIRECTION('',(0.,0.,1.));#3=AXIS2_PLACEMENT_3D('',#1,#2,#2);",
            |s| assert!(Frame::read(s, 3).is_err()),
        );
    }
}
