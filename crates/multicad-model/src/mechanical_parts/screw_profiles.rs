//! Fresh, source-named screw profiles. No separately shared head/drive cache.
use super::{ModelError, error, hexagon, screw_boolean, screw_prism};
use multicad_core::{
    document::EntityId,
    model::{
        HexalobularContour, ResolvedPhillipsHParameters, ResolvedPhillipsInnerForm,
        ResolvedScrewDrive, ResolvedScrewHead, ResolvedThread,
    },
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RevolutionAxis,
    RigidTransform, Solid,
};
use std::f64::consts::{PI, TAU};

type Edge = (CurvedEdge2d, &'static str);
fn line(edges: &mut Vec<Edge>, a: [f64; 2], b: [f64; 2], role: &'static str) {
    if a != b {
        edges.push((CurvedEdge2d::Line { a, b }, role));
    }
}
fn revolve(
    feature: EntityId,
    component: &str,
    edges: Vec<Edge>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let origins = edges
        .iter()
        .map(|(_, role)| Origin {
            source: feature,
            role: format!("screw.{component}/{role}"),
        })
        .collect::<Vec<_>>();
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        outer: edges.into_iter().map(|(e, _)| e).collect(),
        holes: vec![],
    };
    Solid::revolve_curved_profile_named(
        &profile,
        RevolutionAxis {
            origin_mm: [0.; 3],
            direction: [0., 0., 1.],
        },
        TAU,
        feature,
        &origins,
        policy,
    )
    .map_err(|e| error(Some(feature), e))
}
pub(super) fn new_head(head: &ResolvedScrewHead) -> bool {
    matches!(
        head,
        ResolvedScrewHead::Pan { .. }
            | ResolvedScrewHead::Button { .. }
            | ResolvedScrewHead::Countersunk { .. }
    )
}
/// One meridian owns the head, nominal neck, real blend and plain shank.
/// Constructing it fresh avoids mutating any cached/source operand.
pub(super) fn blank(
    feature: EntityId,
    head: &ResolvedScrewHead,
    shank: f64,
    plain_length: f64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let mut edges = vec![];
    let height = head.height_mm();
    let top = -height;
    match *head {
        ResolvedScrewHead::Pan {
            radius_mm: r,
            crown_radius_mm: c,
            top_land_radius_mm: l,
            neck_radius_mm: n,
            ..
        }
        | ResolvedScrewHead::Button {
            radius_mm: r,
            crown_radius_mm: c,
            top_land_radius_mm: l,
            neck_radius_mm: n,
            ..
        } => {
            let center = top + (c * c - l * l).sqrt();
            let skirt = center - (c * c - r * r).sqrt();
            if ![center, skirt].into_iter().all(f64::is_finite)
                || !(top < skirt && skirt < 0.)
                || r - shank - n < policy.minimum_feature_mm
                || plain_length < n
            {
                return Err(error(
                    Some(feature),
                    "Crowned screw profile or neck blend does not fit the retained plain transition",
                ));
            }
            line(&mut edges, [0., top], [l, top], "top.land");
            edges.push((
                CurvedEdge2d::Arc {
                    center: [0., center],
                    a: [l, top],
                    b: [r, skirt],
                    clockwise: false,
                },
                "crown",
            ));
            line(&mut edges, [r, skirt], [r, 0.], "skirt");
            line(&mut edges, [r, 0.], [shank + n, 0.], "bearing");
            if n > 0. {
                edges.push((
                    CurvedEdge2d::Arc {
                        center: [shank + n, n],
                        a: [shank + n, 0.],
                        b: [shank, n],
                        clockwise: true,
                    },
                    "neck.blend",
                ));
            }
            line(&mut edges, [shank, n], [shank, plain_length], "shank");
        }
        ResolvedScrewHead::Countersunk {
            radius_mm: r,
            included_angle_deg,
            edge_chamfer_mm: ch,
            neck_radius_mm: n,
            ..
        } => {
            let theta = (included_angle_deg / 2.).to_radians();
            let rim = height - (r - shank) / theta.tan();
            let neck_z = n * (theta / 2.).tan();
            if !rim.is_finite() || rim < 0. || ch < 0. || ch > rim || plain_length < neck_z {
                return Err(error(
                    Some(feature),
                    "Countersunk rim, chamfer or neck blend does not fit the retained screw profile",
                ));
            }
            let center = [shank + n, neck_z];
            let tangent = [center[0] - n * theta.cos(), center[1] - n * theta.sin()];
            line(&mut edges, [0., top], [r - ch, top], "top.land");
            line(&mut edges, [r - ch, top], [r, top + ch], "top.chamfer");
            line(&mut edges, [r, top + ch], [r, top + rim], "rim");
            line(&mut edges, [r, top + rim], tangent, "cone");
            if n > 0. {
                edges.push((
                    CurvedEdge2d::Arc {
                        center,
                        a: tangent,
                        b: [shank, neck_z],
                        clockwise: true,
                    },
                    "neck.blend",
                ));
            }
            line(&mut edges, [shank, neck_z], [shank, plain_length], "shank");
        }
        _ => {
            return Err(error(
                Some(feature),
                "New screw profile builder received a legacy head",
            ));
        }
    }
    line(
        &mut edges,
        [shank, plain_length],
        [0., plain_length],
        "thread.junction",
    );
    line(&mut edges, [0., plain_length], [0., top], "axis");
    revolve(feature, "head", edges, policy)
}
fn extrude(
    feature: EntityId,
    component: &str,
    profile: CurvedProfile,
    roles: &[String],
    displacement: [f64; 3],
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    if roles.len() != profile.outer.len() {
        return Err(error(
            Some(feature),
            "Screw profile requires an authored role for every curve",
        ));
    }
    let origins = roles
        .iter()
        .map(|role| Origin {
            source: feature,
            role: format!("screw.{component}/{role}"),
        })
        .collect::<Vec<_>>();
    Solid::extrude_curved_profile_vector_named(&profile, displacement, feature, &origins, policy)
        .and_then(|s| {
            s.with_feature_cap_roles(
                feature,
                &format!("screw.{component}/start"),
                &format!("screw.{component}/end"),
            )
        })
        .map_err(|e| error(Some(feature), e))
}
fn conical_bottom(
    feature: EntityId,
    component: &str,
    tip: f64,
    angle: f64,
    reach: f64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let half = (angle / 2.).to_radians();
    let upper = tip - reach / half.tan();
    revolve(
        feature,
        component,
        vec![
            (
                CurvedEdge2d::Line {
                    a: [0., tip],
                    b: [reach, upper],
                },
                "cone",
            ),
            (
                CurvedEdge2d::Line {
                    a: [reach, upper],
                    b: [0., upper],
                },
                "closure",
            ),
            (
                CurvedEdge2d::Line {
                    a: [0., upper],
                    b: [0., tip],
                },
                "axis",
            ),
        ],
        policy,
    )
}
fn hexalobular(
    feature: EntityId,
    head: &ResolvedScrewHead,
    dimensions: [f64; 5],
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let [major, minor, lobe, depth, angle] = dimensions;
    let top = -head.height_mm();
    let tip = top + depth + major / (angle / 2.).to_radians().tan();
    let contour = HexalobularContour::new(2. * major, 2. * minor, lobe)
        .map_err(|e| error(Some(feature), e))?;
    let margin = major.max(1.);
    let start = top - margin;
    let outer = contour
        .arcs
        .iter()
        .map(|a| CurvedEdge2d::Arc {
            center: a.center,
            a: a.start,
            b: a.end,
            clockwise: !a.convex,
        })
        .collect();
    let roles = contour
        .arcs
        .iter()
        .enumerate()
        .map(|(i, arc)| format!("{}/{}", if arc.convex { "lobe" } else { "valley" }, i / 2))
        .collect::<Vec<_>>();
    let prism = extrude(
        feature,
        "hexalobular",
        CurvedProfile {
            frame: PlaneFrame {
                origin: [0., 0., start],
                ..Default::default()
            },
            outer,
            holes: vec![],
        },
        &roles,
        [0., 0., tip - start + margin],
        policy,
    )?;
    // The finite cone's closure is outside the entire prism/head. A radius
    // based only on the lobes can close inside a deep functional region and
    // leave a false cap across the physical mouth.
    let cone_reach = (tip - start + margin) * (angle / 2.).to_radians().tan();
    let cone = conical_bottom(
        feature,
        "hexalobular.bottom",
        tip,
        angle,
        cone_reach,
        policy,
    )?;
    screw_boolean(feature, &prism, &cone, BooleanOperation::Intersect, policy)
}
fn phillips(
    feature: EntityId,
    head: &ResolvedScrewHead,
    p: &ResolvedPhillipsHParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let top = -head.height_mm();
    let a = p.outer_flank_angle_deg.to_radians();
    let floor = p.floor_angle_deg.to_radians();
    let beta = p.tilt_angle_deg.to_radians();
    let theta = (p.outer_corner_angle_deg / 2.).to_radians();
    let m = p.mouth_diameter_mm;
    let g = p.g_diameter_mm;
    let b = p.b_width_mm;
    let r = p.mouth_radius_mm;
    let tg = (m - g) / (2. * a.tan());
    let total = tg + g / 2. * floor.tan();
    let tb = tg + (g - b) / 2. * floor.tan();
    let tip = top + total;
    let center = [m / 2. + r * (1. - a.sin()) / a.cos(), top + r];
    let qhead = [center[0], top];
    let qflank = [center[0] - r * a.cos(), center[1] - r * a.sin()];
    if qflank[1] >= top + tg - policy.minimum_feature_mm {
        return Err(error(
            Some(feature),
            "Phillips mouth roundover consumes the complete outer flank",
        ));
    }
    let margin = m.max(1.);
    let above = top - margin;
    let mut cutter = revolve(
        feature,
        "phillips.envelope",
        vec![
            (
                CurvedEdge2d::Line {
                    a: [0., above],
                    b: [qhead[0], above],
                },
                "closure",
            ),
            (
                CurvedEdge2d::Line {
                    a: [qhead[0], above],
                    b: qhead,
                },
                "entry.extension",
            ),
            (
                CurvedEdge2d::Arc {
                    center,
                    a: qhead,
                    b: qflank,
                    clockwise: true,
                },
                "entry.roundover",
            ),
            (
                CurvedEdge2d::Line {
                    a: qflank,
                    b: [g / 2., top + tg],
                },
                "outer.flank",
            ),
            (
                CurvedEdge2d::Line {
                    a: [g / 2., top + tg],
                    b: [0., tip],
                },
                "floor",
            ),
            (
                CurvedEdge2d::Line {
                    a: [0., tip],
                    b: [0., above],
                },
                "axis",
            ),
        ],
        policy,
    )?;
    // Every corner is constructed in its true C-C plane normal to D. A circle
    // here has horizontal section Rx=R/cos(beta), not projected Rx=R*cos(beta).
    let zlo = above - margin;
    let zhi = tip + margin;
    let rb = b / 2. + (top + tb - zlo) * beta.tan();
    let reach = 4. * (head.maximum_radius_mm() + m + tip.abs() + 1.);
    let mut edges = vec![];
    let roles: Vec<String>;
    match p.inner {
        ResolvedPhillipsInnerForm::ShallowV {
            angle_deg,
            e_width_mm: e,
        } => {
            roles = [
                "inner.upper",
                "outer.upper",
                "closure",
                "outer.lower",
                "inner.lower",
            ]
            .map(str::to_owned)
            .into();
            let de = e / (2. * (angle_deg / 2.).to_radians().tan());
            let upper = [de, e / 2.];
            let lower = [de, -e / 2.];
            let far = [de + reach * theta.cos(), e / 2. + reach * theta.sin()];
            edges.extend([
                CurvedEdge2d::Line {
                    a: [0., 0.],
                    b: upper,
                },
                CurvedEdge2d::Line { a: upper, b: far },
                CurvedEdge2d::Line {
                    a: far,
                    b: [far[0], -far[1]],
                },
                CurvedEdge2d::Line {
                    a: [far[0], -far[1]],
                    b: lower,
                },
                CurvedEdge2d::Line {
                    a: lower,
                    b: [0., 0.],
                },
            ]);
        }
        ResolvedPhillipsInnerForm::Rounded { radius_mm: radius } => {
            roles = ["inner.round", "outer.upper", "closure", "outer.lower"]
                .map(str::to_owned)
                .into();
            let upper = [radius - radius * theta.sin(), radius * theta.cos()];
            let lower = [upper[0], -upper[1]];
            let far = [
                upper[0] + reach * theta.cos(),
                upper[1] + reach * theta.sin(),
            ];
            edges.extend([
                CurvedEdge2d::Arc {
                    center: [radius, 0.],
                    a: lower,
                    b: upper,
                    clockwise: true,
                },
                CurvedEdge2d::Line { a: upper, b: far },
                CurvedEdge2d::Line {
                    a: far,
                    b: [far[0], -far[1]],
                },
                CurvedEdge2d::Line {
                    a: [far[0], -far[1]],
                    b: lower,
                },
            ]);
        }
    }
    let corner = extrude(
        feature,
        "phillips.corner.0",
        CurvedProfile {
            frame: PlaneFrame {
                origin: [rb, 0., zlo],
                u: [beta.cos(), 0., beta.sin()],
                v: [0., 1., 0.],
            },
            outer: edges.clone(),
            holes: vec![],
        },
        &roles,
        [-(zhi - zlo) * beta.tan(), 0., zhi - zlo],
        policy,
    )?;
    // Fresh independently named constructions retain each wing's authored role.
    for i in 0..4 {
        let angle = i as f64 * PI / 2.;
        let (s, c) = angle.sin_cos();
        let current = if i == 0 {
            corner.share().map_err(|e| error(Some(feature), e))?
        } else {
            let ccorner = extrude(
                feature,
                &format!("phillips.corner.{i}"),
                CurvedProfile {
                    frame: PlaneFrame {
                        origin: [rb, 0., zlo],
                        u: [beta.cos(), 0., beta.sin()],
                        v: [0., 1., 0.],
                    },
                    outer: edges.clone(),
                    holes: vec![],
                },
                &roles,
                [-(zhi - zlo) * beta.tan(), 0., zhi - zlo],
                policy,
            )?;
            ccorner
                .transformed(RigidTransform {
                    rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
                    translation_mm: [0.; 3],
                })
                .map_err(|e| error(Some(feature), e))?
        };
        cutter = screw_boolean(
            feature,
            &cutter,
            &current,
            BooleanOperation::Subtract,
            policy,
        )?;
    }
    Ok(cutter)
}
pub(super) fn cutter(
    feature: EntityId,
    head: &ResolvedScrewHead,
    drive: &ResolvedScrewDrive,
    policy: GeometryPolicy,
) -> Result<Option<Solid>, ModelError> {
    Ok(match drive {
        ResolvedScrewDrive::ExternalHex => None,
        ResolvedScrewDrive::HexSocket {
            across_flats_mm,
            depth_mm,
        } => Some(screw_prism(
            feature,
            "socket",
            -head.height_mm(),
            *depth_mm,
            hexagon(*across_flats_mm),
            policy,
        )?),
        ResolvedScrewDrive::Hexalobular {
            major_radius_mm,
            minor_radius_mm,
            outer_lobe_radius_mm,
            depth_mm,
            bottom_angle_deg,
        } => Some(hexalobular(
            feature,
            head,
            [
                *major_radius_mm,
                *minor_radius_mm,
                *outer_lobe_radius_mm,
                *depth_mm,
                *bottom_angle_deg,
            ],
            policy,
        )?),
        ResolvedScrewDrive::PhillipsH { profile } => {
            Some(phillips(feature, head, profile, policy)?)
        }
    })
}

/// Canonical source proves positive envelopes. Native construction additionally
/// respects this owner's numeric minimum feature, independently of catalog or
/// ISO wall/floor recommendations. No head-height shortcut forbids a CSK neck.
pub(super) fn validate_native_clearance(
    feature: EntityId,
    head: &ResolvedScrewHead,
    drive: &ResolvedScrewDrive,
    shank: f64,
    underhead: f64,
    thread: &ResolvedThread,
    policy: GeometryPolicy,
) -> Result<(), ModelError> {
    if matches!(drive, ResolvedScrewDrive::ExternalHex) {
        return Ok(());
    }
    let fail = || {
        error(
            Some(feature),
            "Screw profile clearance is below the native minimum feature size or cannot be bounded safely",
        )
    };
    let minimum = policy.minimum_feature_mm;
    let k = head.height_mm();
    let depth = drive.removed_depth_mm();
    let plain = underhead - thread.length_mm;
    if head.top_land_radius_mm() - drive.entry_radius_mm() < minimum
        || k + underhead - depth < minimum
    {
        return Err(fail());
    }
    let body = |d: f64| {
        let z = d - k;
        if z >= plain {
            let mut major = thread.major_radius_mm + thread.profile.radial_taper * (z - plain);
            if let Some(limit) = thread
                .npt
                .as_ref()
                .and_then(|p| p.stock_crest_radius_limit_mm)
            {
                major = major.min(limit);
            }
            major - thread.profile.depth_mm
        } else {
            head.radial_envelope_mm(z, shank)
        }
    };
    let mut breaks = vec![0., depth, k, k + plain, k + underhead];
    match *head {
        ResolvedScrewHead::Pan {
            radius_mm,
            crown_radius_mm,
            top_land_radius_mm,
            neck_radius_mm,
            ..
        }
        | ResolvedScrewHead::Button {
            radius_mm,
            crown_radius_mm,
            top_land_radius_mm,
            neck_radius_mm,
            ..
        } => {
            let center = -k + (crown_radius_mm.powi(2) - top_land_radius_mm.powi(2)).sqrt();
            let skirt = center - (crown_radius_mm.powi(2) - radius_mm.powi(2)).sqrt();
            breaks.extend([k + skirt, k + neck_radius_mm]);
        }
        ResolvedScrewHead::Countersunk {
            radius_mm,
            included_angle_deg,
            edge_chamfer_mm,
            neck_radius_mm,
            ..
        } => {
            let theta = (included_angle_deg / 2.).to_radians();
            let rim = k - (radius_mm - shank) / theta.tan();
            let center = neck_radius_mm * (theta / 2.).tan();
            breaks.extend([
                edge_chamfer_mm,
                rim,
                k + center - neck_radius_mm * theta.sin(),
                k + center,
            ]);
        }
        _ => {}
    }
    match drive {
        ResolvedScrewDrive::Hexalobular { depth_mm, .. } => breaks.push(*depth_mm),
        ResolvedScrewDrive::PhillipsH { profile } => breaks.extend([
            profile.mouth_radius_mm * (1. - profile.outer_flank_angle_deg.to_radians().sin()),
            (profile.mouth_diameter_mm - profile.g_diameter_mm)
                / (2. * profile.outer_flank_angle_deg.to_radians().tan()),
        ]),
        _ => {}
    }
    breaks.retain(|d| d.is_finite() && *d >= 0. && *d <= depth);
    breaks.sort_by(f64::total_cmp);
    breaks.dedup();
    let mut pending = breaks
        .windows(2)
        .map(|w| (w[0], w[1], 0u32))
        .collect::<Vec<_>>();
    let mut subdivisions = 0;
    while let Some((left, right, level)) = pending.pop() {
        let a = body(left);
        let b = body(right);
        let ca = drive.radial_envelope_mm(left);
        let cb = drive.radial_envelope_mm(right);
        if ![a, b, ca, cb].into_iter().all(f64::is_finite) {
            return Err(fail());
        }
        // The source branches are monotone between the explicit crown, rim,
        // cone, neck and drive joins. Endpoint minimum(body)-maximum(cutter)
        // therefore bounds every radial section, rather than sampling it.
        if a.min(b) - ca.max(cb) >= minimum {
            continue;
        }
        if a - ca < minimum || b - cb < minimum || level >= 48 || subdivisions >= 16384 {
            return Err(fail());
        }
        let mid = left + (right - left) / 2.;
        if mid == left || mid == right {
            return Err(fail());
        }
        subdivisions += 1;
        pending.push((left, mid, level + 1));
        pending.push((mid, right, level + 1));
    }
    Ok(())
}
