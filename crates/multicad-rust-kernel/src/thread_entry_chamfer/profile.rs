//! The unchanged line, plateau and circular root laws in one source period.
use super::{interval::I, *};
#[derive(Clone, Copy)]
pub(super) struct Profile {
    pub radius: I,
    pub first: I,
    pub second: I,
}
impl Chamfer {
    pub(super) fn certify_source_radius(&self) -> Result<(), Error> {
        if self.tool.root_circle().is_some() {
            let knots = self.tool.profile_breaks();
            // The circular valley is convex. Its two trimmed endpoints
            // therefore bound its maximum radius; flank and crest bounds
            // follow from their monotone source coefficients.
            for q in [knots[2], knots[3]] {
                if self.profile(2, I::point(q))?.radius.hi > self.tool.spec().major_radius_mm {
                    return Err(Error::Unresolved);
                }
            }
        }
        Ok(())
    }
    pub(super) fn profile(&self, band: usize, q: I) -> Result<Profile, Error> {
        let spec = self.tool.spec();
        let breaks = self.tool.profile_breaks();
        let zero = I::point(0.);
        let value = if band == 0 || band == 4 {
            Profile {
                radius: I::point(spec.major_radius_mm),
                first: zero,
                second: zero,
            }
        } else if band == 2 {
            if let Some(root) = self.tool.root_circle() {
                let x = q - I::point(root.phase);
                let a = I::point(root.radius);
                let h = (a.square() - x.square()).sqrt();
                Profile {
                    radius: I::point(root.center_radius) - h,
                    first: x / h,
                    second: a.square() / (h.square() * h),
                }
            } else {
                Profile {
                    radius: I::point(self.tool.root_radius_mm()),
                    first: zero,
                    second: zero,
                }
            }
        } else {
            let start = if band == 1 { breaks[1] } else { breaks[4] };
            let slope = I::point(self.tool.slope_at_phase(start, band));
            Profile {
                radius: I::point(spec.major_radius_mm) + slope * (q - I::point(start)),
                first: slope,
                second: zero,
            }
        };
        value.radius.checked()?;
        value.first.checked()?;
        value.second.checked()?;
        Ok(value)
    }
    pub(super) fn rate_interval(&self) -> Result<I, Error> {
        let rate = I::point(self.tool.spec().pitch_mm)
            * I::point(f64::from(self.tool.spec().starts))
            / (I::pi() * I::point(2.));
        (if self.tool.spec().left_hand {
            -rate
        } else {
            rate
        })
        .hull(I::point(self.tool.helical_rate()))
        .checked()
    }
    pub(super) fn cone_slope(&self) -> Result<I, Error> {
        ((I::point(self.tool.spec().major_radius_mm) - I::point(self.entry_radius_mm))
            / I::point(self.entry_length_mm))
        .checked()
    }
}
