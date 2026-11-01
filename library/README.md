<!-- Kind: reference; Audience: catalog maintainers and reviewers; Purpose: record source, geometry and coverage of bundled hardware; Non-goals: certify manufactured parts or reproduce vendor CAD. -->
# Curated mechanical hardware

These native projects contain dynamic, pinned bearing-envelope definitions. Bore, outside diameter and width are exposed instance inputs. Each definition retains its own literal dimensional reference; changing an input creates custom dimensions without changing that reference. Updating a library file does not update placed instances automatically.

The solids describe external mounting envelopes. They omit races, rolling elements, shields, seals and fillets. In particular, an annular envelope's volume is not a real bearing's material volume or mass. They carry no load, life, tolerance or fit certification.

## Dimensional sources

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

The Rust developer example `mechanical_library` builds source through the same typed transactions and native compiler as the application. Saved previews are software renderings of those native solids. It reads the reviewed `curation.json`, creates dynamic symbol definitions, and writes `.tcd` files plus the bounded `library.json` manifest. It refuses to overwrite existing outputs.

Regenerate into an empty review folder:

```sh
cargo run -p multicad-app --example mechanical_library --locked -- /tmp/toskcad-hardware-review
```

Copy the source notices and this README alongside regenerated files before packaging. Document, feature, Part, parameter and symbol identities derive from the bearing designation, so adding another entry does not renumber existing definitions. A dimensional or definition change requires a new catalog/symbol revision and explicit update review. Runtime geometry comes from the retained native source, never from a lookup in a newer dimension table.

## Coverage still open

This increment supplies four open bearing mounting envelopes. It does not supply sealed/shielded variants, detailed bearing internals, stocked screws, hex nuts or washers. The generic native washer/nut/envelope generators are separate from verified catalog entries.

The requested fastener nominal sizes remain 2, 2.5, 3, 3.5, 4, 4.5, 5, 6, 7, 8, 9 and 10 mm, with larger sizes extensible. None is represented as stocked fastener coverage by these bearing entries. Screw head/drive combinations, wood-thread conventions, material filters and fastener-connection integration remain CAD-200 work.
