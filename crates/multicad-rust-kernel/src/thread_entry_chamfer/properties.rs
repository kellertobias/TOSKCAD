//! Certified local-chart integrals over the original analytic material.
use super::{interval::I, trim::Jet, *};
mod integrals;
mod quadrature;
use integrals::{Kind, Source};

/// Accuracy is relative to physical dimensional scales, never a fixed mm³
/// threshold. With major radius R and full height H, volume uses R²H and
/// surface area uses R(R+H); moments multiply these by their corresponding
/// R or H coordinate scales. All returned intervals include quadrature and
/// arithmetic error. Centroids must additionally meet the same requested
/// tolerance in their R/R/H coordinate scales; centroidal inertia uses the
/// corresponding volume-times-squared-length scale. Amplified derived errors
/// tighten the raw integral targets under the same cumulative budget.
/// This local-chart API performs no rigid placement.
#[derive(Clone, Copy, Debug)]
pub struct PropertiesOptions {
    pub relative_tolerance: f64,
    pub maximum_cells: usize,
}
impl Default for PropertiesOptions {
    fn default() -> Self {
        Self {
            relative_tolerance: 1e-7,
            maximum_cells: 1_000_000,
        }
    }
}
#[derive(Clone, Debug)]
pub struct FaceProperties {
    pub face: Face,
    pub area_mm2: Enclosure,
    pub first_moments_mm3: [Enclosure; 3],
    pub centroid_mm: [Enclosure; 3],
}
/// Unit-density material properties in the retained source's local chart.
/// `second_moments_mm5[i][j]` is ∫x_i*x_j dV; inertia is about the centroid.
/// No placement, approximate source trim or display mesh contributes to these
/// authoritative integrals.
#[derive(Clone, Debug)]
pub struct MassProperties {
    pub volume_mm3: Enclosure,
    pub first_moments_mm4: [Enclosure; 3],
    pub second_moments_mm5: [[Enclosure; 3]; 3],
    pub centroid_mm: [Enclosure; 3],
    pub centroidal_inertia_mm5: [[Enclosure; 3]; 3],
    pub faces: Vec<FaceProperties>,
}
const COUNT: usize = 14;
type Values = [I; COUNT];
fn enclosure(x: I) -> Enclosure {
    Enclosure {
        lower: x.lo,
        upper: x.hi,
    }
}
fn zero() -> Values {
    [I::point(0.); COUNT]
}
fn add(a: &mut Values, b: Values) {
    for (x, y) in a.iter_mut().zip(b) {
        *x = *x + y;
    }
}
fn positive(x: I) -> Result<I, Error> {
    let x = x.checked()?;
    if x.lo <= 0. {
        Err(Error::Unresolved)
    } else {
        Ok(x)
    }
}
fn tightened_tolerance(current: f64, ratio: f64) -> Result<f64, Error> {
    let next = current * (0.25 / ratio);
    if !next.is_finite() || next <= 0. || next >= current {
        Err(Error::Unresolved)
    } else {
        Ok(next)
    }
}
fn centroid(moment: [I; 3], measure: I) -> Result<[I; 3], Error> {
    let result = moment.map(|m| m / measure);
    for v in result {
        v.checked()?;
    }
    Ok(result)
}
impl MassProperties {
    fn quality(&self, body: &Chamfer, tolerance: f64) -> Result<f64, Error> {
        let r = I::point(body.tool.spec().major_radius_mm);
        let h = I::point(body.full_length_mm());
        let length = [r, r, h];
        let v = r * r * h;
        let area = r * (r + h);
        let mut ratio = 0_f64;
        let mut check = |value: Enclosure, scale: I| -> Result<(), Error> {
            let value = I::new(value.lower, value.upper).checked()?;
            let threshold = (I::point(tolerance) * positive(scale)?).lo;
            ratio = ratio.max((I::point(value.width()) / I::point(threshold)).hi);
            Ok(())
        };
        check(self.volume_mm3, v)?;
        for i in 0..3 {
            check(self.first_moments_mm4[i], v * length[i])?;
            check(self.centroid_mm[i], length[i])?;
            for j in 0..3 {
                check(self.second_moments_mm5[i][j], v * length[i] * length[j])?;
                let scale = if i == j {
                    v * (length[(i + 1) % 3].square() + length[(i + 2) % 3].square())
                } else {
                    v * length[i] * length[j]
                };
                check(self.centroidal_inertia_mm5[i][j], scale)?;
            }
        }
        for face in &self.faces {
            check(face.area_mm2, area)?;
            for (i, l) in length.into_iter().enumerate() {
                check(face.first_moments_mm3[i], area * l)?;
                check(face.centroid_mm[i], l)?;
            }
        }
        Ok(ratio)
    }
}
impl Chamfer {
    pub fn properties(&self, options: PropertiesOptions) -> Result<MassProperties, Error> {
        self.properties_with_interrupt(options, None)
    }
    pub fn properties_with_interrupt(
        &self,
        options: PropertiesOptions,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<MassProperties, Error> {
        if !options.relative_tolerance.is_finite()
            || options.relative_tolerance <= 0.
            || options.relative_tolerance >= 1.
        {
            return Err(Error::Invalid);
        }
        let mut work = Work::new(
            QueryOptions {
                tolerance: options.relative_tolerance,
                maximum_cells: options.maximum_cells,
            },
            interrupt,
        )?;
        let mut raw_tolerance = options.relative_tolerance;
        loop {
            work.step()?;
            let result = self.integrated_properties(raw_tolerance, &mut work)?;
            let ratio = result.quality(self, options.relative_tolerance)?;
            if ratio <= 1. {
                return Ok(result);
            }
            raw_tolerance = tightened_tolerance(raw_tolerance, ratio)?;
        }
    }
    fn integrated_properties(
        &self,
        raw_tolerance: f64,
        work: &mut Work<'_>,
    ) -> Result<MassProperties, Error> {
        let r = I::point(self.tool.spec().major_radius_mm);
        let h = I::point(self.full_length_mm());
        let v = r * r * h;
        let area = r * (r + h);
        let scales: Values = [
            v,
            v * r,
            v * r,
            v * h,
            v * r * r,
            v * r * r,
            v * h * h,
            v * r * r,
            v * r * h,
            v * r * h,
            area,
            area * r,
            area * r,
            area * h,
        ];
        for x in scales {
            positive(x)?;
        }
        let segments = self.phase_segments()?;
        let count = self.tool.spec().starts;
        let mut total = zero();
        let mut faces = vec![zero(); self.face_count() as usize];
        // Each source contributes a share of the requested total enclosure.
        let epsilon = raw_tolerance / (f64::from(count) * segments.len() as f64 * 4.);
        for start in 0..count {
            for segment in &segments {
                let wall = self
                    .wall_face_index(start, segment.band)
                    .ok_or(Error::Unresolved)?;
                let cone = self
                    .cone_face_index(start, segment.half)
                    .ok_or(Error::Unresolved)?;
                for (kind, face) in [
                    (Kind::Wall, wall),
                    (Kind::Cone, cone),
                    (Kind::Entry, 0),
                    (Kind::End, 1),
                ] {
                    if kind == Kind::Cone && !segment.active {
                        continue;
                    }
                    let source = Source {
                        body: self,
                        start,
                        segment: *segment,
                        kind,
                    };
                    let value = quadrature::integrate(source, scales, epsilon, work)?;
                    add(&mut total, value);
                    add(&mut faces[face as usize], value);
                }
            }
        }
        if let Some(tail) = self.terminal {
            let neck = integrals::neck(tail)?;
            add(&mut total, neck);
            add(&mut faces[(2 + 6 * count) as usize], neck);
            add(
                &mut faces[(3 + 6 * count) as usize],
                integrals::disk(tail.neck_radius_mm(), tail.full_length_mm())?,
            );
        }
        let volume = positive(total[0])?;
        let first = [total[1], total[2], total[3]];
        let second = [
            [total[4], total[7], total[8]],
            [total[7], total[5], total[9]],
            [total[8], total[9], total[6]],
        ];
        let centered: [[I; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| second[i][j] - first[i] * first[j] / volume)
        });
        let inertia = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                if i == j {
                    centered[(i + 1) % 3][(i + 1) % 3] + centered[(i + 2) % 3][(i + 2) % 3]
                } else {
                    -centered[i][j]
                }
            })
        });
        let mut output = Vec::with_capacity(faces.len());
        for (index, values) in faces.into_iter().enumerate() {
            let area = positive(values[10])?;
            let moment = [values[11], values[12], values[13]];
            output.push(FaceProperties {
                face: self.face(index as u32).ok_or(Error::Unresolved)?,
                area_mm2: enclosure(area),
                first_moments_mm3: moment.map(enclosure),
                centroid_mm: centroid(moment, area)?.map(enclosure),
            });
        }
        // Validate the complete result before publishing any properties.
        for x in first
            .into_iter()
            .chain(second.into_iter().flatten())
            .chain(inertia.into_iter().flatten())
        {
            x.checked()?;
        }
        Ok(MassProperties {
            volume_mm3: enclosure(volume),
            first_moments_mm4: first.map(enclosure),
            second_moments_mm5: second.map(|r| r.map(enclosure)),
            centroid_mm: centroid(first, volume)?.map(enclosure),
            centroidal_inertia_mm5: inertia.map(|r| r.map(enclosure)),
            faces: output,
        })
    }
}
#[cfg(test)]
mod tests;
