//! Validate every emitted shell against its expected member in file order.
use super::*;
use crate::{bevel_gear::Bevel, spur_gear::herringbone::Herringbone};

/// Expected slots follow the writer's flattened solid order. An analytic
/// slot is independently decoded for subsequent source-topology comparison.
#[derive(Clone, Copy, Debug)]
pub enum ExpectedStepMember<'a> {
    Analytic,
    TwistedGear {
        gear: &'a Herringbone,
        origin: Point,
        u: Point,
        v: Point,
    },
    Bevel {
        gear: &'a Bevel,
        origin: Point,
        u: Point,
        v: Point,
    },
}

/// Independently decoded geometry or an integrated authored-surface proof.
#[derive(Clone, Debug, PartialEq)]
pub enum VerifiedStepMember {
    Analytic {
        readback: Box<Readback>,
        imported: Option<Box<ImportSolid>>,
    },
    TwistedGear {
        volume_mm3: f64,
    },
    Bevel {
        volume_mm3: f64,
    },
}

/// Parse one flat STEP representation and validate each expected shell.
/// The member count must match and each slot uses its expected validator.
/// Each shell keeps isolated geometry caches, while exclusive topology
/// ownership is checked globally. Callers compare analytic readbacks with
/// their source topology before accepting the compound.
pub fn verify_authored_compound(
    input: &str,
    expected: &[ExpectedStepMember<'_>],
    tolerance: f64,
) -> Result<Vec<VerifiedStepMember>> {
    let mut expected = expected.iter();
    let members =
        compound::read_members_by(input, tolerance, true, false, |shell, solid, warped| {
            match expected.next().ok_or(ReadError::Invalid)? {
                ExpectedStepMember::Analytic => {
                    let (readback, imported) =
                        compound::read_analytic_member(shell, solid, warped)?;
                    Ok(VerifiedStepMember::Analytic {
                        readback: Box::new(readback),
                        imported: imported.map(Box::new),
                    })
                }
                ExpectedStepMember::TwistedGear { gear, origin, u, v } => {
                    let volume_mm3 = herringbone::verify_member(
                        shell,
                        solid,
                        gear,
                        [*origin, *u, *v],
                        tolerance,
                    )?;
                    Ok(VerifiedStepMember::TwistedGear { volume_mm3 })
                }
                ExpectedStepMember::Bevel { gear, origin, u, v } => {
                    let volume_mm3 =
                        bevel::verify_member(shell, solid, gear, [*origin, *u, *v], tolerance)?;
                    Ok(VerifiedStepMember::Bevel { volume_mm3 })
                }
            }
        })?;
    if expected.next().is_some() {
        return Err(ReadError::Invalid);
    }
    Ok(members)
}

#[cfg(test)]
mod tests;
