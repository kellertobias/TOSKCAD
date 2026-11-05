//! Declared spherical-involute straight bevel intent; custom spherical roots.
use super::*;
pub const BEVEL_GEAR_CAPABILITY: &str = "model.gear.bevel.1";
pub const BEVEL_PROFILE_SOURCE_URL: &str = "https://ijme.us/cd_11/PDF/Paper%20163%20ENG%20107.pdf";
pub const BEVEL_DIMENSIONS_SOURCE_URL: &str = "https://www.kggear.co.jp/en/wp-content/themes/bizvektor-global-edition/pdf/9.7_Calculation-for-Standard-straight-bevel-gear_TechnicalData_KGSTOCKGEARS.pdf";
pub const BEVEL_PROFILE_SOURCE: &str = "Ligata/Zhang 2011 spherical-involute straight flanks; KG standard back cones; custom tangent spherical-circle roots. 90-degree shafts, 20-degree pressure, 24–64 teeth per member. No Gleason/octoid, spiral, zerol, generated cutter, strength or fit certification.";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BevelToothSystem {
    SphericalInvoluteStraight,
    GleasonOctoid,
    Spiral,
    Zerol,
}
impl BevelToothSystem {
    pub const ALL: [Self; 4] = [
        Self::SphericalInvoluteStraight,
        Self::GleasonOctoid,
        Self::Spiral,
        Self::Zerol,
    ];
    pub fn supported(self) -> bool {
        self == Self::SphericalInvoluteStraight
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::SphericalInvoluteStraight => "Spherical-involute straight",
            Self::GleasonOctoid => "Gleason / octoid (unavailable)",
            Self::Spiral => "Spiral (unavailable)",
            Self::Zerol => "Zerol (unavailable)",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BevelDimensions {
    pub tooth_system: BevelToothSystem,
    pub mate_teeth: Dimension,
    pub shaft_angle: Dimension,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BevelRootBranch {
    Meridian,
    Involute,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBevel {
    pub tooth_system: BevelToothSystem,
    pub teeth: u32,
    pub mate_teeth: u32,
    pub module_mm: f64,
    pub pressure_angle_rad: f64,
    pub shaft_angle_rad: f64,
    pub pitch_cone_angle_rad: f64,
    pub base_cone_angle_rad: f64,
    pub tip_cone_angle_rad: f64,
    pub root_cone_angle_rad: f64,
    pub outer_cone_distance_mm: f64,
    pub inner_cone_distance_mm: f64,
    /// Width measured along the pitch cone generator, not axial extrusion.
    pub face_width_mm: f64,
    pub pitch_radius_mm: f64,
    pub nominal_tooth_thickness_mm: f64,
    pub design_tooth_thickness_mm: f64,
    pub physical_tooth_thickness_mm: f64,
    pub half_base_angle_rad: f64,
    pub bore_radius_mm: f64,
    pub minimum_wall_mm: f64,
    pub toe_root_radius_mm: f64,
    pub heel_root_radius_mm: f64,
    pub heel_tip_radius_mm: f64,
    pub toe_tip_radius_mm: f64,
    pub heel_root_axial_mm: f64,
    pub heel_tip_axial_mm: f64,
    pub toe_root_axial_mm: f64,
    pub toe_tip_axial_mm: f64,
    pub root_fillet_radius_mm: f64,
    pub root_rounding_angle_rad: f64,
    pub root_branch: BevelRootBranch,
    pub flank_start_gamma_rad: f64,
    pub flank_start_sigma_rad: f64,
    /// Unit spherical center axis, before back-cone projection.
    pub root_center_direction: [f64; 3],
    pub flank_tangent_direction: [f64; 3],
    pub root_tangent_direction: [f64; 3],
    pub frame: Frame,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedBevelPair {
    pub shaft_angle_rad: f64,
    pub cone_distance_mm: f64,
    pub face_width_mm: f64,
    pub pitch_cone_angles_rad: [f64; 2],
}
fn spherical(gamma: f64, phi: f64) -> [f64; 3] {
    [
        gamma.sin() * phi.cos(),
        gamma.sin() * phi.sin(),
        gamma.cos(),
    ]
}
fn spherical_sigma(gamma: f64, beta: f64) -> Result<f64, DocumentError> {
    let value = gamma.cos() / beta.cos();
    if !(-1. ..=1.).contains(&value) {
        return Err(invalid("Bevel spherical involute lies below its base cone"));
    }
    Ok(value.acos())
}
fn spherical_inv(gamma: f64, beta: f64) -> Result<f64, DocumentError> {
    let sigma = spherical_sigma(gamma, beta)?;
    Ok(sigma / beta.sin() - sigma.sin().atan2(beta.sin() * sigma.cos()))
}
impl GearParameters {
    pub(super) fn resolve_bevel(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedBevel, DocumentError> {
        self.validate()?;
        let bevel = self.bevel.as_ref().expect("validated bevel dimensions");
        if !bevel.tooth_system.supported() {
            return Err(invalid(format!(
                "{} is unavailable; only declared spherical-involute straight bevels are implemented",
                bevel.tooth_system.label()
            )));
        }
        let m = number(&self.module, Unit::Millimeter, parameters)?;
        let z = number(&self.teeth, Unit::Count, parameters)?;
        let mate = number(&bevel.mate_teeth, Unit::Count, parameters)?;
        let shaft = number(&bevel.shaft_angle, Unit::Radian, parameters)?;
        let alpha = number(&self.pressure_angle, Unit::Radian, parameters)?;
        let width = number(&self.face_width, Unit::Millimeter, parameters)?;
        let bore = number(&self.bore_diameter, Unit::Millimeter, parameters)?;
        let wall = number(&self.minimum_wall, Unit::Millimeter, parameters)?;
        let backlash = number(&self.backlash, Unit::Millimeter, parameters)?;
        let allowance = number(&self.fabrication_allowance, Unit::Millimeter, parameters)?;
        let fillet = number(&self.root_fillet_ratio, Unit::Ratio, parameters)?;
        if m < 0.01
            || !(24. ..=64.).contains(&z)
            || !(24. ..=64.).contains(&mate)
            || z.fract().abs() > 1e-9
            || mate.fract().abs() > 1e-9
            || (shaft - PI / 2.).abs() > 1e-12
            || (alpha - 20_f64.to_radians()).abs() > 1e-12
            || width <= 0.
            || bore < 0.
            || wall <= 0.
            || backlash < 0.
            || allowance < 0.
            || !(0.01..=0.2).contains(&fillet)
        {
            return Err(invalid(
                "Spherical-involute straight bevels require module ≥0.01 mm, 24–64 whole teeth on each member, 90-degree shaft angle, 20-degree pressure angle, positive cone face width/wall, nonnegative bore/thinning and custom fillet ratio 0.01–0.2",
            ));
        }
        if number(&self.helix_angle, Unit::Radian, parameters)? != 0.
            || self.hand != GearHand::None
            || number(&self.center_gap, Unit::Millimeter, parameters)? != 0.
            || number(&self.profile_shift, Unit::Ratio, parameters)? != 0.
        {
            return Err(invalid(
                "Straight bevels require zero helix, hand none, center gap and profile shift; hubs and other tooth systems are unavailable",
            ));
        }
        let delta = shaft.sin().atan2(mate / z + shaft.cos());
        let distance = m * z / (2. * delta.sin());
        if width > distance / 3. || width > 10. * m {
            return Err(invalid(
                "Bevel cone face width must not exceed min(cone distance / 3, 10 * module)",
            ));
        }
        let inner = distance - width;
        let beta = (delta.sin() * alpha.cos()).asin();
        let tip = delta + (m / distance).atan();
        let root = delta - (1.25 * m / distance).atan();
        let radius = m * fillet;
        let rho = (radius / distance).asin();
        if !(0. ..PI / 2.).contains(&root)
            || root == 0.
            || tip >= PI / 2.
            || inner <= 0.
            || rho <= 0.
            || !rho.is_finite()
        {
            return Err(invalid(
                "Bevel pitch, tip, root and inner cones must remain positive and acute",
            ));
        }
        let nominal = PI * m / 2.;
        let design = nominal - backlash;
        let physical = design - allowance;
        let half = physical / (m * z) + spherical_inv(delta, beta)?;
        let tip_half = half - spherical_inv(tip, beta)?;
        let center_gamma = root + rho;
        let branch_threshold = (beta.cos() * rho.cos()).acos();
        let (branch, touch_gamma, touch_sigma, center, touch) = if center_gamma <= branch_threshold
        {
            let gamma = (center_gamma.cos() / rho.cos()).acos();
            if !gamma.is_finite() || gamma > beta + 1e-12 {
                return Err(invalid(
                    "Bevel meridian root transition exceeds the base cone",
                ));
            }
            let phi = half + (rho.sin() / center_gamma.sin()).asin();
            (
                BevelRootBranch::Meridian,
                gamma,
                0.,
                spherical(center_gamma, phi),
                spherical(gamma, half),
            )
        } else {
            let sigma = (center_gamma.cos() / beta.cos()).acos() - rho;
            let gamma = (beta.cos() * sigma.cos()).acos();
            let phi = half - spherical_inv(gamma, beta)?;
            let c = spherical(gamma, phi);
            let dg = beta.cos() * sigma.sin() / gamma.sin();
            let dp = -1. / beta.sin() + beta.sin() / gamma.sin().powi(2);
            let derivative = [
                gamma.cos() * phi.cos() * dg - gamma.sin() * phi.sin() * dp,
                gamma.cos() * phi.sin() * dg + gamma.sin() * phi.cos() * dp,
                -gamma.sin() * dg,
            ];
            let norm = derivative.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm <= 0. || !norm.is_finite() {
                return Err(invalid("Bevel root involute tangent is degenerate"));
            }
            let t = derivative.map(|v| v / norm);
            let n = [
                c[1] * t[2] - c[2] * t[1],
                c[2] * t[0] - c[0] * t[2],
                c[0] * t[1] - c[1] * t[0],
            ];
            let u = std::array::from_fn(|i| rho.cos() * c[i] + rho.sin() * n[i]);
            (BevelRootBranch::Involute, gamma, sigma, u, c)
        };
        let center_phi = center[1].atan2(center[0]);
        let root_tangent = spherical(root, center_phi);
        let toe_root = inner * root.sin() / (root - delta).cos();
        if physical <= 0.
            || tip_half <= 1e-5 / (m * z / 2.)
            || center_phi >= PI / z
            || center_phi <= 0.
            || touch_gamma >= delta
            || touch_gamma >= tip
            || toe_root - bore / 2. < wall
            || center
                .iter()
                .chain(&touch)
                .chain(&root_tangent)
                .any(|v| !v.is_finite())
            || (center[2].acos() - center_gamma).abs() > 1e-10
            || tip_half.sin() * inner * tip.sin() <= 1e-5
        {
            return Err(invalid(
                "Bevel thinning, custom root circle or bore leaves a missing tip/root flat, invalid base transition or insufficient toe wall",
            ));
        }
        Ok(ResolvedBevel {
            tooth_system: bevel.tooth_system,
            teeth: z as u32,
            mate_teeth: mate as u32,
            module_mm: m,
            pressure_angle_rad: alpha,
            shaft_angle_rad: shaft,
            pitch_cone_angle_rad: delta,
            base_cone_angle_rad: beta,
            tip_cone_angle_rad: tip,
            root_cone_angle_rad: root,
            outer_cone_distance_mm: distance,
            inner_cone_distance_mm: inner,
            face_width_mm: width,
            pitch_radius_mm: m * z / 2.,
            nominal_tooth_thickness_mm: nominal,
            design_tooth_thickness_mm: design,
            physical_tooth_thickness_mm: physical,
            half_base_angle_rad: half,
            bore_radius_mm: bore / 2.,
            minimum_wall_mm: wall,
            toe_root_radius_mm: toe_root,
            heel_root_radius_mm: distance * root.sin() / (root - delta).cos(),
            heel_tip_radius_mm: distance * tip.sin() / (tip - delta).cos(),
            toe_tip_radius_mm: inner * tip.sin() / (tip - delta).cos(),
            heel_root_axial_mm: distance * root.cos() / (root - delta).cos(),
            heel_tip_axial_mm: distance * tip.cos() / (tip - delta).cos(),
            toe_root_axial_mm: inner * root.cos() / (root - delta).cos(),
            toe_tip_axial_mm: inner * tip.cos() / (tip - delta).cos(),
            root_fillet_radius_mm: radius,
            root_rounding_angle_rad: rho,
            root_branch: branch,
            flank_start_gamma_rad: touch_gamma,
            flank_start_sigma_rad: touch_sigma,
            root_center_direction: center,
            flank_tangent_direction: touch,
            root_tangent_direction: root_tangent,
            frame: self.frame.resolve(parameters)?,
        })
    }
}
/// Explicitly derive a pair's mate counts from its primary typed count sources.
/// Callers must reconcile explicit/omitted user intent before requesting this
/// coupling. Preparation itself never silently normalizes contradictory source.
pub fn couple_bevel_pair_recipes(
    recipes: &[GearParameters; 2],
    shaft_angle: &Dimension,
) -> Result<[GearParameters; 2], DocumentError> {
    check_dimension(shaft_angle, Unit::Degree)?;
    if recipes
        .iter()
        .any(|p| p.family != GearFamily::Bevel || p.bevel.is_none())
    {
        return Err(invalid(
            "Bevel coupling requires two complete bevel recipes",
        ));
    }
    let mut result = recipes.clone();
    for i in 0..2 {
        let bevel = result[i].bevel.as_mut().expect("checked bevel");
        bevel.mate_teeth = recipes[1 - i].teeth.clone();
        bevel.shaft_angle = shaft_angle.clone();
    }
    Ok(result)
}
pub(super) fn validate_bevel_pair_sources(
    recipes: [&GearParameters; 2],
) -> Result<(), DocumentError> {
    if !recipes.iter().any(|r| r.family == GearFamily::Bevel) {
        return Ok(());
    }
    let (Some(a), Some(b)) = (&recipes[0].bevel, &recipes[1].bevel) else {
        return Err(invalid("A bevel pair needs two declared bevel recipes"));
    };
    if recipes.iter().any(|r| r.family != GearFamily::Bevel)
        || a.tooth_system != b.tooth_system
        || a.mate_teeth != recipes[1].teeth
        || b.mate_teeth != recipes[0].teeth
        || a.shaft_angle != b.shaft_angle
    {
        return Err(invalid(
            "Bevel pair mate counts must share the opposite original typed tooth source and one shared typed shaft angle; explicit conflicting sources are refused",
        ));
    }
    Ok(())
}
impl GearPairParameters {
    pub(super) fn resolve_bevel_pair(
        &self,
        gears: [ResolvedGearGeometry; 2],
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedGearPair, DocumentError> {
        let a = gears[0]
            .bevel()
            .ok_or_else(|| invalid("Bevel pairs require two spherical-involute bevel members"))?;
        let b = gears[1]
            .bevel()
            .ok_or_else(|| invalid("Bevel pairs require two spherical-involute bevel members"))?;
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.);
        if !close(a.module_mm, b.module_mm)
            || !close(a.pressure_angle_rad, b.pressure_angle_rad)
            || !close(a.face_width_mm, b.face_width_mm)
            || !close(a.outer_cone_distance_mm, b.outer_cone_distance_mm)
            || !close(
                a.pitch_cone_angle_rad + b.pitch_cone_angle_rad,
                a.shaft_angle_rad,
            )
        {
            return Err(invalid(
                "Bevel pairs require matching module, pressure, cone face width and cone distance, with complementary pitch cones",
            ));
        }
        let drive = number(&self.drive_angle, Unit::Radian, evaluated)?;
        if drive.abs() > 2000. * PI {
            return Err(invalid(
                "Bevel pair drive exceeds the bounded 1000-turn coordinate",
            ));
        }
        let phases = [
            drive,
            PI - PI / f64::from(b.teeth) - drive * f64::from(a.teeth) / f64::from(b.teeth),
        ];
        let base = self.frame.resolve(evaluated)?;
        let shaft = a.shaft_angle_rad;
        let mating = std::array::from_fn(|i| {
            let u = if i == 0 {
                base.u
            } else {
                std::array::from_fn(|j| base.u[j] * shaft.cos() - base.n[j] * shaft.sin())
            };
            let (sin, cos) = phases[i].rem_euclid(2. * PI).sin_cos();
            crate::references::ReferenceFrame {
                origin_mm: base.origin,
                u: std::array::from_fn(|j| u[j] * cos + base.v[j] * sin),
                v: std::array::from_fn(|j| -u[j] * sin + base.v[j] * cos),
            }
        });
        let context = ResolvedBevelPair {
            shaft_angle_rad: shaft,
            cone_distance_mm: a.outer_cone_distance_mm,
            face_width_mm: a.face_width_mm,
            pitch_cone_angles_rad: [a.pitch_cone_angle_rad, b.pitch_cone_angle_rad],
        };
        Ok(ResolvedGearPair {
            gears,
            rack_travel: None,
            bevel: Some(context),
            center_distance_mm: 0.,
            phases_rad: phases,
            mating,
        })
    }
}
