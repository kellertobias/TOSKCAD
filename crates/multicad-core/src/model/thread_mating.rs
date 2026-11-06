//! Placement-derived complete-profile engagement and helical phase.
//! These analytical checks do not replace native collision or receiving-wall checks.
use super::{
    MetricFitEngagementBasis, ResolvedMechanicalPart, ResolvedThread, RigidPose, ThreadFit,
    ThreadHand, ThreadRepresentation, align_frames,
};
use crate::{document::DocumentError, references::ReferenceFrame};
use serde::Serialize;
use std::f64::consts::TAU;

fn invalid(message: &str) -> DocumentError {
    DocumentError::Invalid(message.into())
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn difference(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ThreadPhaseCorrection {
    /// Roll about the follower's own directed thread axis, not world Z.
    pub roll_rad: f64,
    /// World-space correction that holds the follower's thread origin fixed.
    pub correction: RigidPose,
    /// Follower origin measured along the reference's directed axis.
    pub axial_station_mm: f64,
}

/// Physical helix-start frame of a resolved machine screw. The retained thread
/// frame locates the whole screw at its underhead datum. Native construction
/// starts its helix after the unthreaded shank, U minus retained thread length.
pub fn machine_screw_thread_start(
    part: &ResolvedMechanicalPart,
    occurrence_pose: RigidPose,
) -> Result<ReferenceFrame, DocumentError> {
    occurrence_pose.validate()?;
    let ResolvedMechanicalPart::MachineScrew {
        underhead_length_mm,
        thread,
        ..
    } = part
    else {
        return Err(invalid(
            "A phase reference requires a machine screw with a known helical start",
        ));
    };
    let plain = underhead_length_mm - thread.length_mm;
    if !plain.is_finite() || plain < 0. {
        return Err(invalid(
            "The machine screw has an invalid unthreaded length",
        ));
    }
    let start = ReferenceFrame {
        origin_mm: std::array::from_fn(|i| thread.frame.origin[i] + plain * thread.frame.n[i]),
        u: thread.frame.u,
        v: thread.frame.v,
    };
    start.validate().map_err(|e| invalid(&e))?;
    let world = occurrence_pose.frame(start);
    world.validate().map_err(|e| invalid(&e))?;
    Ok(world)
}

/// Align two single-start symmetric 60-degree straight helices following the native generator's
/// +U start and handed lead convention. Frames must denote their actual helix
/// starts in the same world coordinates. The follower may have the opposite
/// directed axis; a proper rotation never changes the declared thread hand.
///
/// Nominal diameter, pitch, starts and hand are checked here. The caller must
/// separately establish compatible families/profiles/fits, actual full-profile
/// intervals and native non-interference. Cosmetic envelopes gain no modeled
/// geometry or manufacturing permission by receiving this numerical result.
pub fn straight_thread_phase(
    reference: &ResolvedThread,
    reference_start: ReferenceFrame,
    follower: &ResolvedThread,
    follower_start: ReferenceFrame,
    linear_tolerance_mm: f64,
) -> Result<ThreadPhaseCorrection, DocumentError> {
    reference_start.validate().map_err(|e| invalid(&e))?;
    follower_start.validate().map_err(|e| invalid(&e))?;
    if !linear_tolerance_mm.is_finite() || linear_tolerance_mm <= 0. {
        return Err(invalid(
            "Phase alignment needs a finite positive linear tolerance",
        ));
    }
    let same = |a: f64, b: f64| {
        a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-10 * a.abs().max(b.abs()).max(1.)
    };
    if reference.starts != 1
        || reference.starts != follower.starts
        || reference.hand != follower.hand
        || !same(
            reference.nominal_major_radius_mm,
            follower.nominal_major_radius_mm,
        )
        || !same(reference.pitch_mm, follower.pitch_mm)
        || !same(reference.lead_mm, follower.lead_mm)
        || reference.pitch_mm <= 0.
        || reference.nominal_major_radius_mm <= 0.
        || reference.lead_mm <= 0.
        || !same(
            reference.lead_mm,
            reference.pitch_mm * f64::from(reference.starts),
        )
    {
        return Err(invalid(
            "Thread phase requires matching positive nominal diameter, pitch, hand, starts and lead",
        ));
    }
    if reference.profile.radial_taper != 0. || follower.profile.radial_taper != 0. {
        return Err(invalid(
            "This phase alignment requires straight threads; tapered threads need their gauge-position contract",
        ));
    }
    for thread in [reference, follower] {
        if thread.buttress.is_some()
            || thread.round.is_some()
            || thread.lead_screw.is_some()
            || thread.pipe.is_some()
            || thread.npt.is_some()
            || thread.profile.lower_flank_deg != 30.
            || thread.profile.upper_flank_deg != 30.
        {
            return Err(invalid(
                "This phase alignment requires the supported symmetric straight profile; other families need their native phase contract",
            ));
        }
    }
    let normal = reference_start.normal();
    if dot(normal, follower_start.normal()).abs() < 1. - 1e-9 {
        return Err(invalid(
            "The mating thread axes must be parallel or opposed",
        ));
    }
    let separation = difference(follower_start.origin_mm, reference_start.origin_mm);
    let station = dot(separation, normal);
    let radial = std::array::from_fn(|i| separation[i] - station * normal[i]);
    if dot(radial, radial).sqrt() > linear_tolerance_mm {
        return Err(invalid("The mating thread axes are not coincident"));
    }
    let sign = match reference.hand {
        ThreadHand::Right => 1.,
        ThreadHand::Left => -1.,
    };
    // Reduce distance before multiplying, including negative stations. This
    // avoids unnecessarily large trigonometric arguments on long assemblies.
    let angle = sign * TAU * station.rem_euclid(reference.lead_mm) / reference.lead_mm;
    let (sin, cos) = angle.sin_cos();
    let crest = std::array::from_fn(|i| reference_start.u[i] * cos + reference_start.v[i] * sin);
    let raw = dot(crest, follower_start.v).atan2(dot(crest, follower_start.u));
    let period = TAU / f64::from(reference.starts);
    let roll = (raw + period / 2.).rem_euclid(period) - period / 2.;
    let (sin, cos) = roll.sin_cos();
    let target = ReferenceFrame {
        origin_mm: follower_start.origin_mm,
        u: std::array::from_fn(|i| follower_start.u[i] * cos + follower_start.v[i] * sin),
        v: std::array::from_fn(|i| follower_start.v[i] * cos - follower_start.u[i] * sin),
    };
    let correction = align_frames(follower_start, target, [0.; 3], 0., false)?;
    Ok(ThreadPhaseCorrection {
        roll_rad: roll,
        correction,
        axial_station_mm: station,
    })
}

/// Derived from placed canonical complete-profile intervals, never an authored
/// claim that the components happen to engage. Native solids must still be
/// checked after applying `phase.correction` to the follower occurrence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedMetricThreadMating {
    pub overlap_start_mm: f64,
    pub overlap_end_mm: f64,
    pub overlap_length_mm: f64,
    pub phase: ThreadPhaseCorrection,
    pub scope: &'static str,
}

pub fn resolve_metric_thread_mating(
    external: &ResolvedThread,
    external_start: ReferenceFrame,
    internal: &ResolvedThread,
    internal_start: ReferenceFrame,
    minimum_engagement_mm: f64,
    linear_tolerance_mm: f64,
) -> Result<ResolvedMetricThreadMating, DocumentError> {
    if !minimum_engagement_mm.is_finite() || minimum_engagement_mm <= 0. {
        return Err(invalid(
            "Required complete-thread engagement must be finite and positive",
        ));
    }
    for (thread, class) in [
        (external, ThreadFit::Metric6gExternal),
        (internal, ThreadFit::Metric6HInternal),
    ] {
        if thread.representation != ThreadRepresentation::Modeled
            || thread.fit.as_ref().is_none_or(|fit| fit.class != class)
            || thread.metric_fit.is_none()
            || thread.metric_fit_engagement.is_none()
        {
            return Err(invalid(
                "Metric mating requires modeled sourced 6g external and 6H internal threads with known complete-profile intervals",
            ));
        }
    }
    let phase = straight_thread_phase(
        external,
        external_start,
        internal,
        internal_start,
        linear_tolerance_mm,
    )?;
    // Near-parallel axes can drift outside tolerance over a long component.
    // Check the physical far endpoint as well as the origin checked by phase.
    let normal = external_start.normal();
    let endpoint = std::array::from_fn(|i| {
        internal_start.origin_mm[i] + internal_start.normal()[i] * internal.length_mm
    });
    let offset = difference(endpoint, external_start.origin_mm);
    let station = dot(offset, normal);
    let radial = std::array::from_fn(|i| offset[i] - station * normal[i]);
    if dot(radial, radial).sqrt() > linear_tolerance_mm {
        return Err(invalid(
            "Mating axes diverge beyond tolerance across the physical thread",
        ));
    }
    let complete = |thread: &ResolvedThread| -> Result<(f64, f64), DocumentError> {
        let interval = thread
            .end_treatments
            .as_ref()
            .map_or((0., thread.length_mm), |t| {
                (t.full_profile_start_mm, t.full_profile_end_mm)
            });
        if !interval.0.is_finite()
            || !interval.1.is_finite()
            || interval.0 < 0.
            || interval.1 > thread.length_mm
            || interval.1 <= interval.0
        {
            return Err(invalid(
                "Mating requires a known positive complete-profile interval inside the physical thread",
            ));
        }
        Ok(interval)
    };
    let (a0, a1) = complete(external)?;
    let (b0, b1) = complete(internal)?;
    let direction = dot(external_start.normal(), internal_start.normal());
    let b0 = phase.axial_station_mm + direction * b0;
    let b1 = phase.axial_station_mm + direction * b1;
    let start = a0.max(b0.min(b1));
    let end = a1.min(b0.max(b1));
    let overlap = end - start;
    if overlap <= 0. || overlap < minimum_engagement_mm {
        return Err(invalid(
            "Placed complete thread profiles have insufficient engagement; adjust screw length, stack or mate position",
        ));
    }
    for thread in [external, internal] {
        let fit = thread.metric_fit.as_ref().expect("validated metric fit");
        if overlap <= fit.preset.engagement_over_mm || overlap > fit.preset.engagement_up_to_mm {
            return Err(invalid(
                "Actual placed engagement is outside the supported normal fit interval",
            ));
        }
        let intent = thread
            .metric_fit_engagement
            .as_ref()
            .expect("validated intent");
        if intent.basis == MetricFitEngagementBasis::IntendedLength
            && intent.intended_length_mm.is_none_or(|length| {
                !length.is_finite() || (length - overlap).abs() > linear_tolerance_mm
            })
        {
            return Err(invalid(
                "Declared intended engagement disagrees with actual placed complete-profile overlap",
            ));
        }
    }
    Ok(ResolvedMetricThreadMating {
        overlap_start_mm: start,
        overlap_end_mm: end,
        overlap_length_mm: overlap,
        phase,
        scope: "Analytical overlap of known placed complete profiles and native helix phase; native component collision, receiving material, blind clearance and strength checks remain separate",
    })
}
