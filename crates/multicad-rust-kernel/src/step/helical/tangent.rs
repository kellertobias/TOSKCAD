//! Axial shank with a crest arc shared by the thread and the cylinder.
use super::*;

pub(super) struct ShankTrim {
    pub(super) crest: Entity,
    pub(super) complement: Entity,
    pub(super) wall: Entity,
    pub(super) seat_hole: Entity,
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
