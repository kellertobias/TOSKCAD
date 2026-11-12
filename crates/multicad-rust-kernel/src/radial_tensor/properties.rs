//! Source-integral surface and mass enclosures in a centered physical chart.
use super::*;
use crate::tapered_thread::{Scalar, Series};
mod measure;
mod quadrature;
mod source;
#[cfg(test)]
mod tests;
#[derive(Clone, Debug, PartialEq)]
pub struct FaceProperties {
    pub face_index: usize,
    pub area_mm2: [f64; 2],
    pub centroid_mm: [[f64; 2]; 3],
    /// The actual retained support, without inferred normal or placement.
    pub planar_support: Option<Axis>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MassProperties {
    pub volume_mm3: [f64; 2],
    pub surface_area_mm2: [f64; 2],
    pub centroid_mm: [[f64; 2]; 3],
    /// Density one; order xx, yy, zz, xy, xz, yz in the world axes.
    pub inertia_about_centroid_mm5: [[f64; 2]; 6],
    pub characteristic_length_mm: f64,
    /// Input face order, preserving every finite source face index.
    pub faces: Vec<FaceProperties>,
}
mod bounds;
mod faces;
mod integrate;
use measure::QUANTITIES;
impl RadialBoundary {
    /// Full-width error bounds normalized by L^3 (volume), L^2 (area),
    /// L (centroids), and L^5 (centroidal inertia), where L is returned in
    /// the receipt. Sewing/closure floors and world reconstruction are
    /// included; an uncertified request returns an error, not an estimate.
    pub fn properties_with_interrupt(
        &self,
        relative_accuracy: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<MassProperties, Error> {
        self.properties_with_budget(relative_accuracy, interrupt, 512_000_000)
    }
    fn properties_with_budget(
        &self,
        accuracy: f64,
        interrupt: Option<&dyn Fn() -> bool>,
        limit: usize,
    ) -> Result<MassProperties, Error> {
        if !accuracy.is_finite() || !(0. ..1.).contains(&accuracy) || accuracy == 0. {
            return Err(Error::Invalid);
        }
        let mut work = Work {
            remaining: limit,
            interrupt,
        };
        work.step()?;
        let sources = faces::build(self, &mut work)?;
        let bounds = bounds::prove(self, &sources, &mut work)?;
        let length = I::point(bounds.length);
        let volume_target = I::point(accuracy) * measure::power_i(length, 3);
        let area_target = I::point(accuracy) * length.square();
        (I::point(2.) * bounds.volume_error).bounded_by(volume_target.lo)?;
        let total_area_error = bounds.area_errors.iter().fold(I::point(0.), |s, &v| s + v);
        (I::point(2.) * total_area_error).bounded_by(area_target.lo)?;
        let center = self.ordered.caps.axial.chart.nominal_height * 0.5;
        let epsilon = accuracy / (256. * sources.len() as f64);
        let scale = bounds.volume_scale;
        let scales = std::array::from_fn(|i| match i {
            0 => scale,
            1..=3 => scale * length,
            4..=9 => scale * length.square(),
            10 => length.square(),
            _ => measure::power_i(length, 3),
        });
        for s in scales {
            s.positive()?;
        }
        let mut raw = [I::point(0.); QUANTITIES];
        let mut face_properties = Vec::with_capacity(sources.len());
        for (source, &error) in sources.into_iter().zip(&bounds.area_errors) {
            let value = integrate::face(
                source.sources,
                source.density,
                &self.ordered.caps.axial.chart,
                center,
                integrate::Accuracy { scales, epsilon },
                &mut work,
            )?;
            for (sum, v) in raw.iter_mut().zip(value) {
                *sum = *sum + v;
            }
            let area = (value[10] + symmetric(error)).positive()?;
            let local = std::array::from_fn(|i| (value[11 + i] + symmetric(error * length)) / area);
            let centroid = world(self, local, center)?;
            require_width(area, area_target.lo)?;
            for v in centroid {
                require_width(v, (I::point(accuracy) * length).lo)?;
            }
            face_properties.push(FaceProperties {
                face_index: source.index,
                area_mm2: pair(area),
                centroid_mm: centroid.map(pair),
                planar_support: source.plane,
            });
        }
        let volume = (raw[0] + symmetric(bounds.volume_error)).positive()?;
        let first: [I; 3] =
            std::array::from_fn(|i| raw[1 + i] + symmetric(bounds.volume_error * length));
        let second: [I; 6] =
            std::array::from_fn(|i| raw[4 + i] + symmetric(bounds.volume_error * length.square()));
        let local = first.map(|v| v / volume);
        let centroid = world(self, local, center)?;
        let mut covariance = [[I::point(0.); 3]; 3];
        for (i, (a, b)) in [(0, 0), (1, 1), (2, 2), (0, 1), (0, 2), (1, 2)]
            .into_iter()
            .enumerate()
        {
            let c = second[i] - first[a] * first[b] / volume;
            covariance[a][b] = c;
            covariance[b][a] = c;
        }
        let trace = covariance[0][0] + covariance[1][1] + covariance[2][2];
        let local_inertia = std::array::from_fn::<_, 3, _>(|i| {
            std::array::from_fn::<_, 3, _>(|j| {
                if i == j {
                    trace - covariance[i][j]
                } else {
                    -covariance[i][j]
                }
            })
        });
        let axes = self.ordered.caps.axial.chart.axes;
        let inertia = [(0, 0), (1, 1), (2, 2), (0, 1), (0, 2), (1, 2)].map(|(a, b)| {
            let mut value = I::point(0.);
            for i in 0..3 {
                for j in 0..3 {
                    value = value + axes[i][a] * local_inertia[i][j] * axes[j][b];
                }
            }
            if a == b {
                value.lo = value.lo.max(0.);
            }
            value
        });
        require_width(volume, volume_target.lo)?;
        let area = face_properties.iter().fold(I::point(0.), |s, f| {
            s + I::new(f.area_mm2[0], f.area_mm2[1])
        });
        require_width(area, area_target.lo)?;
        for v in centroid {
            require_width(v, (I::point(accuracy) * length).lo)?;
        }
        for v in inertia {
            require_width(v, (I::point(accuracy) * measure::power_i(length, 5)).lo)?;
        }
        Ok(MassProperties {
            volume_mm3: pair(volume),
            surface_area_mm2: pair(area),
            centroid_mm: centroid.map(pair),
            inertia_about_centroid_mm5: inertia.map(pair),
            characteristic_length_mm: bounds.length,
            faces: face_properties,
        })
    }
}
fn symmetric(v: I) -> I {
    I::new(-v.hi, v.hi)
}
fn pair(v: I) -> [f64; 2] {
    [v.lo, v.hi]
}
fn require_width(v: I, budget: f64) -> Result<(), Error> {
    v.checked()?;
    (I::point(v.hi) - I::point(v.lo)).bounded_by(budget)
}
fn world(body: &RadialBoundary, local: [I; 3], center: f64) -> Result<[I; 3], Error> {
    let chart = &body.ordered.caps.axial.chart;
    let offset =
        std::array::from_fn(|i| I::point(chart.bottom_point[i]) - I::point(chart.axis_origin[i]));
    let station = dot(offset, chart.axes[2]);
    let p = std::array::from_fn::<_, 3, _>(|i| {
        I::point(chart.axis_origin[i])
            + chart.axes[0][i] * local[0]
            + chart.axes[1][i] * local[1]
            + chart.axes[2][i] * (station + I::point(center) + local[2])
    });
    for v in p {
        v.checked()?;
    }
    Ok(p)
}
