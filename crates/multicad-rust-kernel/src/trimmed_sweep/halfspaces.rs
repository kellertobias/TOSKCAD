//! Exact implicit cap constraints of the retained affine generator laws.
use super::*;
use crate::Rational;

use crate::exact_binary::binary_fraction;

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
