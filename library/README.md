<!-- Kind: reference; Audience: catalog maintainers and reviewers; Purpose: record source, geometry and coverage of the bundled Library; Non-goals: certify manufactured parts or reproduce vendor CAD. -->
# Included Library definitions

The Library includes metric machine screws, simplified wood-screw stock envelopes, hex-nut dimensional references, normal-series plain washers, generic trees and shrubs, four curated bearing mounting envelopes and generic apartment equipment. All use dynamic, pinned native symbol definitions.

## Mechanical envelopes

The bearing projects contain dynamic, pinned bearing-envelope definitions. Bore, outside diameter and width are exposed instance inputs. Each definition retains its own literal dimensional reference; changing an input creates custom dimensions without changing that reference. Updating a library file does not update placed instances automatically.

The solids describe external mounting envelopes. They omit races, rolling elements, shields, seals and fillets. In particular, an annular envelope's volume is not a real bearing's material volume or mass. They carry no load, life, tolerance or fit certification.

## Dimensional sources

### Metric machine screws

[machine-screws-curation.json](machine-screws-curation.json) retains nineteen original dimensional presets: fourteen socket-cap/internal-hex rows and five external-hex rows, with separate underhead/thread lengths and explicit status for all twelve requested nominal diameters. Socket dimensions combine explicit HPC 2025 SKU rows with NBK's ISO 4762:2004 / JIS B 1176:2015 limits. External-hex rows follow the selected Norelem DIN 933 dimensional matrix; the separate dated Nufast drawing supplies only the documented cross-checks. Material and grade remain unspecified.

[Machine-screw source notices](sources/bolts/MACHINE-SCREW-NOTICES.md) record the pinned LGPL-2.1-or-later BOLTS starting data, engineering targets, source discrepancies and geometry omissions. Internal hex drives are six-sided recesses with a disclosed flat floor; outer socket-cap surfaces are unspecified envelopes. DIN M10 across flats remains 17 mm, distinct from the 16 mm ISO 4017 variant. Head/drive dimensions do not establish ISO 965 thread fits, current-standard compliance or manufactured fit.

Underhead length and thread length are independently editable. The canonical mounting frame is at the underhead plane, with the head behind it and the shaft toward the tip. Source reference dimensions remain literal when a configured instance becomes custom; placed definitions retain their pins until an explicit update. Only listed source rows are presets, and unavailable/custom cases do not assert commercial unavailability.

The same curation adds 52 profile presets across eight separately identified families: ISO 14583:2011 pan/hexalobular, ISO 14581:2013 countersunk/hexalobular, ISO 7045:2011 pan/Phillips H, ISO 7046-1:2011 countersunk/Phillips H, ISO 7380-1:2011 button/internal hex, ISO 10642:2004 countersunk/internal hex, three FILA socket-cap/hexalobular product rows and one Accu button/hexalobular product. Historical editions and manufacturer-specific coverage remain explicit. Its 96 family/nominal cells record 52 selected rows and 44 unavailable cases over all twelve requested diameters. Each selected row names its one curated discrete length; other lengths and stock are not implied.

These definitions retain one typed authored length. Countersunk length includes the head; other families use underhead length. Thread extent remains an expression subtracting the head where needed and the retained plain transition. The countersunk frame is a neck construction datum. Each pin exposes a modeled/cosmetic choice independently per instance. Manufacturing refuses cosmetic envelopes.

[Profile source notices](sources/bolts/SCREW-PROFILE-NOTICES.md) distinguish exact source limits and gauge facts from the independently authored native contour. Pan, button and countersunk heads have physical front lands and tangent neck blends; FILA socket caps retain their disclosed cylindrical envelope without a neck-radius input. Hexalobular recesses use tangent circular arcs; Phillips H0–H4 retain their inclined-section geometry and separate mouth rounding. These models do not certify complete standard gauges, physical driver engagement or thread fit. Original facts remain attached when dimensional edits create a custom configuration. Source PDFs, artwork and supplier CAD are not bundled.

### Metric hex nuts

[nuts-curation.json](nuts-curation.json) owns nineteen edition-specific presets and all twenty-four family/nominal coverage statuses: ten DIN 934:1987 rows and nine ISO 4032:2012 rows. M4.5 and M9 are unavailable in both selected families; ISO 4032:2012 has no M7 row. These statements describe the selected source coverage, rather than commercial availability. M10 remains 17 mm across flats and 8 mm high in DIN, versus 16 mm and 8.4 mm in ISO. The different heights at other sizes also remain distinct.

[Nut source notices](sources/bolts/NUT-NOTICES.md) retain the pinned LGPL-2.1-or-later BOLTS data, primary cross-checks, exact editions and limits of each verification. The original [nut.blt](sources/bolts/nut.blt) is unchanged. Across-flats and height targets use the retained source maxima. The native blank is a sharp hexagon with a nominal-basic JIS B 0205-2/4:2001 internal thread; standard chamfers, washer faces, entry details and dimensional tolerances are omitted. The source reference does not certify an ISO 965 fit class, current-standard compliance, material, grade or stock.

Each pin exposes nominal thread diameter, pitch, across flats, height and thread representation. Library placement and instance editing offer Cosmetic envelope and Modeled helix as named choices. Representation is independently retained per instance; it does not require replacing a pin or changing sibling instances. Cosmetic geometry remains display-only and manufacturing refuses it. Modeled geometry contains the actual internal helix. Physical dimension edits retain their literal source facts and become configured/custom; a representation-only change preserves those dimensional facts.

### Wood-screw stock envelopes

[wood-screws-curation.json](wood-screws-curation.json) retains 44 discrete Bitork DIN 571-related external-hex dimensional rows: d4 × 16–40 mm, d5 × 16–50 mm, d6 × 20–60 mm, d8 × 25–100 mm and d10 × 30–100 mm. These ranges contain only the individually listed source lengths. The twelve requested nominal diameters each have explicit supported or unavailable status; d7 has head facts but no populated length row. Fischer TX and Phillips H combinations remain excluded because complete supported dimensions are unverified. These are wood designations, without a machine-thread M prefix. Technical table membership does not establish current supplier inventory, material or strength.

Each native preview shows a connected smooth nominal-diameter cylinder and external-hex head, using nominal head height and maximum across flats. The sharp hexagonal envelope does not reproduce the source diagonal or head chamfers. A flat distal plane encloses the manufacturer-unspecified point. Separate DIN 7998-related pitch, core and V-angle references are retained as intent; they do not create a metric helix or recover a wood-thread profile. The minimum threaded extent is 0.6 times underhead length and includes the point and thread-start transition. It is a lower bound, not actual thread length or start. All manufacturing exports refuse this simplified source, including through placed symbols and downstream geometry.

Seven instance inputs retain the nominal diameter, underhead length, head dimensions and reference profile. The minimum extent remains bound to underhead length; the immutable literal reference stays unchanged when a configuration becomes custom. Generic hardware editing also supports an explicit overall-length convention. [Source notices](sources/wood-screws/WOOD-SCREW-NOTICES.md) and the [source ledger](sources/wood-screws/source-ledger.json) record the dimensional evidence, conflicting d5 head-height maximum and rights boundaries. No source PDF, artwork, third-party CAD or FreeCAD code is bundled.

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

The Rust developer example `mechanical_library` builds the selected hardware family through the same typed transactions and native compiler as the application. Saved previews render those native solids. The default mode reads the washer curation; `--machine-screws` reads its original screw rows, `--screw-heads` reads the additional profile rows, `--hex-nuts` reads the nut curation, and `--wood-screws` reads the simplified wood-envelope curation. Each creates dynamic symbol definitions and `.tcd` files with the bounded `library.json` manifest. Projects outside the selected family are checksum-checked and copied byte for byte, preserving their identities, revisions and pins. The output folder must be empty. Required source data, notices and this README accompany the result.

Regenerate into an empty review folder:

```sh
cargo run -p multicad-app --example mechanical_library --locked -- /tmp/toskcad-hardware-review
cargo run -p multicad-app --example mechanical_library --locked -- /tmp/toskcad-screw-review --machine-screws
cargo run -p multicad-app --example mechanical_library --locked -- /tmp/toskcad-screw-head-review --screw-heads
cargo run -p multicad-app --example mechanical_library --locked -- /tmp/toskcad-nut-review --hex-nuts
cargo run -p multicad-app --example mechanical_library --locked -- /tmp/toskcad-wood-review --wood-screws
```

Washer identities use exact scaled nominal keys. Nut identities add the exact family/edition in their own namespace; representation remains an instance input. Legacy screw identities retain their head-family and underhead-length namespace. Additional profile identities use explicit family codes and the authored length in that family's retained convention, including overall length for countersunk heads. Wood-envelope identities use a separate namespace keyed by exact nominal diameter and underhead length; same-pin regeneration refuses differing project bytes or definition content. Bearing identities retain their existing designation-derived namespace. Adding an entry does not renumber another definition. A dimensional or definition change requires a new catalog/symbol revision and explicit update review. Runtime geometry comes from retained native source, never a lookup in a newer dimension table.

## Generic planting presets

`plant-generic-tree.tcd` and `plant-generic-shrub.tcd` are owned generic context definitions, with editable height and outer canopy/spread plus a linked authored plan. They use the same pins as the Architecture Planting tool. The simple tree has a trunk and cylindrical crown; the shrub has one cylindrical envelope. Plan circles use double and single rings respectively. These are illustrative planned-size defaults, with no species, growth or supplier claims and no copied artwork. [Planting source and drawing policy](../docs/architecture/plants.md) records dimensions, visibility and output behavior.

`plant_library` regenerates just those two projects into an empty folder. Plant identities are fixed by preset kind and source role. Changing a definition requires an explicit revision and pin update. `mechanical_library` preserves their existing files when extending the complete bundled catalog.

## Generic apartment equipment

The 22 `equipment-*.tcd` projects supply owned generic electrical/lighting, heating, sanitary and appliance families. Dimensions remain editable instance inputs. Power outlets identify Type F; an E27 lamp holder is separate from luminaires, and a cooktop is separate from a full cooker with oven. A junction box has an enclosure and cover with a generic JB plan mark; its editable defaults are planning dimensions, not installation specifications. Descriptions carry family-specific search terms. Native 3D forms, true-size monochrome footprints and optional readable plan marks retain the same definition pins as the Equipment tool. [Equipment source and drawing policy](../docs/architecture/equipment.md) explains host datums and boundaries.

`equipment_library` regenerates these 22 projects alone. Every Library preview renders a deterministic temporary native occurrence without adding it to the saved definition or changing its pin. The renderer rejects empty or blank output. These are planning envelopes and original generic symbols; no manufacturer geometry or standards artwork is included. `mechanical_library` preserves their existing files when extending the complete bundled catalog.

## Hardware coverage still open

The washer and nut curations own their bounded nominal coverage and explicit unavailable/custom cases. The Library also supplies four open bearing mounting envelopes. Sealed/shielded variants, detailed bearing internals and screw families beyond the listed machine-screw and simplified wood-envelope matrices remain outside these entries.

The washer, nut and screw curations share the requested nominal-size set and record their own supported choices, with larger sizes extensible through explicit reviewed revisions. Head/drive combinations beyond the listed profile matrix, modeled wood-thread and point geometry, material variants and fastener-connection integration require their own sourced definitions. Reference metadata does not assert stock availability.
