use super::*;
use crate::{
    chamfered_bores::ChamferedBores,
    stadium::Stadium,
    step_read::{MAX_PLANAR_STEP_BYTES, MAX_STEP_ENTITIES},
};

/// Maximum number of closed solids in one flat STEP representation.
pub const MAX_COMPOUND_SOLIDS: usize = 1024;

/// Append independently closed analytic solids in world coordinates. Each
/// member keeps its own shell and topology in the final representation.
pub struct Writer {
    file: StepFile,
    solids: Vec<Entity>,
}

pub(super) fn write_one(
    encode: impl FnOnce(&mut Writer) -> Result<(), StepError>,
) -> Result<String, StepError> {
    let mut writer = Writer::new();
    encode(&mut writer)?;
    writer.finish()
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

fn exceeds_readback_budget(file: &StepFile) -> bool {
    file.next - 1 > MAX_STEP_ENTITIES || file.data.len() > MAX_PLANAR_STEP_BYTES
}

impl Writer {
    pub fn new() -> Self {
        Self {
            file: StepFile::new(),
            solids: Vec::new(),
        }
    }

    /// Finalize one world-space representation. An empty STEP is invalid.
    pub fn finish(mut self) -> Result<String, StepError> {
        let count = self.solids.len();
        if count == 0 {
            return Err(StepError::InvalidGeometry);
        }
        self.file.finish(self.solids);
        if count > 1 && exceeds_readback_budget(&self.file) {
            return Err(StepError::Limit);
        }
        Ok(self.file.data)
    }

    /// A rejected append leaves all previously accepted members intact.
    fn append(
        &mut self,
        encode: impl FnOnce(&mut StepFile) -> Result<Entity, StepError>,
    ) -> Result<(), StepError> {
        let compound = !self.solids.is_empty();
        if self.solids.len() == MAX_COMPOUND_SOLIDS
            || (compound && exceeds_readback_budget(&self.file))
        {
            return Err(StepError::Limit);
        }
        let previous = (self.file.data.len(), self.file.next);
        let encoded = encode(&mut self.file).and_then(|solid| {
            if compound && exceeds_readback_budget(&self.file) {
                Err(StepError::Limit)
            } else {
                Ok(solid)
            }
        });
        match encoded {
            Ok(solid) => {
                self.solids.push(solid);
                Ok(())
            }
            Err(error) => {
                self.file.data.truncate(previous.0);
                self.file.next = previous.1;
                Err(error)
            }
        }
    }

    pub fn append_orthogonal(
        &mut self,
        boundary: &Boundary,
        transform: impl Fn(Point) -> Point,
    ) -> Result<(), StepError> {
        if !boundary.supports_step() {
            return Err(StepError::Unsupported);
        }
        self.append(|file| encode_orthogonal(file, boundary, &[], &[], &[], &transform))
    }

    pub fn append_pocketed_orthogonal(
        &mut self,
        geometry: &OrthogonalSolid,
        boundary: &Boundary,
        pockets: &[OrthogonalPocket],
        transform: &dyn Fn(Point) -> Point,
    ) -> Result<(), StepError> {
        self.append_slotted_orthogonal(geometry, boundary, pockets, &[], transform)
    }

    pub fn append_slotted_orthogonal(
        &mut self,
        geometry: &OrthogonalSolid,
        boundary: &Boundary,
        pockets: &[OrthogonalPocket],
        slots: &[OrthogonalSlot],
        transform: &dyn Fn(Point) -> Point,
    ) -> Result<(), StepError> {
        if !supports_slotted_orthogonal(boundary, pockets, slots) {
            return Err(StepError::Unsupported);
        }
        orthogonal_slots::validate(geometry, boundary, slots, pockets, 1e-7, 1e-7)
            .map_err(|_| StepError::InvalidGeometry)?;
        self.append(|file| encode_orthogonal(file, boundary, pockets, slots, &[], transform))
    }

    pub fn append_drilled_orthogonal(
        &mut self,
        geometry: &OrthogonalSolid,
        boundary: &Boundary,
        pockets: &[OrthogonalPocket],
        through: &[OrthogonalThroughBore],
        transform: &dyn Fn(Point) -> Point,
    ) -> Result<(), StepError> {
        if through.is_empty() {
            return self.append_pocketed_orthogonal(geometry, boundary, pockets, transform);
        }
        if !supports_drilled_orthogonal(boundary, pockets, through) {
            return Err(StepError::Unsupported);
        }
        crate::orthogonal_pockets::validate_with_through_bores(
            geometry, boundary, pockets, through, 1e-7, 1e-7,
        )
        .map_err(|_| StepError::InvalidGeometry)?;
        self.append(|file| encode_orthogonal(file, boundary, pockets, &[], through, transform))
    }

    pub fn append_polygon_prism(
        &mut self,
        prism: &PolygonPrism,
        origin: Point,
        u: Point,
        v: Point,
        reverse_sweep: bool,
    ) -> Result<(), StepError> {
        self.append_polygon(
            prism,
            &[],
            &[],
            Placement::new(origin, u, v)?,
            reverse_sweep,
        )
    }

    pub fn append_pocketed_polygon_prism(
        &mut self,
        prism: &PolygonPrism,
        pockets: &[CircularPocket],
        origin: Point,
        u: Point,
        v: Point,
        reverse_sweep: bool,
    ) -> Result<(), StepError> {
        validate(prism, pockets, 1e-7, 1e-7).map_err(|_| StepError::InvalidGeometry)?;
        self.append_polygon(
            prism,
            pockets,
            &[],
            Placement::new(origin, u, v)?,
            reverse_sweep,
        )
    }

    pub fn append_chamfered_polygon_prism(
        &mut self,
        prism: &PolygonPrism,
        chamfers: &[BoreChamfer],
        origin: Point,
        u: Point,
        v: Point,
        reverse_sweep: bool,
    ) -> Result<(), StepError> {
        let bores = ChamferedBores::new(prism, chamfers, 1e-7, 1e-7)
            .map_err(|_| StepError::InvalidGeometry)?;
        self.append_polygon(
            prism,
            &[],
            bores.stacks(),
            Placement::new(origin, u, v)?,
            reverse_sweep,
        )
    }

    fn append_polygon(
        &mut self,
        prism: &PolygonPrism,
        pockets: &[CircularPocket],
        bore_stacks: &[BoreStack],
        placement: Placement,
        reverse_sweep: bool,
    ) -> Result<(), StepError> {
        let height = prism.height().as_f64();
        self.append(|file| {
            encode_prism(
                file,
                &mut prism.rings().iter().map(Vec::as_slice),
                prism.circular_holes(),
                pockets,
                bore_stacks,
                placement,
                if reverse_sweep { -height } else { height },
            )
        })
    }

    pub fn append_linear_sweep(
        &mut self,
        sweep: &Sweep,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        let height = sweep.height_mm();
        self.append(|file| {
            encode_planar_sweep(
                file,
                &placement,
                sweep.base(),
                [([0., 0., -1.], [1., 0., 0.]), ([0., 0., 1.], [1., 0., 0.])],
                |end, ring, vertex| {
                    if end {
                        let [x, y] = sweep.top()[ring][vertex];
                        [x, y, height]
                    } else {
                        let [x, y] = sweep.base()[ring][vertex];
                        [x, y, 0.]
                    }
                },
                |ring, vertex| {
                    let [x, y] = sweep.base()[ring][vertex];
                    let [top_x, top_y] = sweep.top()[ring][vertex];
                    direction([0.; 3], [top_x - x, top_y - y, height])
                },
            )
        })
    }

    /// Append a straight normal sweep bounded by exact lines and circles.
    pub fn append_normal_sweep(
        &mut self,
        sweep: &crate::normal_sweep::Sweep,
        origin: Point,
        u: Point,
        v: Point,
        reverse_sweep: bool,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| normal_sweep::encode(file, sweep, &[], placement, reverse_sweep))
    }

    /// Append an exact normal sweep with certified paired through-openings.
    pub fn append_pierced_normal_sweep(
        &mut self,
        pierced: &crate::normal_sweep::PiercedSweep,
        origin: Point,
        u: Point,
        v: Point,
        reverse_sweep: bool,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| {
            normal_sweep::encode(
                file,
                pierced.sweep(),
                pierced.openings(),
                placement,
                reverse_sweep,
            )
        })
    }

    pub fn append_trimmed_sweep(
        &mut self,
        sweep: &TrimmedSweep,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        let supports = [false, true].map(|end| {
            let normal = sweep.cap_normal(end);
            // Pick a basis axis well away from parallel to the face normal.
            let mut indices = [0, 1, 2];
            indices.sort_unstable_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()));
            let mut axis = [0.; 3];
            axis[indices[0]] = 1.;
            let reference = cross(cross(normal, axis), normal);
            (normal, reference)
        });
        self.append(|file| {
            encode_planar_sweep(
                file,
                &placement,
                sweep.rings(),
                supports,
                |end, ring, vertex| sweep.point(end, sweep.rings()[ring][vertex]),
                |_, _| Ok(sweep.direction()),
            )
        })
    }

    pub fn append_axial_cylinder(
        &mut self,
        cylinder: &AxialCylinder,
        reference: Point,
    ) -> Result<(), StepError> {
        let axis = [0., 0., 1.];
        if reference[2] != 0. {
            return Err(StepError::InvalidPlacement);
        }
        let [x, y] = cylinder.circle().center().map(Rational::as_f64);
        let bottom = [x, y, cylinder.bottom().as_f64()];
        let placement = Placement::new(bottom, reference, cross(axis, reference))?;
        let top = [x, y, cylinder.top().as_f64()];
        self.append(|file| {
            encode_circular_between(
                file,
                placement,
                top,
                cylinder.circle().radius().as_f64(),
                None,
            )
        })
    }

    pub fn append_placed_cylinder(
        &mut self,
        cylinder: &AxialCylinder,
        bottom: Point,
        reference: Point,
        side: Point,
    ) -> Result<(), StepError> {
        self.append_placed_circular_sweep(
            cylinder.circle().radius().as_f64(),
            None,
            cylinder.height().as_f64(),
            bottom,
            reference,
            side,
        )
    }

    pub fn append_placed_annulus(
        &mut self,
        annulus: &Annulus,
        bottom: Point,
        reference: Point,
        side: Point,
    ) -> Result<(), StepError> {
        self.append_placed_circular_sweep(
            annulus.outer_radius_mm(),
            Some(annulus.inner_radius_mm()),
            annulus.height_mm(),
            bottom,
            reference,
            side,
        )
    }

    pub fn append_placed_circular_sweep(
        &mut self,
        outer_radius: f64,
        inner_radius: Option<f64>,
        height: f64,
        bottom: Point,
        reference: Point,
        side: Point,
    ) -> Result<(), StepError> {
        if ![outer_radius, height]
            .into_iter()
            .chain(inner_radius)
            .all(|v| v.is_finite() && v > 0.)
            || inner_radius.is_some_and(|inner| inner >= outer_radius)
        {
            return Err(StepError::InvalidGeometry);
        }
        let placement = Placement::new(bottom, reference, side)?;
        let top = std::array::from_fn(|i| bottom[i] + placement.normal[i] * height);
        self.append(|file| {
            encode_circular_between(file, placement, top, outer_radius, inner_radius)
        })
    }

    pub fn append_cylindrical_revolution(
        &mut self,
        revolution: &crate::revolution::Revolution,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        let ring = cylindrical_revolution::meridian(revolution).ok_or(StepError::Unsupported)?;
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| cylindrical_revolution::encode(file, placement, ring))
    }

    pub fn append_axial_profile_stack(
        &mut self,
        stack: &crate::axial_profile_stack::ProfileStack,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| axial_stack::encode(file, placement, stack))
    }

    pub fn append_box(
        &mut self,
        size: [Rational; 3],
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| encode_rectangle(file, size, &[], placement))
    }

    pub fn append_rectangular_prism(
        &mut self,
        prism: &Prism,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        if prism.rounded_end().is_some() {
            return Err(StepError::Unsupported);
        }
        let holes: Vec<_> = prism.inner_loops().map(|(_, circle)| circle).collect();
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| encode_rectangle(file, prism.dimensions(), &holes, placement))
    }

    pub fn append_stadium_prism(
        &mut self,
        profile: &Stadium,
        height: f64,
        origin: Point,
        u: Point,
        v: Point,
        reverse_sweep: bool,
    ) -> Result<(), StepError> {
        self.append(|file| {
            stadium::encode_stadium_prism(file, profile, height, origin, u, v, reverse_sweep)
        })
    }
}

#[cfg(test)]
mod tests;
