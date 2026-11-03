<!-- Kind: reference; Audience: catalog maintainers and reviewers; Purpose: record source, geometry and coverage of the bundled Library; Non-goals: certify manufactured parts or reproduce vendor CAD. -->
# Included Library definitions

The Library includes normal-series plain washers, generic trees and shrubs, four curated bearing mounting envelopes and generic apartment equipment. All use dynamic, pinned native symbol definitions.

## Mechanical envelopes

The bearing projects contain dynamic, pinned bearing-envelope definitions. Bore, outside diameter and width are exposed instance inputs. Each definition retains its own literal dimensional reference; changing an input creates custom dimensions without changing that reference. Updating a library file does not update placed instances automatically.

The solids describe external mounting envelopes. They omit races, rolling elements, shields, seals and fillets. In particular, an annular envelope's volume is not a real bearing's material volume or mass. They carry no load, life, tolerance or fit certification.

## Dimensional sources

### Plain washers

[washers-curation.json](washers-curation.json) owns the exact requested nominal coverage, physical dimensions, source commit/hash, cross-checks and the explicit M10 bore correction. Fastener nominal diameter is a retained catalog fact, separate from physical bore. Material and grade remain unspecified. The selected series is BOLTS `plainwasher1` normal-series reference dimensions at the pinned revision; upstream standard labels without editions do not certify a current standard.

[Washer source notices](sources/bolts/WASHER-NOTICES.md) identify the LGPL-2.1-or-later dimensional data, original author, dated adaptation and geometry omissions. The original [washer.blt](sources/bolts/washer.blt) remains unmodified. Manufacturer sources corroborate dimensions; their CAD, artwork and catalog prose are not bundled.

The washer projects expose bore, outside diameter and thickness as editable instance inputs. Their literal dimensional reference and nominal fact remain attached when a configuration becomes custom. No nominal is inferred from a bore measurement. Native previews show the actual annular geometry. The models omit dimensional tolerances, edge breaks and chamfers.

### Bearing mounting envelopes

The selected dimensions are adapted from `data/bearings.blt` in [BOLTS revision 5c83300a57761f70db87f50c9a1f647c7278a4c2](https://github.com/boltsparts/boltsparts/blob/5c83300a57761f70db87f50c9a1f647c7278a4c2/data/bearings.blt). That file identifies Johannes Reinhardt and Javier Martínez García as authors and grants LGPL 2.1 or later. Its complete unmodified source and license accompany this catalog in `sources/bolts/`. `curation.json` records the source hash, selected dimensions and cross-check date.

All four outer dimension triples were independently checked against these manufacturer pages on 2026-11-01:

| Open designation | Bore (mm) | Outside (mm) | Width (mm) | Cross-check |
| --- | ---: | ---: | ---: | --- |
| 608 | 8 | 22 | 7 | [JTEKT 608](https://www.jtekt.co.jp/products/bearing/en/p/detail/?pno=608) |
| 6000 | 10 | 26 | 8 | [JTEKT 6000](https://koyo.jtekt.co.jp/en/products/detail/?pno=6000) |
| 6200 | 10 | 30 | 9 | [JTEKT 6200](https://www.jtekt.co.jp/products/bearing/en/p/detail/?pno=6200) |
| 6300 | 10 | 35 | 11 | [JTEKT 6300](https://www.jtekt.co.jp/products/bearing/en/p/detail/?pno=6300) |

Only the licensed reference dimensions are adapted. No manufacturer CAD, meshes, images, logos or catalog prose are included. Manufacturer pages provide cross-checks, rather than redistribution grants. BOLTS fillet values differ from the manufacturer pages and are deliberately unused; the model explicitly omits fillets. The BOLTS standard labels do not establish compliance with a current standard edition.

## Generation and revisions

The Rust developer example `mechanical_library` builds washer source through the same typed transactions and native compiler as the application. Saved previews are software renderings of those native solids. It reads the reviewed washer curation, creates dynamic symbol definitions, and writes `.tcd` files plus the bounded `library.json` manifest. Existing non-washer projects are checksum-checked and copied byte for byte from the repository Library, preserving their identities, revisions and pins. The output folder must be empty. Required source data, notices and this README accompany the generated manifest automatically.

Regenerate into an empty review folder:

```sh
cargo run -p multicad-app --example mechanical_library --locked -- /tmp/toskcad-hardware-review
```

Washer document, feature, Part, parameter and symbol identities use a separate namespace and exact scaled nominal keys, including fractional millimeter sizes. Bearing identities retain their existing designation-derived namespace. Adding a washer does not renumber another definition. A dimensional or definition change requires a new catalog/symbol revision and explicit update review. Runtime geometry comes from the retained native source, never from a lookup in a newer dimension table.

## Generic planting presets

`plant-generic-tree.tcd` and `plant-generic-shrub.tcd` are owned generic context definitions, with editable height and outer canopy/spread plus a linked authored plan. They use the same pins as the Architecture Planting tool. The simple tree has a trunk and cylindrical crown; the shrub has one cylindrical envelope. Plan circles use double and single rings respectively. These are illustrative planned-size defaults, with no species, growth or supplier claims and no copied artwork. [Planting source and drawing policy](../docs/architecture/plants.md) records dimensions, visibility and output behavior.

`plant_library` regenerates just those two projects into an empty folder. Plant identities are fixed by preset kind and source role. Changing a definition requires an explicit revision and pin update. `mechanical_library` preserves their existing files when extending the complete bundled catalog.

## Generic apartment equipment

The 22 `equipment-*.tcd` projects supply owned generic electrical/lighting, heating, sanitary and appliance families. Dimensions remain editable instance inputs. Power outlets identify Type F; an E27 lamp holder is separate from luminaires, and a cooktop is separate from a full cooker with oven. A junction box has an enclosure and cover with a generic JB plan mark; its editable defaults are planning dimensions, not installation specifications. Descriptions carry family-specific search terms. Native 3D forms, true-size monochrome footprints and optional readable plan marks retain the same definition pins as the Equipment tool. [Equipment source and drawing policy](../docs/architecture/equipment.md) explains host datums and boundaries.

`equipment_library` regenerates these 22 projects alone. Every Library preview renders a deterministic temporary native occurrence without adding it to the saved definition or changing its pin. The renderer rejects empty or blank output. These are planning envelopes and original generic symbols; no manufacturer geometry or standards artwork is included. `mechanical_library` preserves their existing files when extending the complete bundled catalog.

## Hardware coverage still open

The washer curation owns its bounded normal-series nominal coverage and explicit unavailable/custom cases. The Library also supplies four open bearing mounting envelopes. Sealed/shielded variants, detailed bearing internals, stocked screws and sourced hex-nut presets remain outside these entries.

The exact requested fastener nominal matrix remains in the washer curation, with larger sizes extensible through explicit reviewed revisions. Screw head/drive combinations, wood-thread conventions, material variants and fastener-connection integration require their own sourced definitions. No bearing or washer reference metadata asserts stock availability.
