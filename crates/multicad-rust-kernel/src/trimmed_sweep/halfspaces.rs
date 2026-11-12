//! Exact implicit cap constraints of the retained affine generator laws.
use super::*;
use crate::Rational;

// Exact binary64 admission for the bounded rational proof below. Decimal
// round-trip text would preserve the float but not its exact binary value.
fn binary_fraction(value: f64) -> Option<Rational> {
    if !value.is_finite() {
        return None;
    }
    if value == 0. {
        return Rational::new(0, 1).ok();
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let mut significand = bits & ((1_u64 << 52) - 1);
    let mut power = if exponent == 0 {
        -1074
    } else {
        significand |= 1_u64 << 52;
        exponent - 1075
    };
    let reduction = significand.trailing_zeros().min((-power).max(0) as u32);
    significand >>= reduction;
    power += reduction as i32;
    if power.unsigned_abs() > 62 {
        return None;
    }
    let mut numerator = i64::try_from(significand).ok()?;
    let denominator = if power >= 0 {
        numerator = numerator.checked_mul(1_i64 << power)?;
        1
    } else {
        1_i64 << -power
    };
    if value.is_sign_negative() {
        numerator = -numerator;
    }
    Rational::new(numerator, denominator).ok()
}

impl TrimmedSweep {
    /// Exact coefficient planes [nx, ny, nz, offset], with material on the
    /// nx*x+ny*y+nz*z <= offset side. These describe actual affine travel,
    /// rather than nominal cap frames whose defining divisions may round.
    /// Returns `None` for placements other than signed axis permutations or
    /// when the bounded exact arithmetic cannot represent the result.
    pub fn exact_cap_halfspaces(
        &self,
        origin: Point,
        u: Point,
        v: Point,
    ) -> Option<[[Rational; 4]; 2]> {
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let axes = [u, v, normal];
        if axes.iter().any(|axis| {
            axis.iter().filter(|&&x| x.abs() == 1.).count() != 1
                || axis.iter().any(|&x| x != 0. && x.abs() != 1.)
        }) {
            return None;
        }
        let [Some(dx), Some(dy), Some(dz)] = self.direction().map(binary_fraction) else {
            return None;
        };
        let zero = Rational::new(0, 1).ok()?;
        let one = Rational::new(1, 1).ok()?;
        let mut planes = [[zero; 4]; 2];
        for (end, law) in self.travel_coefficients().into_iter().enumerate() {
            let [Some(a), Some(b), Some(c)] = law.map(binary_fraction) else {
                return None;
            };
            // Eliminating the generator parameter from p=(x,y,0)+d*(a+bx+cy)
            // gives N=(-dz*b,-dz*c,1+dx*b+dy*c), N.p=dz*a.
            let normal = [
                zero.checked_sub(dz.checked_mul(b).ok()?).ok()?,
                zero.checked_sub(dz.checked_mul(c).ok()?).ok()?,
                one.checked_add(dx.checked_mul(b).ok()?)
                    .ok()?
                    .checked_add(dy.checked_mul(c).ok()?)
                    .ok()?,
            ];
            let sign = if end == 0 { -1 } else { 1 };
            let mut world = [zero; 3];
            for (j, axis) in axes.iter().enumerate() {
                for (i, &component) in axis.iter().enumerate() {
                    if component != 0. {
                        world[i] = normal[j]
                            .checked_mul(Rational::new(sign * component as i64, 1).ok()?)
                            .ok()?;
                    }
                }
            }
            let mut offset = dz
                .checked_mul(a)
                .ok()?
                .checked_mul(Rational::new(sign, 1).ok()?)
                .ok()?;
            for (n, origin) in world.into_iter().zip(origin) {
                offset = offset
                    .checked_add(n.checked_mul(binary_fraction(origin)?).ok()?)
                    .ok()?;
            }
            planes[end] = [world[0], world[1], world[2], offset];
        }
        Some(planes)
    }
}

#[cfg(test)]
mod tests;
