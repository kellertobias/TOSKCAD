//! B-rep hidden-line and section operations. Native handles remain local to the
//! caller's geometry worker; only owned immutable curve records leave it.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid, ffi};
use multicad_core::drawings::{VectorCurve, VectorRole};
mod curved;
mod panel;
mod planar;
mod section;
mod sphere;
mod torus;

#[derive(Clone, Copy, Debug)]
pub struct DrawingProjection {
    pub frame: PlaneFrame,
    pub section_offset_mm: Option<f64>,
    /// Section only: draw the exact plane intersection and nothing behind it.
    pub section_only: bool,
    /// Finite viewing depth behind the section plane; `None` is unlimited.
    pub section_depth_mm: Option<f64>,
    /// Lateral section limits in frame u and v, from the frame origin.
    pub section_u_mm: Option<[f64; 2]>,
    pub section_v_mm: Option<[f64; 2]>,
    pub detail: Option<([f64; 2], f64)>,
    pub hidden_lines: bool,
    pub tangent_edges: bool,
    pub chord_tolerance_mm: f64,
    pub maximum_curves: u32,
    pub maximum_points: u32,
}
impl Default for DrawingProjection {
    fn default() -> Self {
        Self {
            frame: PlaneFrame::default(),
            section_offset_mm: None,
            section_only: false,
            section_depth_mm: None,
            section_u_mm: None,
            section_v_mm: None,
            detail: None,
            hidden_lines: false,
            tangent_edges: false,
            chord_tolerance_mm: 0.01,
            maximum_curves: 100_000,
            maximum_points: 1_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DrawingHatch {
    pub angle_rad: f64,
    pub spacing_mm: f64,
    pub line_width_mm: f64,
    pub pattern: multicad_core::materials::HatchPattern,
}
pub struct DrawingSolid<'a> {
    pub solid: &'a Solid,
    pub hatch: Option<DrawingHatch>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedVector {
    pub source_index: usize,
    pub role: VectorRole,
    pub curve: VectorCurve,
    pub preview_mm: Vec<[f64; 2]>,
}
fn err(kind: GeometryErrorKind, message: impl ToString) -> GeometryError {
    GeometryError {
        operation: "drawing projection",
        kind,
        message: message.to_string(),
    }
}
fn p(p: [f64; 3]) -> ffi::Point3 {
    ffi::Point3 {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}
fn projection_members<'a>(solids: &[DrawingSolid<'a>]) -> Vec<(usize, &'a Solid)> {
    fn leaves<'a>(solid: &'a Solid, source: usize, output: &mut Vec<(usize, &'a Solid)>) {
        if let Some(compound) = solid.rust_compound() {
            for member in compound.members() {
                leaves(member, source, output);
            }
        } else {
            output.push((source, solid));
        }
    }
    let mut members = Vec::new();
    for (source, input) in solids.iter().enumerate() {
        leaves(input.solid, source, &mut members);
    }
    members
}

fn contains_loft(solid: &Solid) -> bool {
    solid.nonplanar_loft_body().is_some()
        || solid
            .rust_compound()
            .is_some_and(|compound| compound.members().any(contains_loft))
}

fn contains_sphere(solid: &Solid) -> bool {
    solid.sphere_body().is_some()
        || solid
            .rust_compound()
            .is_some_and(|compound| compound.members().any(contains_sphere))
}

fn contains_bevel(solid: &Solid) -> bool {
    solid.bevel_body().is_some()
        || solid
            .rust_compound()
            .is_some_and(|compound| compound.members().any(contains_bevel))
}

fn contains_herringbone(solid: &Solid) -> bool {
    solid.herringbone_body().is_some()
        || solid
            .rust_compound()
            .is_some_and(|compound| compound.members().any(contains_herringbone))
}

pub fn project_drawing(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let policy = GeometryPolicy::default();
    let frame = options.frame.checked(policy)?;
    if solids.is_empty()
        || solids.len() > 4096
        || !options.chord_tolerance_mm.is_finite()
        || !(1e-7..=1e4).contains(&options.chord_tolerance_mm)
        || options.maximum_curves == 0
        || options.maximum_curves > 200_000
        || options.maximum_points == 0
        || options.maximum_points > 2_000_000
        || options
            .section_offset_mm
            .is_some_and(|v| !v.is_finite() || v.abs() > 1e7)
        || (options.section_offset_mm.is_none()
            && (options.section_only
                || options.section_depth_mm.is_some()
                || options.section_u_mm.is_some()
                || options.section_v_mm.is_some()))
        || (options.section_only && options.section_depth_mm.is_some())
        || options
            .section_depth_mm
            .is_some_and(|d| !d.is_finite() || !(1e-5..=1e7).contains(&d))
        || [options.section_u_mm, options.section_v_mm]
            .into_iter()
            .flatten()
            .any(|[a, b]| {
                !a.is_finite() || !b.is_finite() || a.abs() > 1e7 || b.abs() > 1e7 || b - a < 1e-5
            })
        || options.detail.is_some_and(|(c, r)| {
            c.iter().any(|v| !v.is_finite() || v.abs() > 1e7)
                || !r.is_finite()
                || !(1e-5..=1e7).contains(&r)
        })
    {
        return Err(err(
            GeometryErrorKind::Input,
            "Invalid drawing source, tolerance, section/detail region or budget",
        ));
    }
    for input in solids {
        if input.hatch.is_some_and(|h| {
            !h.angle_rad.is_finite()
                || !h.spacing_mm.is_finite()
                || (matches!(
                    h.pattern,
                    multicad_core::materials::HatchPattern::Alternating
                        | multicad_core::materials::HatchPattern::Dots
                ) && (!h.line_width_mm.is_finite() || h.line_width_mm <= 0.))
                || !(1e-5..=1e7).contains(&h.spacing_mm)
        }) {
            return Err(err(
                GeometryErrorKind::Input,
                "Invalid section hatch specification",
            ));
        }
    }
    if solids.iter().any(|input| input.solid.uses_rust_geometry()) {
        if solids.iter().any(|input| !input.solid.uses_rust_geometry()) {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust drawing projection requires all sources to use Rust geometry",
            ));
        }
        if let [input] = solids
            && let Some((sphere, frame)) = input.solid.sphere_body()
            && (options.section_offset_mm.is_none()
                || options.section_only
                || (options.section_depth_mm.is_none()
                    && options.section_u_mm.is_none()
                    && options.section_v_mm.is_none()))
        {
            return sphere::project(input, frame.origin, sphere.radius_mm(), options);
        }
        if let [input] = solids
            && let Some((torus, frame)) = input.solid.torus_body()
        {
            return torus::project(input, frame, torus.radii_mm(), options);
        }
        if !options.section_only
            && let [input] = solids
            && let Some(projection) = panel::project_herringbone(input, options)
        {
            return projection;
        }
        if solids.iter().any(|input| contains_bevel(input.solid)) {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust exact drawing projection for spherical bevel gears is unavailable",
            ));
        }
        if !options.section_only && solids.iter().any(|input| contains_herringbone(input.solid)) {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust exact drawing projection for continuous herringbone gears is unavailable",
            ));
        }
        if let [input] = solids
            && let Some(projection) = panel::project_spline_sweep(input, options)
        {
            return projection;
        }
        if solids
            .iter()
            .any(|input| panel::contains_spline_sweep(input.solid))
        {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust spline sweep drawing supports one solid at a time; scene occlusion is unavailable",
            ));
        }
        if let Some(depth) = options.section_offset_mm {
            if let [input] = solids
                && input.solid.annulus_body().is_some()
            {
                return if options.section_only {
                    section::project_circular(input, options, depth)
                } else {
                    curved::project_annulus_depth(input, options, depth)
                };
            }
            return section::project(solids, options, depth);
        }
        if let [input] = solids
            && input.solid.nonplanar_loft_body().is_some()
        {
            return section::project_loft_view(input, options);
        }
        if solids.iter().any(|input| contains_loft(input.solid)) {
            return section::project_loft_scene(&projection_members(solids), options);
        }
        if let [input] = solids
            && let Some(projection) = panel::project(input, options)
                .or_else(|| panel::project_chamfered_bores(input, options))
                .or_else(|| panel::project_normal_sweep(input, options))
                .or_else(|| panel::project_spur_gear(input, options))
        {
            return projection;
        }
        if solids
            .iter()
            .any(|input| curved::needs_curved_scene(input.solid))
        {
            return curved::project_scene(solids, options);
        }
        return planar::project(solids, options);
    }
    let (center, radius) = options.detail.unwrap_or(([0.; 2], 1.));
    let data = ffi::DrawingOptionsData {
        frame,
        section: options.section_offset_mm.is_some(),
        section_offset: options.section_offset_mm.unwrap_or(0.),
        section_only: options.section_only,
        depth: options.section_depth_mm.is_some(),
        depth_mm: options.section_depth_mm.unwrap_or(0.),
        window_u: options.section_u_mm.is_some(),
        u_min: options.section_u_mm.map_or(0., |r| r[0]),
        u_max: options.section_u_mm.map_or(0., |r| r[1]),
        window_v: options.section_v_mm.is_some(),
        v_min: options.section_v_mm.map_or(0., |r| r[0]),
        v_max: options.section_v_mm.map_or(0., |r| r[1]),
        detail: options.detail.is_some(),
        detail_center: p([center[0], center[1], 0.]),
        detail_radius: radius,
        tolerance: options.chord_tolerance_mm,
        hidden: options.hidden_lines,
        tangents: options.tangent_edges,
        max_curves: options.maximum_curves,
        max_points: options.maximum_points,
    };
    let mut job = ffi::drawing_job(&data).map_err(|e| err(GeometryErrorKind::Native, e))?;
    for input in solids {
        let hatch = input.hatch.unwrap_or(DrawingHatch {
            angle_rad: 0.,
            spacing_mm: 1.,
            line_width_mm: 0.18,
            pattern: multicad_core::materials::HatchPattern::None,
        });
        ffi::drawing_add(
            job.pin_mut(),
            input.solid.occt()?,
            match hatch.pattern {
                multicad_core::materials::HatchPattern::None => 0,
                multicad_core::materials::HatchPattern::Lines => 1,
                multicad_core::materials::HatchPattern::Cross => 2,
                multicad_core::materials::HatchPattern::Alternating => 3,
                multicad_core::materials::HatchPattern::Solid => 4,
                multicad_core::materials::HatchPattern::Dots => 5,
                multicad_core::materials::HatchPattern::SmallCrosses => 6,
                multicad_core::materials::HatchPattern::Brick => 7,
                multicad_core::materials::HatchPattern::Concrete => 8,
                multicad_core::materials::HatchPattern::Grass => 9,
            },
            hatch.angle_rad,
            hatch.spacing_mm,
            hatch.line_width_mm,
        )
        .map_err(|e| err(GeometryErrorKind::Native, e))?;
    }
    let output = ffi::drawing_run(job.pin_mut()).map_err(|e| err(GeometryErrorKind::Native, e))?;
    output
        .into_iter()
        .map(|c| {
            if c.source_index as usize >= solids.len()
                || c.preview.len() < 4
                || !c.preview.len().is_multiple_of(2)
                || c.data
                    .iter()
                    .chain(&c.preview)
                    .chain(&c.knots)
                    .any(|v| !v.is_finite())
            {
                return Err(err(
                    GeometryErrorKind::InvalidResult,
                    "Invalid native vector record",
                ));
            }
            let data = &c.data;
            let curve = match c.kind {
                3 if c.role == 3 && data.len() == 6 && c.preview.as_slice() == data => {
                    VectorCurve::HatchTriangle {
                        points: std::array::from_fn(|i| [data[i * 2], data[i * 2 + 1]]),
                    }
                }
                0 if data.len() == 4 => VectorCurve::Line {
                    a: [data[0], data[1]],
                    b: [data[2], data[3]],
                },
                1 if data.len() == 10 && data[6] > 0. && data[7] > 0. && data[9] > data[8] => {
                    VectorCurve::Ellipse {
                        center: [data[0], data[1]],
                        u: [data[2], data[3]],
                        v: [data[4], data[5]],
                        major_mm: data[6],
                        minor_mm: data[7],
                        start_rad: data[8],
                        end_rad: data[9],
                    }
                }
                2 if data.len() >= 8
                    && (data.len() - 2).is_multiple_of(3)
                    && c.degree > 0
                    && c.knots.len() >= 2
                    && c.knots.len() == c.multiplicities.len()
                    && data[1] > data[0] =>
                {
                    let poles: Vec<_> = data[2..]
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|p| [p[0], p[1]])
                        .collect();
                    let weights: Vec<_> =
                        data[2..].as_chunks::<3>().0.iter().map(|p| p[2]).collect();
                    if weights.iter().any(|w| *w <= 0.) {
                        return Err(err(
                            GeometryErrorKind::InvalidResult,
                            "Invalid rational drawing curve",
                        ));
                    }
                    VectorCurve::BSpline {
                        degree: c.degree,
                        periodic: c.periodic,
                        poles,
                        weights,
                        knots: c.knots,
                        multiplicities: c.multiplicities,
                        first: data[0],
                        last: data[1],
                    }
                }
                _ => {
                    return Err(err(
                        GeometryErrorKind::InvalidResult,
                        "Unsupported or invalid exact drawing curve",
                    ));
                }
            };
            let role = match c.role {
                0 => VectorRole::Visible,
                1 => VectorRole::Hidden,
                2 => VectorRole::Section,
                3 => VectorRole::Hatch,
                _ => {
                    return Err(err(
                        GeometryErrorKind::InvalidResult,
                        "Unknown drawing vector role",
                    ));
                }
            };
            Ok(ProjectedVector {
                source_index: c.source_index as usize,
                role,
                curve,
                preview_mm: c.preview.as_chunks::<2>().0.to_vec(),
            })
        })
        .collect()
}
