//! Bounded axial-straight worm intent and independently generated wheel geometry.
use super::*;
pub const WORM_PROFILE_SOURCE: &str = "Explicit bounded ZA axial-straight screw and Houser/Su 1999 generated wheel envelope; q10, one right-hand start, normal pressure20 degrees; no ZI/ZN/ZK, accuracy grade, load or physical-fit certification";
pub const WORM_GEAR_CAPABILITY: &str = "model.gear.worm.1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WormMember {
    Worm,
    Wheel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WormToothSystem {
    ZaAxialStraight,
    Zi,
    Zn,
    Zk,
}
impl WormToothSystem {
    pub const ALL: [Self; 4] = [Self::ZaAxialStraight, Self::Zi, Self::Zn, Self::Zk];
    // The family registry separately owns availability of the bounded system.
    pub fn supported(self) -> bool {
        self == Self::ZaAxialStraight
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::ZaAxialStraight => "ZA axial-straight",
            Self::Zi => "ZI (unavailable)",
            Self::Zn => "ZN (unavailable)",
            Self::Zk => "ZK (unavailable)",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WormDimensions {
    pub role: WormMember,
    pub tooth_system: WormToothSystem,
    pub mate_teeth: Dimension,
    pub diameter_quotient: Dimension,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWormGeometry {
    pub role: WormMember,
    pub tooth_system: WormToothSystem,
    /// OWN count: 1 for worm; wheel tooth count for wheel.
    pub teeth: u32,
    pub starts: u32,
    pub wheel_teeth: u32,
    pub axial_module_mm: f64,
    pub diameter_quotient: f64,
    pub normal_pressure_angle_rad: f64,
    pub axial_pressure_angle_rad: f64,
    pub lead_angle_rad: f64,
    /// Axial worm spacing / circular wheel pitch. Both equal pi*axial module.
    pub pitch_mm: f64,
    pub lead_mm: f64,
    pub pitch_radius_mm: f64,
    pub tip_radius_mm: f64,
    /// Minimum generated wheel torus radius (middle plane), or worm root.
    pub minimum_root_radius_mm: f64,
    pub bore_radius_mm: f64,
    /// Threaded worm length or axial wheel face width; both centered on Z=0.
    pub face_width_mm: f64,
    pub nominal_tooth_thickness_mm: f64,
    pub design_tooth_thickness_mm: f64,
    pub physical_tooth_thickness_mm: f64,
    pub thinning_mm: f64,
    pub hob_pitch_space_thickness_mm: Option<f64>,
    pub minimum_wall_mm: f64,
    pub frame: Frame,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedWormPair {
    pub worm_member: GearPairMember,
    pub wheel_member: GearPairMember,
    pub wheel_teeth: u32,
    pub center_distance_mm: f64,
    pub shaft_angle_rad: f64,
    /// Drive always belongs to the worm, independent of array order.
    pub worm_period_rad: f64,
}
impl GearParameters {
    pub(super) fn resolve_worm(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedWormGeometry, DocumentError> {
        self.validate()?;
        let intent = self.worm.as_ref().expect("validated worm dimensions");
        if !intent.tooth_system.supported() {
            return Err(invalid(
                "Only the bounded ZA axial-straight system is implemented; ZI, ZN and ZK are unavailable",
            ));
        }
        let m = number(&self.module, Unit::Millimeter, parameters)?;
        let own = number(&self.teeth, Unit::Count, parameters)?;
        let mate = number(&intent.mate_teeth, Unit::Count, parameters)?;
        let q = number(&intent.diameter_quotient, Unit::Ratio, parameters)?;
        let alpha = number(&self.pressure_angle, Unit::Radian, parameters)?;
        let width = number(&self.face_width, Unit::Millimeter, parameters)?;
        let bore = number(&self.bore_diameter, Unit::Millimeter, parameters)?;
        let wall = number(&self.minimum_wall, Unit::Millimeter, parameters)?;
        let backlash = number(&self.backlash, Unit::Millimeter, parameters)?;
        let allowance = number(&self.fabrication_allowance, Unit::Millimeter, parameters)?;
        let thinning = backlash + allowance;
        if !(0.5..=2.).contains(&m)
            || own.fract().abs() > 1e-9
            || mate.fract().abs() > 1e-9
            || (q - 10.).abs() > 1e-12
            || (alpha - 20_f64.to_radians()).abs() > 1e-12
            || bore < 0.
            || wall <= 0.
            || backlash < 0.
            || allowance < 0.
            || !(0. ..=0.1 * m).contains(&thinning)
        {
            return Err(invalid(
                "ZA gearing requires axial module 0.5–2 mm, whole member counts, diameter quotient10, NORMAL pressure20 degrees, positive wall and nonnegative total thinning no greater than0.1 module",
            ));
        }
        if self.module_system != GearModuleSystem::Axial
            || self.hand != GearHand::Right
            || number(&self.helix_angle, Unit::Radian, parameters)? != 0.
            || number(&self.center_gap, Unit::Millimeter, parameters)? != 0.
            || number(&self.profile_shift, Unit::Ratio, parameters)? != 0.
            || number(&self.root_fillet_ratio, Unit::Ratio, parameters)? != 0.
            || self.outer_diameter.is_some()
            || self.rack.is_some()
            || self.bevel.is_some()
        {
            return Err(invalid(
                "ZA gearing requires axial module, right hand, zero authored helix/gap/shift/fillet and no unrelated rack/bevel/outer dimensions; lead angle is derived",
            ));
        }
        let (wheel_teeth, pitch_radius, root) = match intent.role {
            WormMember::Worm => {
                if own != 1.
                    || !(32. ..=64.).contains(&mate)
                    || !(16. * m..=32. * m).contains(&width)
                {
                    return Err(invalid(
                        "Worm needs one start,32–64 mate teeth and threaded length16–32 axial modules",
                    ));
                }
                (mate as u32, 5. * m, 3.75 * m)
            }
            WormMember::Wheel => {
                if mate != 1.
                    || !(32. ..=64.).contains(&own)
                    || !(0.5 * m..=2. * m).contains(&width)
                {
                    return Err(invalid(
                        "Wheel needs32–64 teeth,one mate start and face width0.5–2 axial modules",
                    ));
                }
                (own as u32, own * m / 2., (own / 2. - 1.25) * m)
            }
        };
        if root - bore / 2. < wall {
            return Err(invalid(
                "Bore consumes the minimum worm root or wheel middle-plane torus wall",
            ));
        }
        let nominal = PI * m / 2.;
        Ok(ResolvedWormGeometry {
            role: intent.role,
            tooth_system: intent.tooth_system,
            teeth: own as u32,
            starts: 1,
            wheel_teeth,
            axial_module_mm: m,
            diameter_quotient: q,
            normal_pressure_angle_rad: alpha,
            axial_pressure_angle_rad: (alpha.tan() * 1.01_f64.sqrt()).atan(),
            lead_angle_rad: 0.1_f64.atan(),
            pitch_mm: PI * m,
            lead_mm: PI * m,
            pitch_radius_mm: pitch_radius,
            tip_radius_mm: pitch_radius + m,
            minimum_root_radius_mm: root,
            bore_radius_mm: bore / 2.,
            face_width_mm: width,
            nominal_tooth_thickness_mm: nominal,
            design_tooth_thickness_mm: nominal - backlash,
            physical_tooth_thickness_mm: nominal - thinning,
            thinning_mm: thinning,
            hob_pitch_space_thickness_mm: if intent.role == WormMember::Wheel {
                Some(nominal + thinning)
            } else {
                None
            },
            minimum_wall_mm: wall,
            frame: self.frame.resolve(parameters)?,
        })
    }
}
/// Explicit caller reconciliation precedes this normalization. Original typed
/// count and quotient sources are cloned, never replaced by evaluated literals.
pub fn couple_worm_pair_recipes(
    recipes: &[GearParameters; 2],
) -> Result<[GearParameters; 2], DocumentError> {
    let roles = recipes
        .iter()
        .map(|r| r.worm.as_ref().map(|w| w.role))
        .collect::<Vec<_>>();
    if recipes.iter().any(|r| r.family != GearFamily::Worm)
        || !matches!(
            roles.as_slice(),
            [Some(WormMember::Worm), Some(WormMember::Wheel)]
                | [Some(WormMember::Wheel), Some(WormMember::Worm)]
        )
    {
        return Err(invalid(
            "Worm coupling needs exactly one declared worm and one declared wheel",
        ));
    }
    let wi = if roles[0] == Some(WormMember::Worm) {
        0
    } else {
        1
    };
    let quotient = recipes[wi]
        .worm
        .as_ref()
        .expect("checked")
        .diameter_quotient
        .clone();
    let mut result = recipes.clone();
    for i in 0..2 {
        let w = result[i].worm.as_mut().expect("checked");
        w.mate_teeth = recipes[1 - i].teeth.clone();
        w.diameter_quotient = quotient.clone();
    }
    Ok(result)
}
pub(super) fn validate_worm_pair_sources(
    recipes: [&GearParameters; 2],
) -> Result<(), DocumentError> {
    if !recipes.iter().any(|r| r.family == GearFamily::Worm) {
        return Ok(());
    }
    let (Some(a), Some(b)) = (&recipes[0].worm, &recipes[1].worm) else {
        return Err(invalid(
            "A worm pair needs both retained generating recipes",
        ));
    };
    if recipes.iter().any(|r| r.family != GearFamily::Worm)
        || a.role == b.role
        || a.tooth_system != b.tooth_system
        || a.mate_teeth != recipes[1].teeth
        || b.mate_teeth != recipes[0].teeth
        || a.diameter_quotient != b.diameter_quotient
    {
        return Err(invalid(
            "Worm pair needs opposite roles,matching tooth system,opposite original typed counts and the worm's shared typed diameter quotient; explicit conflicts are refused",
        ));
    }
    Ok(())
}
impl GearPairParameters {
    pub(super) fn resolve_worm_pair(
        &self,
        gears: [ResolvedGearGeometry; 2],
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedGearPair, DocumentError> {
        let a = gears[0]
            .worm()
            .ok_or_else(|| invalid("Worm pair requires a worm and its generated wheel"))?;
        let b = gears[1]
            .worm()
            .ok_or_else(|| invalid("Worm pair requires a worm and its generated wheel"))?;
        let close = |x: f64, y: f64| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.);
        if a.role == b.role
            || !close(a.axial_module_mm, b.axial_module_mm)
            || !close(a.normal_pressure_angle_rad, b.normal_pressure_angle_rad)
            || a.wheel_teeth != b.wheel_teeth
        {
            return Err(invalid(
                "Worm pair needs opposite roles with matching axial module,normal pressure and generating wheel count",
            ));
        }
        let wi = if a.role == WormMember::Worm { 0 } else { 1 };
        let gi = 1 - wi;
        let drive = number(&self.drive_angle, Unit::Radian, evaluated)?;
        if drive.abs() > 2000. * PI {
            return Err(invalid(
                "Worm pair drive exceeds the bounded1000-turn coordinate",
            ));
        }
        let teeth = a.wheel_teeth;
        let distance = (10. + f64::from(teeth)) * a.axial_module_mm / 2.;
        let mut phases = [0.; 2];
        phases[wi] = drive;
        phases[gi] = drive / f64::from(teeth);
        let base = self.frame.resolve(evaluated)?;
        let mating = std::array::from_fn(|i| {
            let (sin, cos) = phases[i].rem_euclid(2. * PI).sin_cos();
            let wheel = i == gi;
            crate::references::ReferenceFrame {
                origin_mm: std::array::from_fn(|j| {
                    base.origin[j] - if wheel { distance * base.u[j] } else { 0. }
                }),
                u: std::array::from_fn(|j| {
                    base.u[j] * cos
                        + if wheel {
                            -base.n[j] * sin
                        } else {
                            base.v[j] * sin
                        }
                }),
                v: std::array::from_fn(|j| {
                    -base.u[j] * sin
                        + if wheel {
                            -base.n[j] * cos
                        } else {
                            base.v[j] * cos
                        }
                }),
            }
        });
        let context = ResolvedWormPair {
            worm_member: if wi == 0 {
                GearPairMember::A
            } else {
                GearPairMember::B
            },
            wheel_member: if gi == 0 {
                GearPairMember::A
            } else {
                GearPairMember::B
            },
            wheel_teeth: teeth,
            center_distance_mm: distance,
            shaft_angle_rad: PI / 2.,
            worm_period_rad: 2. * PI,
        };
        Ok(ResolvedGearPair {
            gears,
            rack_travel: None,
            bevel: None,
            worm: Some(context),
            center_distance_mm: distance,
            phases_rad: phases,
            mating,
        })
    }
}
