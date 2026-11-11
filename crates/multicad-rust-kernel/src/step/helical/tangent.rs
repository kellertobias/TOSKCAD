//! Axial shank with a crest arc shared by the thread and the cylinder.
use super::*;

pub(super) struct ShankTrim {
    pub(super) crest: Entity,
    pub(super) complement: Entity,
    pub(super) wall: Entity,
    pub(super) seat_hole: Entity,
}

pub(super) struct ExternalTrim {
    pub(super) crests: Vec<Entity>,
    pub(super) complements: Vec<Entity>,
    pub(super) wall: Entity,
    pub(super) far_cap: Entity,
}

pub(super) fn external(
    file: &mut StepFile,
    placement: &Placement,
    thread: &Thread,
    stock_length_mm: f64,
    corners: Vec<[Vertex; 2]>,
) -> Result<ExternalTrim, StepError> {
    let spec = thread.spec();
    let [_, start, _, _, crest, _] = thread.profile_breaks();
    let rate = thread.helical_rate();
    let z = spec.length_mm;
    let center = placement.point_f64([0.; 2], z);
    let arc_axis = placement
        .normal
        .map(|n| if spec.left_hand { n } else { -n });
    let radial = |phase: f64| {
        let (sin, cos) = ((z - phase) / rate).sin_cos();
        placement.vector([cos, sin, 0.])
    };
    let mut crests = Vec::with_capacity(corners.len());
    let mut complements = Vec::with_capacity(corners.len());
    for (k, &[first, last]) in corners.iter().enumerate() {
        let shift = k as f64 * spec.pitch_mm;
        let next = corners[(k + 1) % corners.len()][0];
        crests.push(
            circular_arc_edge(
                file,
                [last, next],
                center,
                arc_axis,
                radial(crest + shift),
                spec.major_radius_mm,
                (start + spec.pitch_mm - crest) / rate.abs(),
            )?
            .0,
        );
        complements.push(
            circular_arc_edge(
                file,
                [first, last],
                center,
                arc_axis,
                radial(start + shift),
                spec.major_radius_mm,
                (crest - start) / rate.abs(),
            )?
            .0,
        );
    }
    let reference = radial(start);
    let far = CircularRim::new(
        file,
        placement.point_f64([0.; 2], stock_length_mm),
        placement.normal,
        reference,
        spec.major_radius_mm,
    )?;
    let seam = line_edge(file, corners[0][0], far.vertex)?;
    let mut lower: Vec<_> = complements
        .iter()
        .zip(&crests)
        .flat_map(|(&a, &b)| [(a, true), (b, true)])
        .collect();
    if !spec.left_hand {
        lower = lower.into_iter().rev().map(|(e, s)| (e, !s)).collect();
    }
    let mut wire: Vec<_> = lower
        .into_iter()
        .map(|(e, s)| file.oriented_edge(e, s))
        .collect();
    wire.extend([
        file.oriented_edge(seam, true),
        file.oriented_edge(far.edge, false),
        file.oriented_edge(seam, false),
    ]);
    let bound = file.bound(&wire, true);
    let center_id = file.point(center);
    let axis = file.axis(center_id, placement.normal, reference);
    let radius = spec.major_radius_mm;
    let support = file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{radius:.16E})"));
    let wall = file.face_on(&[bound], support, true);
    let bound = circular_bound(file, far.edge, true, true);
    let far_cap = file.face(&[bound], far.center, placement.normal, placement.u);
    Ok(ExternalTrim {
        crests,
        complements,
        wall,
        far_cap,
    })
}

pub(super) fn encode(
    file: &mut StepFile,
    placement: &Placement,
    thread: &Thread,
    length_mm: f64,
    [first, last]: [Vertex; 2],
) -> Result<ShankTrim, StepError> {
    let spec = thread.spec();
    let [_, start, _, _, crest, _] = thread.profile_breaks();
    let rate = thread.helical_rate();
    let radial = |phase: f64| {
        let (sin, cos) = (-phase / rate).sin_cos();
        placement.vector([cos, sin, 0.])
    };
    let first_dir = radial(start);
    let last_dir = radial(crest);
    let circle_axis = placement
        .normal
        .map(|n| if spec.left_hand { n } else { -n });
    let crest_sweep = (start + spec.pitch_mm - crest) / rate.abs();
    let complement_sweep = (crest - start) / rate.abs();
    let crest_arc = circular_arc_edge(
        file,
        [last, first],
        placement.origin,
        circle_axis,
        last_dir,
        spec.major_radius_mm,
        crest_sweep,
    );
    let (crest, _) = crest_arc?;
    let complement_arc = circular_arc_edge(
        file,
        [first, last],
        placement.origin,
        circle_axis,
        first_dir,
        spec.major_radius_mm,
        complement_sweep,
    );
    let (complement, _) = complement_arc?;
    let bottom_rim = CircularRim::new(
        file,
        placement.point_f64([0.; 2], -length_mm),
        placement.normal,
        first_dir,
        spec.major_radius_mm,
    );
    let bottom = bottom_rim?;
    let seam = line_edge(file, bottom.vertex, first)?;
    let radius = spec.major_radius_mm;
    let axis = bottom.axis;
    let support = file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{radius:.16E})"));
    // Clock the periodic seam at the start of the complementary arc. Both
    // top arcs then run clockwise around the positive cylinder axis.
    let top = if spec.left_hand {
        [(crest, false), (complement, false)]
    } else {
        [(complement, true), (crest, true)]
    };
    let wire = [
        file.oriented_edge(bottom.edge, true),
        file.oriented_edge(seam, true),
        file.oriented_edge(top[0].0, top[0].1),
        file.oriented_edge(top[1].0, top[1].1),
        file.oriented_edge(seam, false),
    ];
    let bound = file.bound(&wire, true);
    let wall = file.face_on(&[bound], support, true);
    let seat_hole = circular_bound(file, bottom.edge, false, false);
    Ok(ShankTrim {
        crest,
        complement,
        wall,
        seat_hole,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helical_thread::Spec;

    #[test]
    fn external_trim_refuses_corrupt_arc_endpoints_and_nonfinite_tail() {
        let thread = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 2.37,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 2,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let placement = Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
        for corruption in 0..3 {
            let mut file = StepFile::new();
            let [_, start, _, _, crest, _] = thread.profile_breaks();
            let mut corners: Vec<_> = (0..2)
                .map(|k| {
                    [start, crest].map(|p| {
                        let (sin, cos) = ((thread.spec().length_mm - p - k as f64)
                            / thread.helical_rate())
                        .sin_cos();
                        file.vertex([3. * cos, 3. * sin, thread.spec().length_mm])
                    })
                })
                .collect();
            match corruption {
                0 => corners[0][1] = file.vertex([0., 0., thread.spec().length_mm]),
                1 => corners[0][0] = file.vertex([0., 0., thread.spec().length_mm]),
                _ => {}
            }
            assert!(
                external(
                    &mut file,
                    &placement,
                    &thread,
                    if corruption == 2 { f64::NAN } else { 5. },
                    corners
                )
                .is_err()
            );
        }
    }
}
