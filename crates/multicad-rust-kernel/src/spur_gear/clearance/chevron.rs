//! Closed axial interval separation for a fixed continuous-chevron pose.
use super::*;
use crate::spur_gear::herringbone::Herringbone;

const MAX_INTERVALS: usize = 4096;
const MAX_TWIST_UNCERTAINTY_MM: f64 = 0.04;

/// Certify an entire fixed herringbone pose over its aligned face width.
/// Callers supply an orthonormal common transverse chart and a bound on
/// any projection/placement roundoff. The two apex planes must coincide.
pub fn certify_herringbone(
    gears: [&Herringbone; 2],
    placements: [Placement; 2],
    tolerance_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<ResultKind, Error> {
    let profiles = gears.map(Herringbone::profile);
    // Added hubs stay strictly inside the root disk, which belongs to
    // every transverse outer profile. A Clear result excludes intersecting
    // outer Jordan regions and nesting, so these coaxial disks remain
    // disjoint even beyond the aligned tooth bands. Interior witnesses
    // continue to refer to real tooth material with the bore excluded.
    let height = profiles[0].height_mm();
    if height != profiles[1].height_mm()
        || placements.into_iter().any(|p| !p.valid(1e-3))
        || !(1e-7..=1e-3).contains(&tolerance_mm)
    {
        return Err(Error::Invalid);
    }
    let rate = gears.map(Herringbone::twist_rad_per_mm);
    let radial_speed = std::array::from_fn::<_, 2, _>(|i| gears[i].tip_radius_mm() * rate[i].abs());

    // Two chevrons repeat their lower sections in exact reverse order.
    // A single-hand sweep requires the full width. The absolute twist
    // rate bounds displacement even across a chevron's middle plane.
    let width = if gears.iter().any(|gear| gear.is_helical()) {
        height
    } else {
        height / 2.
    };
    let mut pending = vec![[0., width]];
    let mut remaining = MAX_INTERVALS;
    while let Some([start, end]) = pending.pop() {
        checkpoint(interrupt)?;
        remaining = remaining.checked_sub(1).ok_or(Error::Limit)?;
        let half = (end - start) / 2.;
        let midpoint = start + half;
        let placed = std::array::from_fn(|i| {
            let base = placements[i];
            let [u, v] = base.axes;
            let (sin, cos) = gears[i].phase_at(midpoint).sin_cos();
            Placement {
                center_mm: base.center_mm,
                axes: [
                    add(scale(u, cos), scale(v, sin)),
                    add(scale(u, -sin), scale(v, cos)),
                ],
                // Circle displacement is no greater than arc length.
                uncertainty_mm: base.uncertainty_mm + radial_speed[i] * half,
            }
        });
        if placed
            .iter()
            .all(|p| p.uncertainty_mm <= MAX_TWIST_UNCERTAINTY_MM)
        {
            match certify_with_uncertainty(
                profiles,
                placed,
                tolerance_mm,
                MAX_TWIST_UNCERTAINTY_MM,
                interrupt,
            )? {
                ResultKind::Clear => continue,
                ResultKind::CommonInterior => return Ok(ResultKind::CommonInterior),
                ResultKind::Unresolved => {}
            }
        }
        if half <= tolerance_mm {
            return Ok(ResultKind::Unresolved);
        }
        pending.extend([[start, midpoint], [midpoint, end]]);
    }
    Ok(ResultKind::Clear)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spur_gear::herringbone::tests::fixture;

    #[test]
    fn coaxial_hubs_preserve_clearance_outside_the_aligned_tooth_bands() {
        use crate::spur_gear::Hub;
        let phase = std::f64::consts::PI * (1. + 1. / 36.);
        let (sin, cos) = phase.sin_cos();
        let center = 60. / 15_f64.to_radians().cos();
        let placements = [
            Placement::default(),
            Placement {
                center_mm: [center, 0.],
                axes: [[cos, sin], [-sin, cos]],
                uncertainty_mm: 0.,
            },
        ];
        for helical in [false, true] {
            let gears = [24, 36].map(|teeth| {
                let (spec, twist) = fixture(teeth);
                let gear = if helical {
                    Herringbone::new_helical(spec, twist, 1e-5, 1e7)
                } else {
                    Herringbone::new(spec, twist, 1e-5, 1e7)
                }
                .unwrap();
                gear.with_hub(
                    Hub {
                        radius_mm: 8.,
                        axial_start_mm: -10.,
                        axial_end_mm: 20.,
                    },
                    1e-5,
                    1e7,
                )
                .unwrap()
            });
            assert_eq!(
                certify_herringbone([&gears[0], &gears[1]], placements, 1e-7, None),
                Ok(ResultKind::Clear)
            );
            assert!(gears[0].axial_bounds()[0] < 0. && gears[1].axial_bounds()[1] > 4.);
            assert!(center > 2. * 8.);
        }
    }

    #[test]
    fn single_hand_and_mixed_sweeps_check_material_past_the_middle_plane() {
        let (spec_a, twist_a) = fixture(24);
        let (spec_b, twist_b) = fixture(36);
        let center = spec_a.pitch_radius_mm + spec_b.pitch_radius_mm;
        let nominal = std::f64::consts::PI * (1. + 1. / 36.);
        let at_phase = |phase: f64| {
            let (sin, cos) = phase.sin_cos();
            [
                Placement::default(),
                Placement {
                    center_mm: [center, 0.],
                    axes: [[cos, sin], [-sin, cos]],
                    uncertainty_mm: 0.,
                },
            ]
        };
        let tolerance = 1e-7;

        let linear_a = Herringbone::new_helical(spec_a, twist_a, 1e-5, 1e7).unwrap();
        let linear_b = Herringbone::new_helical(spec_b, twist_b, 1e-5, 1e7).unwrap();
        let placement = at_phase(nominal);
        assert_eq!(
            certify_herringbone([&linear_a, &linear_b], placement, tolerance, None),
            Ok(ResultKind::Clear)
        );
        assert_eq!(
            certify_herringbone(
                [&linear_a, &linear_b],
                at_phase(std::f64::consts::PI),
                tolerance,
                None
            ),
            Ok(ResultKind::CommonInterior)
        );

        let chevron_b = Herringbone::new(spec_b, twist_b, 1e-5, 1e7).unwrap();
        let reflected_a = Herringbone::new(spec_a, twist_a, 1e-5, 1e7).unwrap();
        assert_eq!(
            certify_herringbone([&reflected_a, &chevron_b], placement, tolerance, None),
            Ok(ResultKind::Clear)
        );
        assert_eq!(
            certify_herringbone([&linear_a, &chevron_b], placement, tolerance, None),
            Ok(ResultKind::Unresolved)
        );
        // The fixed pitch witness lies in B's gap. An independent point
        // above it lies strictly in both real outlines at the far cap.
        // The slab proof must preserve uncertainty about this collision.
        let (sin, cos) = linear_a.phase_at(spec_a.face_width_mm).sin_cos();
        let end_a = Placement {
            axes: [[cos, sin], [-sin, cos]],
            ..placement[0]
        };
        let witness = [spec_a.pitch_radius_mm, 1.8];
        assert!(
            Rim::build(linear_a.profile(), end_a, 1e-3, None)
                .unwrap()
                .contains_strictly(witness)
        );
        assert!(
            Rim::build(chevron_b.profile(), placement[1], 1e-3, None)
                .unwrap()
                .contains_strictly(witness)
        );
    }

    #[test]
    fn full_width_clearance_separates_contact_from_material_overlap() {
        let [a, b] = [24, 36].map(|teeth| {
            let (spec, twist) = fixture(teeth);
            Herringbone::new(spec, twist, 1e-5, 1e7).unwrap()
        });
        let pair = [&a, &b];
        let placement = |distance: f64, phase: f64| {
            let (sin, cos) = phase.sin_cos();
            [
                Placement::default(),
                Placement {
                    center_mm: [distance, 0.],
                    axes: [[cos, sin], [-sin, cos]],
                    uncertainty_mm: 0.,
                },
            ]
        };
        let center = 60. / 15_f64.to_radians().cos();
        let tolerance = 1e-7;
        let clear = placement(center, std::f64::consts::PI * (1. + 1. / 36.));
        let overlaps = placement(center, std::f64::consts::PI);
        // At the end cap the two tips are exactly tangent. The certified
        // clearance proof must not promote this to a strict separation.
        let touch = placement(a.tip_radius_mm() + b.tip_radius_mm(), std::f64::consts::PI);
        assert_eq!(
            certify_herringbone(pair, clear, tolerance, None),
            Ok(ResultKind::Clear)
        );
        assert_eq!(
            certify_herringbone(pair, overlaps, tolerance, None),
            Ok(ResultKind::CommonInterior)
        );
        assert_eq!(
            certify_herringbone(pair, touch, tolerance, None),
            Ok(ResultKind::Unresolved)
        );
        assert_eq!(
            certify_herringbone(pair, clear, tolerance, Some(&|| true)),
            Err(Error::Cancelled)
        );
        assert_eq!(
            certify_herringbone(pair, [Placement::default(); 2], tolerance, None),
            Err(Error::Invalid)
        );
        assert_eq!(
            certify_herringbone(pair, clear, f64::NAN, None),
            Err(Error::Invalid)
        );
    }
}
