//! Native developed sheet geometry for Model's read-only flat-pattern view.
use multicad_core::{
    document::{Document, EntityId},
    model,
};
use multicad_model::{Compiler, PairCycleProofStore, Runtime, with_pair_cycle_proofs};
use multicad_ui::sheet_pattern::{self, Bend, Chart, Pattern};

type PatternResult = Result<Pattern, String>;

pub(crate) struct Backend {
    proofs: PairCycleProofStore,
    runtime: Option<Runtime<PatternResult>>,
    expected: Option<u64>,
}

impl Backend {
    pub(crate) fn new(proofs: PairCycleProofStore) -> Self {
        Self {
            proofs,
            runtime: None,
            expected: None,
        }
    }

    fn runtime(&mut self) -> Result<&mut Runtime<PatternResult>, String> {
        if self.runtime.is_none() {
            self.runtime = Some(with_pair_cycle_proofs(self.proofs.clone(), || {
                Runtime::with_cached_model_publisher(
                    Compiler {
                        omit_display: true,
                        ..Default::default()
                    },
                    |_, _, _| None,
                    |document, evaluated, tag| {
                        // The tag is the selected Part's ordinal in the exact
                        // immutable Document captured by this worker request.
                        // Keep the full model so downstream consumers are
                        // checked and further selections can reuse the compile.
                        let result = document
                            .entities()
                            .nth(tag as usize)
                            .ok_or_else(|| "The selected sheet Part is missing".to_owned())
                            .and_then(|part| {
                                evaluated
                                    .unfold(document, part.id)
                                    .map(to_pattern)
                                    .map_err(|error| error.message)
                            });
                        Ok(result)
                    },
                )
            })?);
        }
        Ok(self
            .runtime
            .as_mut()
            .expect("native sheet runtime initialized above"))
    }
}

impl sheet_pattern::Backend for Backend {
    fn request(
        &mut self,
        document: &Document,
        revision: u64,
        part: EntityId,
    ) -> Result<(), String> {
        if model::sheet_feature(document, part).is_none() {
            self.cancel();
            return Err("Choose a sheet-metal Part to view its flat pattern".into());
        }
        let tag = document
            .entities()
            .position(|entity| entity.id == part)
            .ok_or("The selected sheet Part is missing")?;
        self.expected = None;
        let generation = self
            .runtime()?
            .request_tagged(document, revision, tag as u64)?;
        self.expected = Some(generation);
        Ok(())
    }

    fn poll(&mut self) -> Option<PatternResult> {
        let expected = self.expected?;
        let reply = self.runtime.as_mut()?.poll()?;
        if reply.generation != expected {
            return None;
        }
        self.expected = None;
        Some(
            reply
                .result
                .map_err(|error| error.message)
                .and_then(|result| result),
        )
    }

    fn cancel(&mut self) {
        self.expected = None;
        if let Some(runtime) = &mut self.runtime {
            runtime.cancel();
        }
    }
}

fn to_pattern(pattern: multicad_model::DevelopedPattern) -> Pattern {
    Pattern {
        name: pattern.name,
        cut: pattern.cut,
        size_mm: pattern.size_mm,
        thickness_mm: pattern.thickness_mm,
        bends: pattern
            .bends
            .into_iter()
            .map(|bend| Bend {
                order: bend.order,
                line: bend.record.line,
                direction: bend.record.direction,
                angle_deg: bend.record.angle_deg,
                inside_radius_mm: bend.record.inside_radius_mm,
                allowance_mm: bend.record.allowance_mm,
            })
            .collect(),
        chart: Chart {
            base: pattern.chart.base,
            legs: pattern
                .chart
                .legs
                .into_iter()
                .map(|leg| (leg.index, leg.quad))
                .collect(),
            bend_zones: pattern.chart.bend_zones,
        },
        drills: pattern
            .drills
            .into_iter()
            .map(|drill| drill.center)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::project_file::{self, ContentHash};
    use multicad_ui::sheet_pattern::Backend as _;
    use std::time::{Duration, Instant};

    fn await_pattern(backend: &mut Backend) -> Pattern {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(reply) = backend.poll() {
                return reply.expect("exact flat-pattern reply");
            }
            assert!(Instant::now() < deadline, "flat-pattern worker timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn shipped_front_panel_flat_pattern_uses_exact_native_geometry() {
        let document = project_file::read(
            &mut include_bytes!("../../../examples/front-panel.tcd").as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document;
        let hash = ContentHash::document(&document).unwrap();
        let entity = |name| {
            document
                .entities()
                .find(|entity| entity.name == name)
                .unwrap()
                .id
        };
        let part = entity("Front panel sheet");
        let mut backend = Backend::new(Default::default());

        backend.request(&document, 0, part).unwrap();
        backend.cancel();
        assert!(backend.poll().is_none());
        assert!(
            backend
                .request(&document, 0, entity("Left end part"))
                .is_err()
        );
        backend.request(&document, 0, part).unwrap();
        let pattern = await_pattern(&mut backend);
        assert_eq!(pattern.name, "Front panel sheet");
        assert!((pattern.size_mm[0] - 260.).abs() < 1e-6);
        assert!(
            (pattern.size_mm[1] - (160. + std::f64::consts::PI * (1.5 + 0.44 * 1.5))).abs() < 1e-6
        );
        assert_eq!(pattern.thickness_mm, 1.5);
        assert_eq!(
            pattern.cut.loops.iter().filter(|wire| wire.outer).count(),
            1
        );
        assert_eq!(
            pattern.cut.loops.iter().filter(|wire| !wire.outer).count(),
            15
        );
        assert_eq!(pattern.drills.len(), 14);
        assert_eq!(pattern.bends.len(), 2);
        for bend in &pattern.bends {
            assert_eq!(bend.direction, "down");
            assert!((bend.angle_deg.abs() - 90.).abs() < 1e-9);
            assert!((bend.inside_radius_mm - 1.5).abs() < 1e-9);
            assert!((bend.line[0][0] - bend.line[1][0]).abs() > 250.);
        }
        assert_eq!(
            pattern
                .bends
                .iter()
                .map(|bend| bend.order)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );

        let exported = multicad_model::export::Exporter::default()
            .developed_sheet_dxf(
                &document,
                0,
                part,
                multicad_core::exchange::LengthUnit::Millimeter,
                &Default::default(),
                None,
            )
            .unwrap();
        let expected = to_pattern(exported.pattern);
        assert_eq!(pattern.cut, expected.cut);
        assert_eq!(pattern.drills, expected.drills);
        for (bend, expected) in pattern.bends.iter().zip(expected.bends) {
            assert_eq!(bend.line, expected.line);
        }

        backend.request(&document, 0, part).unwrap();
        let repeated = await_pattern(&mut backend);
        assert_eq!(repeated.cut, pattern.cut);
        assert!(
            backend
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_none()
        );
        assert_eq!(ContentHash::document(&document).unwrap(), hash);
    }
}
