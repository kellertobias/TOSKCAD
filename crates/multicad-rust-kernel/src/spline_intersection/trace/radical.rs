use super::*;

type Result<T> = std::result::Result<T, crate::Error>;

/// One rational plus or minus the square root of a nonnegative rational.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Root {
    pub mean: Rational,
    pub radicand: Rational,
    pub sign: Ordering,
}

impl Root {
    pub fn rational(value: Rational) -> Self {
        Self {
            mean: value,
            radicand: ZERO,
            sign: Ordering::Equal,
        }
    }

    pub fn shifted(self, offset: Rational) -> Result<Self> {
        Ok(Self {
            mean: self.mean.checked_add(offset)?,
            ..self
        })
    }

    pub fn scaled(self, factor: Rational) -> Result<Self> {
        Ok(Self {
            mean: self.mean.checked_mul(factor)?,
            radicand: self.radicand.checked_mul(square(factor)?)?,
            sign: if factor < ZERO {
                self.sign.reverse()
            } else {
                self.sign
            },
        })
    }

    fn radical_sign(self) -> Ordering {
        if self.radicand == ZERO {
            Ordering::Equal
        } else {
            self.sign
        }
    }

    pub fn value_sign(self) -> Result<Ordering> {
        let a = self.mean.cmp(&ZERO);
        let b = self.radical_sign();
        if a == Ordering::Equal || a == b {
            return Ok(b);
        }
        if b == Ordering::Equal {
            return Ok(a);
        }
        let comparison = square(self.mean)?.cmp(&self.radicand);
        Ok(if a == Ordering::Greater {
            comparison
        } else {
            comparison.reverse()
        })
    }

    pub fn compare(self, other: Self) -> Result<Ordering> {
        let x = self.shifted(ZERO.checked_sub(other.mean)?)?;
        let a = x.value_sign()?;
        let b = other.radical_sign();
        if b == Ordering::Equal {
            return Ok(a);
        }
        if a != b {
            return Ok(b.reverse());
        }
        // Both terms have the same sign. Compare their squares. Squaring
        // x still produces a single rational plus a single radical.
        let offset = square(x.mean)?
            .checked_add(x.radicand)?
            .checked_sub(other.radicand)?;
        let mixed = Self { mean: ZERO, ..x }.scaled(TWO.checked_mul(x.mean)?)?;
        let comparison = mixed.shifted(offset)?.value_sign()?;
        Ok(if a == Ordering::Greater {
            comparison
        } else {
            comparison.reverse()
        })
    }

    pub fn approximation(self) -> Result<f64> {
        let mean = self.mean.as_f64();
        let radical = self.radicand.as_f64().sqrt()
            * if self.radical_sign() == Ordering::Less {
                -1.
            } else {
                1.
            };
        if mean * radical < 0. {
            // Rationalize a nearly cancelling subtraction before converting.
            let difference = square(self.mean)?.checked_sub(self.radicand)?.as_f64();
            Ok(difference / (mean - radical))
        } else {
            Ok(mean + radical)
        }
    }
}
