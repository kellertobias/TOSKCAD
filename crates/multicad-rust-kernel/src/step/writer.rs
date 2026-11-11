use super::*;
use crate::{
    chamfered_bores::ChamferedBores,
    stadium::Stadium,
    step_read::{MAX_PLANAR_STEP_BYTES, MAX_STEP_ENTITIES},
};

/// Maximum number of closed solids in one flat STEP representation.
pub const MAX_COMPOUND_SOLIDS: usize = 1024;

/// Append independently closed analytic solids in world coordinates. Each
/// member keeps its boundary shells and topology in the final representation.
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
        if exceeds_readback_budget(&self.file) {
            return Err(StepError::Limit);
        }
        Ok(self.file.data)
    }

    /// A rejected append leaves all previously accepted members intact.
    fn append(
        &mut self,
        encode: impl FnOnce(&mut StepFile) -> Result<Entity, StepError>,
    ) -> Result<(), StepError> {
        if self.solids.len() == MAX_COMPOUND_SOLIDS || exceeds_readback_budget(&self.file) {
            return Err(StepError::Limit);
        }
        let previous = (self.file.data.len(), self.file.next);
        let encoded = encode(&mut self.file).and_then(|solid| {
            if exceeds_readback_budget(&self.file) {
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
                self.file.points.retain(|_, entity| entity.0 < previous.1);
                Err(error)
            }
        }
    }

    /// Append one exact rounded cylinder without changing generic import admission.
    pub fn append_rounded_cylinder(
        &mut self,
        geometry: &crate::rounded_cylinder::RoundedCylinder,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| super::rounded_cylinder::encode(file, geometry, placement, tolerance_mm))
    }

    /// Append a complete spherical face. The vertex loop denotes its
    /// collapsed south pole, while the surface chart supplies the seam.
    pub fn append_sphere(
        &mut self,
        sphere: crate::sphere::Sphere,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| {
            let radius = sphere.radius_mm();
            let center = file.point(placement.origin);
            let axis = file.axis(center, placement.normal, placement.u);
            let surface = file.entity(format_args!("SPHERICAL_SURFACE('',{axis},{radius:.16E})"));
            let pole = file.vertex(placement.point_f64([0.; 2], -radius)).vertex;
            let loop_id = file.entity(format_args!("VERTEX_LOOP('',{pole})"));
            let bound = file.entity(format_args!("FACE_BOUND('',{loop_id},.T.)"));
            let face = file.face_on(&[bound], surface, true);
            Ok(file.solid(&[face]))
        })
    }

    /// Append one complete regular torus with both periodic seams.
    pub fn append_torus(
        &mut self,
        torus: crate::torus::Torus,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| super::torus::encode(file, torus, &placement))
    }

    /// Append a complete five-face regular helical cylinder cut.
    pub fn append_helical_tube_cut(
        &mut self,
        cut: crate::helical_tube_cut::Cut,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| super::helical_tube_cut::encode(file, cut, &placement, tolerance_mm))
    }

    /// Append one bounded authored chevron solid with two continuous bands.
    pub fn append_herringbone(
        &mut self,
        gear: &crate::spur_gear::herringbone::Herringbone,
        origin: Point,
        u: Point,
        v: Point,
        tolerance: f64,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| herringbone::encode(file, gear, placement, tolerance))
    }

    /// Append one spherical bevel with exact cones and ruled spline flanks.
    pub fn append_bevel(
        &mut self,
        gear: &crate::bevel_gear::Bevel,
        origin: Point,
        u: Point,
        v: Point,
        tolerance: f64,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| bevel_gear::encode(file, gear, placement, tolerance))
    }

    pub fn append_helical_thread(
        &mut self,
        thread: &crate::helical_thread::Thread,
        stock_radius_mm: Option<f64>,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| {
            helical::encode(
                file,
                thread,
                helical::Closure::stock(stock_radius_mm),
                placement,
                tolerance_mm,
            )
        })
    }

    /// Append a ZA worm placed by its local frame (origin, u, v).
    pub fn append_za_worm(
        &mut self,
        worm: &crate::worm_gear::Worm,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
    ) -> Result<(), StepError> {
        let [origin, u, v] = super::worm_gear::thread_frame(worm, [origin, u, v]);
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| {
            helical::encode(
                file,
                worm.thread(),
                helical::Closure::hollow(worm.bore_radius_mm()),
                placement,
                tolerance_mm,
            )
        })
    }

    pub fn append_helical_screw(
        &mut self,
        screw: &crate::headed_screw::Screw,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        let closure = helical::Closure::screw(screw)?;
        self.append(|file| helical::encode(file, screw.thread(), closure, placement, tolerance_mm))
    }

    pub fn append_orthogonal(
        &mut self,
        boundary: &Boundary,
        transform: impl Fn(Point) -> Point,
    ) -> Result<(), StepError> {
        if !orthogonal_admission(boundary)? {
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
        if !orthogonal_admission(boundary)?
            || !supports_slotted_orthogonal(boundary, pockets, slots)
        {
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
        if !orthogonal_admission(boundary)?
            || !supports_drilled_orthogonal(boundary, pockets, through)
        {
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

    /// Append the exact cubic and analytic trims of a verified normal sweep.
    pub fn append_spline_sweep(
        &mut self,
        sweep: &crate::spline_sweep::Sweep,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| spline_sweep::encode(file, sweep, placement, tolerance_mm))
    }

    /// Append a spur gear with tolerance-bounded spline flanks.
    pub fn append_spur_gear(
        &mut self,
        profile: &crate::spur_gear::Profile,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| spur_gear::encode(file, profile, placement, tolerance_mm))
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

    pub fn append_conical_frustum(
        &mut self,
        revolution: &crate::revolution::Revolution,
        origin: Point,
        u: Point,
        v: Point,
        tolerance: f64,
    ) -> Result<(), StepError> {
        if !cylindrical_revolution::supports_conical_frustum(revolution, tolerance) {
            return Err(StepError::Unsupported);
        }
        self.append_conical_revolution(revolution, origin, u, v, tolerance)
    }

    pub fn append_conical_revolution(
        &mut self,
        revolution: &crate::revolution::Revolution,
        origin: Point,
        u: Point,
        v: Point,
        tolerance: f64,
    ) -> Result<(), StepError> {
        let ring = cylindrical_revolution::conical_meridian(revolution, tolerance)
            .ok_or(StepError::Unsupported)?;
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

    pub fn append_nonplanar_loft(
        &mut self,
        loft: &crate::planar_boolean::NonplanarLoft,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| nonplanar_loft::encode(file, &placement, loft))
    }

    pub fn append_planar_union(
        &mut self,
        union: &crate::planar_boolean::Union,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        if !supports_planar_union(union) {
            if matches!(
                crate::orthogonal_boundary::box_void::certify(&box_void::union_faces(union), 0.),
                Err(crate::orthogonal::BooleanError::Limit)
            ) {
                return Err(StepError::Limit);
            }
            return Err(StepError::Unsupported);
        }
        let placement = Placement::new(origin, u, v)?;
        let previous = (self.solids.len(), self.file.data.len(), self.file.next);
        if previous.0 + union.solid_count() > MAX_COMPOUND_SOLIDS {
            return Err(StepError::Limit);
        }
        if union.shell_count() != union.solid_count() {
            let faces = box_void::union_faces(union);
            return self.append(|file| {
                box_void::encode(file, &faces, &|[x, y, z]| placement.point_f64([x, y], z))
            });
        }
        for shell in union.shells() {
            if let Err(error) =
                self.append(|file| planar_union::encode(file, &placement, union, &shell.faces))
            {
                self.solids.truncate(previous.0);
                self.file.data.truncate(previous.1);
                self.file.next = previous.2;
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn append_warped_union(
        &mut self,
        shell: &crate::warped_union::Shell,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Result<(), StepError> {
        if !supports_warped_union(shell) {
            return Err(StepError::Unsupported);
        }
        let placement = Placement::new(origin, u, v)?;
        self.append(|file| warped_union::encode(file, placement, shell))
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
