use super::*;
impl Inputs {
    pub(super) fn value(
        self,
        doc: &Document,
        params: &EvaluatedParameters,
        previous: Option<&ArchitectureInputs>,
    ) -> Result<ArchitectureInputs, ToolError> {
        let mut result = ArchitectureInputs::default();
        for (key, v) in self.parameters {
            result.parameters.insert(
                id(&key)?,
                symbols::SymbolValue {
                    value: v.value,
                    unit: v.unit,
                },
            );
        }
        for (key, v) in self.dimensions {
            let key = id(&key)?;
            result.dimensions.insert(
                key,
                super::fences::dimension(
                    &v,
                    previous.and_then(|p| p.dimensions.get(&key)),
                    doc,
                    params,
                )?,
            );
        }
        for (key, v) in self.materials {
            result.materials.insert(id(&key)?, typed(v)?);
        }
        for (key, v) in self.options {
            result.options.insert(id(&key)?, typed(v)?);
        }
        Ok(result)
    }
}
impl Context {
    pub(super) fn value(
        self,
        doc: &Document,
        params: &EvaluatedParameters,
        previous: Option<&ArchitectureInstance>,
    ) -> Result<ArchitectureContext, ToolError> {
        let q = |q: Quantity, old| super::fences::dimension(&q, old, doc, params);
        match self {
            Self::Wall {
                path,
                closed,
                level,
                base_z,
                flipped,
            } => {
                let old = previous
                    .and_then(|p| match p.output {
                        ArchitectureOutput::Wall { feature, .. } => Some(feature),
                        _ => None,
                    })
                    .map(|id| read_wall(doc, id))
                    .transpose()?;
                Ok(ArchitectureContext::Wall {
                    path: path
                        .into_iter()
                        .enumerate()
                        .map(|(index, [x, y])| {
                            let prior = old.as_ref().and_then(|v| v.path.get(index));
                            Ok([q(x, prior.map(|v| &v[0]))?, q(y, prior.map(|v| &v[1]))?])
                        })
                        .collect::<Result<_, ToolError>>()?,
                    closed,
                    level: level.as_deref().map(id).transpose()?,
                    base_z: q(base_z, old.as_ref().map(|v| &v.base_z))?,
                    flipped,
                    details: None,
                })
            }
            Self::Fence {
                vertices,
                closed,
                level,
                base_z,
                post_positions,
            } => {
                let old = previous
                    .and_then(|p| match p.output {
                        ArchitectureOutput::Fence { run, .. } => Some(run),
                        _ => None,
                    })
                    .map(|id| read_fence_run(doc, id))
                    .transpose()?;
                Ok(ArchitectureContext::Fence {
                    path: FencePath {
                        vertices: vertices
                            .into_iter()
                            .map(|v| {
                                let identity =
                                    v.id.as_deref().map(id).transpose()?.unwrap_or_default();
                                let prior = old.as_ref().and_then(|p| {
                                    p.path.vertices.iter().find(|v| v.id == identity)
                                });
                                let [x, y] = v.point;
                                Ok(FenceVertex {
                                    id: identity,
                                    point: [
                                        q(x, prior.map(|v| &v.point[0]))?,
                                        q(y, prior.map(|v| &v.point[1]))?,
                                    ],
                                })
                            })
                            .collect::<Result<_, ToolError>>()?,
                        closed,
                        base_z: q(base_z, old.as_ref().map(|v| &v.path.base_z))?,
                    },
                    level: level.as_deref().map(id).transpose()?,
                    post_positions: post_positions
                        .into_iter()
                        .map(|v| {
                            let post = FencePostKey::Interior {
                                start: id(&v.start)?,
                                end: id(&v.end)?,
                                ordinal: v.ordinal,
                            };
                            let prior = old
                                .as_ref()
                                .and_then(|p| p.post_positions.iter().find(|v| v.post == post));
                            Ok(FencePostPosition {
                                post,
                                offset: q(v.offset, prior.map(|v| &v.offset))?,
                            })
                        })
                        .collect::<Result<_, ToolError>>()?,
                })
            }
        }
    }
}
