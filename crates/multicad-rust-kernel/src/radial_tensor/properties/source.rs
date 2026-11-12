//! Bounded calculation cells over unchanged finite source parameterizations.
use super::*;
use crate::tapered_thread::{
    curve_first_bounds, curve_taylor9, tensor_first_bounds, tensor_taylor8,
};
#[derive(Clone)]
pub(super) enum Source {
    Tensor(Tensor),
    Spline(Vec<[I; 3]>),
    Circle(Circle),
}
#[derive(Clone)]
pub(super) struct Tensor {
    pub geometry: crate::tapered_thread::TensorCell,
    pub normal: std::sync::Arc<crate::tapered_thread::NormalPolynomial>,
}
pub(super) struct Jet<const N: usize> {
    pub p: [Series<N>; 3],
    pub tangent: [Series<N>; 3],
    pub normal: [Series<N>; 3],
}
#[derive(Clone)]
pub(super) struct Circle {
    pub center: [I; 3],
    pub u: [I; 3],
    pub v: [I; 3],
    pub radius: I,
    pub start: I,
    pub sweep: I,
    pub range: [f64; 2],
}
pub(super) type Jets = Jet<9>;
fn series<const N: usize>(coefficients: [[I; 3]; N]) -> [Series<N>; 3] {
    std::array::from_fn(|axis| Series {
        c: coefficients.map(|p| p[axis]),
    })
}
impl Source {
    pub fn dimensions(&self) -> usize {
        usize::from(matches!(self, Self::Tensor(_))) + 1
    }
    pub fn taylor(&self, along_v: bool) -> Result<Jets, Error> {
        Ok(match self {
            Self::Tensor(cell) => {
                let [p, du, _] = tensor_taylor8(&cell.geometry, along_v).map(series);
                let normal = series(cell.normal.taylor8(cell.geometry.domain(), along_v)?);
                Jet {
                    p,
                    tangent: du,
                    normal,
                }
            }
            Self::Spline(p) => {
                let coefficients = curve_taylor9(p);
                let position = std::array::from_fn(|i| coefficients[i]);
                let tangent = std::array::from_fn(|i| {
                    coefficients[i + 1].map(|v| v * I::point((i + 1) as f64))
                });
                Jet {
                    p: series(position),
                    tangent: series(tangent),
                    normal: [Series::constant(0.); 3],
                }
            }
            Self::Circle(circle) => {
                let p = circle.jet(I::new(0., 1.));
                Jet {
                    p: std::array::from_fn(|k| Series {
                        c: std::array::from_fn(|i| p[k].c[i]),
                    }),
                    tangent: std::array::from_fn(|k| Series {
                        c: std::array::from_fn(|i| p[k].c[i + 1] * I::point((i + 1) as f64)),
                    }),
                    normal: [Series::constant(0.); 3],
                }
            }
        })
    }
    pub fn at(&self, uv: [I; 2]) -> Jet<1> {
        let [p, tangent, normal] = match self {
            Self::Tensor(cell) => {
                let [p, du, dv] = tensor_first_bounds(&cell.geometry, uv);
                [
                    p,
                    du,
                    [
                        du[1] * dv[2] - du[2] * dv[1],
                        du[2] * dv[0] - du[0] * dv[2],
                        du[0] * dv[1] - du[1] * dv[0],
                    ],
                ]
            }
            Self::Spline(p) => {
                let [position, tangent] = curve_first_bounds(p, uv[0]);
                [position, tangent, [I::point(0.); 3]]
            }
            Self::Circle(circle) => {
                let p = circle.jet(uv[0]);
                [p.map(|v| v.c[0]), p.map(|v| v.c[1]), [I::point(0.); 3]]
            }
        };
        Jet {
            p: p.map(Series::interval),
            tangent: tangent.map(Series::interval),
            normal: normal.map(Series::interval),
        }
    }
    pub fn subdivide(&self, along_v: bool) -> Result<[Self; 2], Error> {
        match self {
            Self::Tensor(cell) => {
                Ok(cell
                    .geometry
                    .subdivide(usize::from(along_v))?
                    .map(|geometry| {
                        Self::Tensor(Tensor {
                            geometry,
                            normal: cell.normal.clone(),
                        })
                    }))
            }
            Self::Spline(points) => {
                let (a, b) = trims::split(points);
                Ok([Self::Spline(a), Self::Spline(b)])
            }
            Self::Circle(circle) => Ok(circle.subdivide()?.map(Self::Circle)),
        }
    }
}

impl Circle {
    fn jet(&self, t: I) -> [Series<10>; 3] {
        let Self {
            center,
            u,
            v,
            radius,
            start,
            sweep,
            range,
        } = self;
        let mut phase = Series::interval(
            *start + *sweep * (I::point(range[0]) + (I::point(range[1]) - I::point(range[0])) * t),
        );
        phase.c[1] = *sweep * (I::point(range[1]) - I::point(range[0]));
        let [sin, cos] = phase.sin_cos();
        std::array::from_fn(|k| {
            Series::interval(center[k])
                + Series::interval(*radius)
                    * (Series::interval(u[k]) * cos + Series::interval(v[k]) * sin)
        })
    }
    fn subdivide(&self) -> Result<[Self; 2], Error> {
        let [lo, hi] = self.range;
        let mid = lo + (hi - lo) * 0.5;
        if mid <= lo || mid >= hi {
            return Err(Error::Unresolved);
        }
        let mut a = self.clone();
        let mut b = self.clone();
        a.range[1] = mid;
        b.range[0] = mid;
        Ok([a, b])
    }
}
