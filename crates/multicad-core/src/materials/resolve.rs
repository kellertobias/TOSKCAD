use super::*;
use crate::document::{Document, DocumentError, EntityData, EntityId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    Missing,
    WrongKind,
    DefinitionChanged,
    MaterialMismatch,
    UnsupportedLegacyHatch,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub owner: EntityId,
    pub target: EntityId,
    pub role: String,
    pub kind: DiagnosticKind,
    pub message: String,
}
fn problem(
    owner: EntityId,
    target: EntityId,
    role: &str,
    kind: DiagnosticKind,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        owner,
        target,
        role: role.into(),
        kind,
        message: message.into(),
    }
}
pub fn part_properties(
    document: &Document,
    part: EntityId,
) -> Result<Option<(EntityId, PartProperties)>, DocumentError> {
    for e in document.entities() {
        if let Some(LibraryDefinition::PartProperties(p)) = read(e)?
            && p.part == part
        {
            return Ok(Some((e.id, p)));
        }
    }
    Ok(None)
}
pub fn material_style(
    document: &Document,
    material: EntityId,
) -> Result<Option<(EntityId, MaterialStyle)>, DocumentError> {
    for e in document.entities() {
        if let Some(LibraryDefinition::MaterialStyle(p)) = read(e)?
            && p.material == material
        {
            return Ok(Some((e.id, p)));
        }
    }
    Ok(None)
}
fn matches_role(document: &Document, id: EntityId, role: &str) -> bool {
    let Some(e) = document.get(id) else {
        return false;
    };
    match role {
        "part" => matches!(e.data, EntityData::Part { .. }),
        "material" => matches!(e.data, EntityData::Material { .. }),
        _ => matches!(
            (role, read(e)),
            ("stock", Ok(Some(LibraryDefinition::Stock(_))))
                | ("appearance", Ok(Some(LibraryDefinition::Appearance(_))))
                | ("hatch", Ok(Some(LibraryDefinition::Hatch(_))))
        ),
    }
}
pub fn diagnostics(document: &Document) -> Result<Vec<Diagnostic>, DocumentError> {
    let mut output = Vec::new();
    for entity in document.entities() {
        let Some(def) = read(entity)? else { continue };
        for l in def.links() {
            if document.get(l.target).is_none() {
                output.push(problem(
                    entity.id,
                    l.target,
                    &l.role,
                    DiagnosticKind::Missing,
                    format!("Missing {} definition {}", l.role, l.target),
                ));
            } else if !matches_role(document, l.target, &l.role) {
                output.push(problem(
                    entity.id,
                    l.target,
                    &l.role,
                    DiagnosticKind::WrongKind,
                    format!("{} is not a {} definition", l.target, l.role),
                ));
            }
        }
        let refs = match &def {
            LibraryDefinition::MaterialStyle(s) => {
                vec![(&s.appearance, "appearance"), (&s.hatch, "hatch")]
            }
            LibraryDefinition::PartProperties(s) => vec![
                (&s.material, "material"),
                (&s.stock, "stock"),
                (&s.appearance, "appearance"),
                (&s.hatch, "hatch"),
            ],
            _ => vec![],
        };
        for (binding, role) in refs {
            if let Some(binding) = binding
                && let Some(target) = document.get(binding.id)
                && fingerprint(target)? != binding.content
            {
                output.push(problem(
                    entity.id,
                    binding.id,
                    role,
                    DiagnosticKind::DefinitionChanged,
                    format!(
                        "{} definition changed; review and reassign to acknowledge",
                        target.name
                    ),
                ));
            }
        }
        if let LibraryDefinition::PartProperties(p) = def
            && let Some(part) = document.get(p.part)
            && let EntityData::Part { material, .. } = &part.data
        {
            if p.material.as_ref().map(|r| r.id) != *material {
                output.push(problem(
                    entity.id,
                    p.part,
                    "material",
                    DiagnosticKind::MaterialMismatch,
                    "Part material changed outside its assignment; review the stock and overrides",
                ));
            }
            if let Some(stock) = p.stock
                && let Some(e) = document.get(stock.id)
                && let Some(LibraryDefinition::Stock(s)) = read(e)?
                && Some(s.material) != *material
            {
                output.push(problem(
                    entity.id,
                    stock.id,
                    "stock",
                    DiagnosticKind::MaterialMismatch,
                    "Stock belongs to a different manufacturing material",
                ));
            }
        }
    }
    // Legacy Part.material references are also visible in this single report.
    for e in document.entities() {
        if let EntityData::Part {
            material: Some(id), ..
        } = &e.data
        {
            if document.get(*id).is_none() {
                output.push(problem(
                    e.id,
                    *id,
                    "material",
                    DiagnosticKind::Missing,
                    "Part material is missing",
                ));
            } else if !matches_role(document, *id, "material") {
                output.push(problem(
                    e.id,
                    *id,
                    "material",
                    DiagnosticKind::WrongKind,
                    "Part material has the wrong entity kind",
                ));
            }
        }
    }
    Ok(output)
}
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveProperties {
    pub material: Option<EntityId>,
    pub stock: Option<(EntityId, Stock)>,
    pub appearance: Appearance,
    pub hatch: Option<HatchStyle>,
    pub diagnostics: Vec<Diagnostic>,
}
/// Broken explicit assignments return an error rather than falling through to an
/// unrelated default. Changed-content warnings retain the live linked definition.
pub fn resolve(document: &Document, part: EntityId) -> Result<EffectiveProperties, DocumentError> {
    let e = document.get(part).ok_or(DocumentError::Missing(part))?;
    let EntityData::Part { material, .. } = &e.data else {
        return Err(DocumentError::Invalid(format!("{part} is not a Part")));
    };
    let props = part_properties(document, part)?;
    let style = material
        .map(|m| material_style(document, m))
        .transpose()?
        .flatten();
    let owners = [
        Some(part),
        props.as_ref().map(|s| s.0),
        style.as_ref().map(|s| s.0),
    ];
    let related: Vec<_> = diagnostics(document)?
        .into_iter()
        .filter(|d| owners.contains(&Some(d.owner)))
        .collect();
    if let Some(d) = related
        .iter()
        .find(|d| d.kind != DiagnosticKind::DefinitionChanged)
    {
        return Err(DocumentError::Invalid(d.message.clone()));
    }
    let mut appearance = Appearance::default();
    let mut hatch = None;
    if let Some(id) = material {
        let source = document.get(*id).ok_or(DocumentError::Missing(*id))?;
        if let EntityData::Material {
            color_rgba,
            section_hatch,
            ..
        } = &source.data
        {
            appearance.color_rgba = *color_rgba;
            if let Some(def) = section_hatch {
                if def.type_id == "materials.hatch.inline" && def.schema_version == 1 {
                    let value: HatchStyle = serde_json::from_value(def.data.clone())
                        .map_err(|e| DocumentError::Invalid(e.to_string()))?;
                    LibraryDefinition::Hatch(value.clone()).validate()?;
                    hatch = Some(value);
                } else {
                    return Err(DocumentError::Invalid(format!(
                        "Material {id} uses unsupported legacy section hatch {}",
                        def.type_id
                    )));
                }
            }
        }
    }
    let props = props.map(|p| p.1);
    let style = style.map(|p| p.1);
    let ar = props
        .as_ref()
        .and_then(|p| p.appearance.as_ref())
        .or_else(|| style.as_ref().and_then(|p| p.appearance.as_ref()));
    let hr = props
        .as_ref()
        .and_then(|p| p.hatch.as_ref())
        .or_else(|| style.as_ref().and_then(|p| p.hatch.as_ref()));
    if let Some(r) = ar
        && let Some(LibraryDefinition::Appearance(a)) =
            read(document.get(r.id).ok_or(DocumentError::Missing(r.id))?)?
    {
        appearance = a;
    }
    if let Some(r) = hr
        && let Some(LibraryDefinition::Hatch(h)) =
            read(document.get(r.id).ok_or(DocumentError::Missing(r.id))?)?
    {
        hatch = Some(h);
    }
    let stock = props
        .and_then(|p| p.stock)
        .map(|r| -> Result<_, DocumentError> {
            let Some(LibraryDefinition::Stock(s)) =
                read(document.get(r.id).ok_or(DocumentError::Missing(r.id))?)?
            else {
                return Err(DocumentError::Invalid("Invalid stock reference".into()));
            };
            Ok((r.id, s))
        })
        .transpose()?;
    Ok(EffectiveProperties {
        material: *material,
        stock,
        appearance,
        hatch,
        diagnostics: related,
    })
}
