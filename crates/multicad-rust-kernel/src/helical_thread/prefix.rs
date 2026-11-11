//! Exact restriction of a validated untapered tool to an axial prefix.
use super::*;

/// The original tool and its separately retained material interval.
/// Pitch, handedness, phase breaks, flank slopes and root-circle data are
/// copied exactly; only the material's finite axial domain changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxialPrefix {
    tool: Thread,
    material: Thread,
}
impl AxialPrefix {
    pub fn tool(&self) -> &Thread {
        &self.tool
    }
    pub fn material(&self) -> &Thread {
        &self.material
    }
}
impl Thread {
    /// Intersect this tool with `0 <= z <= length_mm`. The validated tool
    /// remains unchanged. The existing minimum axial-domain budget is kept.
    pub fn axial_prefix(self, length_mm: f64) -> Result<AxialPrefix, ThreadError> {
        let prefix = self.positive_axial_prefix(length_mm)?;
        if length_mm < self.spec.pitch_mm {
            return Err(ThreadError::Limit);
        }
        Ok(prefix)
    }

    /// Retain an exact positive material trim after a feature has independently
    /// certified its source interval. The public tool-prefix budget is unchanged.
    pub(crate) fn positive_axial_prefix(self, length_mm: f64) -> Result<AxialPrefix, ThreadError> {
        if !length_mm.is_finite() || length_mm <= 0. || length_mm > self.spec.length_mm {
            return Err(ThreadError::Invalid);
        }
        let material = Self {
            spec: Spec {
                length_mm,
                ..self.spec
            },
            ..self
        };
        Ok(AxialPrefix {
            tool: self,
            material,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_prefix_keeps_the_original_tool_and_all_radial_root_and_phase_data() {
        for starts in [1, 2, 8] {
            for left_hand in [false, true] {
                for root_radius_mm in [None, Some(1. / (4. * 3_f64.sqrt()))] {
                    let tool = Thread::new(Spec {
                        major_radius_mm: 3.,
                        pitch_mm: 1.,
                        length_mm: 5.5,
                        depth_mm: root_radius_mm
                            .map_or(5. * 3_f64.sqrt() / 16., |r| 0.875 * 3_f64.sqrt() / 2. - r),
                        crest_width_mm: 0.125,
                        lower_flank_deg: 30.,
                        upper_flank_deg: 30.,
                        starts,
                        left_hand,
                        root_radius_mm,
                    })
                    .unwrap();
                    for length in [1., 4., 5.5] {
                        let prefix = tool.axial_prefix(length).unwrap();
                        assert_eq!(*prefix.tool(), tool);
                        let material = prefix.material();
                        assert_eq!(material.breaks, tool.breaks);
                        assert_eq!(material.flanks, tool.flanks);
                        assert_eq!(material.root, tool.root);
                        assert_eq!(material.helical_rate(), tool.helical_rate());
                        assert_eq!(
                            *material,
                            Thread::new(Spec {
                                length_mm: length,
                                ..tool.spec()
                            })
                            .unwrap()
                        );
                        for angle in [-6., -1., 0., 0.3, 5., 7.] {
                            for z in [0., length / 3., length] {
                                assert_eq!(material.radius(angle, z), tool.radius(angle, z));
                            }
                        }
                        assert!(
                            (material.volume_mm3()
                                - tool.volume_mm3() * length / tool.spec().length_mm)
                                .abs()
                                < 1e-12
                        );
                    }
                    for length in [f64::NAN, -1., 0., 6.] {
                        assert_eq!(tool.axial_prefix(length), Err(ThreadError::Invalid));
                    }
                    assert_eq!(tool.axial_prefix(0.5), Err(ThreadError::Limit));
                }
            }
        }
    }
}
