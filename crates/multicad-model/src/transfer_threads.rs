//! Refuse loss of known internal-thread intent before producing plain transfer cuts.
//! A mouth can lie outside the threaded axial span (counterbores and lead-ins),
//! so sharing its axis is sufficient to require an explicit thread workflow.
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, ResolvedMechanicalPart, ThreadMode},
    parameters::EvaluatedParameters,
    references::{TopologyKey, TopologySnapshot},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct ThreadSources {
    axes: BTreeMap<EntityId, ([f64; 3], [f64; 3], String)>,
    uncertain: bool,
}
impl ThreadSources {
    pub fn inspect(
        document: &Document,
        parameters: &EvaluatedParameters,
        producer: EntityId,
    ) -> Result<Self, String> {
        let mut result = Self {
            axes: BTreeMap::new(),
            uncertain: false,
        };
        let mut pending = vec![(producer, false)];
        let mut visited = BTreeSet::new();
        while let Some((id, uncertain)) = pending.pop() {
            if !visited.insert((id, uncertain)) {
                continue;
            }
            if visited.len() > 8192 {
                return Err("Transfer source ancestry exceeds the thread-inspection budget".into());
            }
            let entity = document
                .get(id)
                .ok_or("Transfer source ancestry has a missing dependency")?;
            let mut descendants_uncertain = uncertain;
            if let EntityData::Feature {
                definition,
                suppressed: false,
                ..
            } = &entity.data
            {
                let thread = match definition.type_id.as_str() {
                    model::THREAD_TYPE => {
                        let source = model::read_thread(document, id).map_err(|e| e.to_string())?;
                        if source.mode == ThreadMode::CutInternal {
                            Some(
                                source
                                    .parameters
                                    .resolve(parameters, source.mode)
                                    .map_err(|e| e.to_string())?,
                            )
                        } else {
                            None
                        }
                    }
                    model::MECHANICAL_PART_TYPE => {
                        let source =
                            model::read_mechanical_part(document, id).map_err(|e| e.to_string())?;
                        match source
                            .parameters
                            .resolve(parameters)
                            .map_err(|e| e.to_string())?
                        {
                            ResolvedMechanicalPart::HexNut { thread, .. } => Some(*thread),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(thread) = thread {
                    result.axes.insert(
                        id,
                        (thread.frame.origin, thread.frame.n, entity.name.clone()),
                    );
                    result.uncertain |= uncertain;
                }
                // Boolean operands retain their local coordinates. Other downstream
                // providers may move, copy or deform an earlier thread. Until that
                // mapping is proven, refusing this source is safer than declaring a
                // different-looking rim unthreaded. Upstream base geometry does not
                // make an already resolved, downstream thread uncertain.
                descendants_uncertain |= !matches!(
                    definition.type_id.as_str(),
                    model::THREAD_TYPE
                        | model::MECHANICAL_PART_TYPE
                        | model::BOOLEAN_TYPE
                        | model::PRIMITIVE_TYPE
                        | model::EXTRUDE_TYPE
                        | model::EXTRUDE_GROUP_TYPE
                        | model::EDGE_TREATMENT_TYPE
                        | model::TRANSFER_TYPE
                );
            }
            pending.extend(
                document
                    .evaluation_dependencies(id)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|dependency| (dependency, descendants_uncertain)),
            );
        }
        Ok(result)
    }
    pub fn has_threads(&self) -> bool {
        !self.axes.is_empty()
    }
    pub fn diagnostic(&self) -> String {
        let names = self
            .axes
            .values()
            .map(|(_, _, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Threaded-hole transfer is unsupported{}; the receiving cut cannot preserve thread properties. Choose an ordinary noncoaxial hole or create the receiving thread explicitly.",
            if names.is_empty() {
                String::new()
            } else {
                format!(" ({names})")
            }
        )
    }
    pub fn check_provenance(
        &self,
        topology: &TopologySnapshot,
        keys: &[TopologyKey],
    ) -> Result<(), String> {
        // Callers first validate the actual circular axis. Boolean ancestry can
        // attach thread origins to unrelated rims; proven noncoaxial geometry
        // must not become a threaded hole merely because it shares a Part.
        if self.has_threads() && !self.uncertain {
            return Ok(());
        }
        for key in keys {
            let element = topology
                .elements
                .iter()
                .find(|e| e.key == *key)
                .ok_or("Transfer contour is missing from the source topology")?;
            if element.origins.iter().any(|origin| {
                origin.role.starts_with("thread/") || origin.role.starts_with("thread.")
            }) {
                return Err(self.diagnostic());
            }
        }
        Ok(())
    }
    pub fn check_axis(
        &self,
        origin: [f64; 3],
        axis: [f64; 3],
        tolerance: f64,
    ) -> Result<(), String> {
        if self.has_threads() && self.uncertain {
            return Err(format!(
                "Cannot prove the selected contour is unthreaded after a source transform or deformation. {}",
                self.diagnostic()
            ));
        }
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        for (at, direction, _) in self.axes.values() {
            if (dot(axis, *direction).abs() - 1.).abs() > 1e-8 {
                continue;
            }
            let delta = std::array::from_fn(|i| origin[i] - at[i]);
            let along = dot(delta, *direction);
            let radial = std::array::from_fn::<_, 3, _>(|i| delta[i] - along * direction[i]);
            if dot(radial, radial).sqrt() <= tolerance.max(1e-6) {
                return Err(self.diagnostic());
            }
        }
        Ok(())
    }
}
