# Additional screw-profile dimensional data notices

The additional profile records in `machine-screws-curation.json` are independently authored scalar selections and engineering adaptations. They extend the Library without changing the nineteen existing machine-screw project definitions or pins. The original machine-screw records and their source notices remain applicable separately.

## BOLTS starting data

The accompanying unmodified `hex_socket.blt` identifies Johannes Reinhardt, copyright 2013, and grants LGPL version 2.1 or later. Its pinned upstream revision is `5c83300a57761f70db87f50c9a1f647c7278a4c2`; its SHA256 is `7d1f57b64c0d8b574beaabaf4dedc8a11b6980371d9e9508c0bd5399a2cc1f53`. Original notices and disclaimers remain intact, and the full license is in `COPYING.LESSER`. The independently authored additional curation arrangement and adaptations are provided under LGPL-2.1-or-later. This grant does not license separate standards, manufacturer works, trademarks or assets by implication.

The BOLTS `hexsocketcountersunk` rows carry both DIN 7991 and ISO 10642 labels but differ from the selected ISO 10642:2004 facts. BOLTS M3 has D = 6, k = 1.7, t = 1.2 mm, whereas ISO 2004 gives theoretical maximum diameter 6.72, actual minimum diameter 5.54, head-height maximum 1.86 and straight socket-depth minimum 1.1. Its M2/M2.5 rows are outside that edition's M3-M20 scope and remain unavailable for this family. Original rows are preserved; new source records identify the separate verified scalar facts.

## Primary references and rights

The curation's `profile_sources` records the primary URLs, editions where known, research-copy hashes and page references. Selected families use ISO 14583:2011 pan hexalobular, ISO 14581:2013 countersunk hexalobular, ISO 7045:2011 pan typeH, ISO 7046-1:2011 countersunk typeH, ISO 7380-1:2011 button hex socket and ISO 10642:2004 countersunk hex socket. Historical editions remain explicitly named. The FILA socket-cap sheet is undated and labels ISO 14579 without an edition; its diameter/height columns are minima. The exact Accu SHB-M3-10-A2-R360 datasheet says based on ISO 7380-1 and does not establish an ISO hexalobular-button family or a specific standard edition.

Public access to a source PDF is not a redistribution license. No ISO/manufacturer PDF, diagram, catalog prose, logo, CAD model or mesh is distributed with this data. Only selected dimensional facts, original references and independently authored adaptations are retained. Source standards' thread-fit, material, grade, finish and mechanical-property requirements are not transferred to an unspecified native model.

Accu SHB-M3-10-A2-R360 retains Stainless steel and the A2 material designation. The FILA ISO 14579 rows retain Stainless steel. Their source offers A2 and A4, so the generic dimensional presets leave grade unspecified. The reference fields leave project material assignments independent. [Accu product specification](https://www.accu.co.uk/torx-button-screws/256076-SHB-M3-10-A2-R360), [FILA ISO 14579 data sheet](https://www.filafixations.com/userfiles/Fiches_techniques/fiches_techniques_PDF/Fiche_Technique_ISO_14579.pdf)

Named standard length rows and manufacturer product lengths are distinct. Four principal pan/countersunk families select only L12. Button hex selects L12 except M10 L16. FILA selects only M6 L30, M8 L40 and M10 L40; Accu retains its one identified M3 L10 product. These selections do not promise current stock or imply intermediate lengths. All twelve requested nominals have an explicit status for each additional family; unavailable describes this curation rather than global commercial nonexistence.

## Engineering geometry

Original head/recess limits remain separate from selected physical targets. Exact circular crowns, flat top lands, rim/edge details, neck blends and recess bottoms are engineering choices within the stated dimensional references. Approximate radii do not become uniquely prescribed exact surfaces. The physical top land retains a measurable head height after recess cutting. Button-hex uses its source land-diameter reference; undimensioned pan/Torx-button lands are explicit targets. Minimum underhead blend bounds are retained and the selected positive transition is modeled.

Countersunk length includes the head; other selected families use underhead length. Each new definition has one typed length source, and its thread length remains an expression subtracting head height where applicable and the positive retained plain transition. Perfect nominal-basic thread/runout details are an engineering adaptation of the source's incomplete-thread conventions.

The actual countersunk rim diameter is distinct from the theoretical cone diameter. Selected 90-degree cones, physical rim and small top-edge chamfer retain a flat top annulus; these choices do not certify the standard's complete head-gauge contour. FILA's missing maximum bounds are not invented, and the Accu sheet's missing crown/bottom data remains attributed as engineering geometry.

Phillips typeH is distinct from Pozidriv typeZ. The H0 inner circle replaces alpha; it is separate from the nominal mouth roundover and has no dormant alpha/e control. Actual inclined C-C geometry is retained. Product penetration is a gauge seating depth from the defined reference plane, not the deepest removed point. The source's reference t1 follows from g and the 28-degree floor. Source f and gauging facts remain separate from the native engineering contour; they are not silently replaced by a flat-bottom diameter or claimed as complete gauge acceptance.

Hexalobular contours use twelve tangent circular arcs, not an illustrative star. Selected basic A/B and an engineering outer-lobe radius are distinguished from normative GO/NoGO gauge surfaces and informative approximate construction ratios. Product size45 A-reference 7.95 and the basic table A7.93 remain separately recorded without an invented explanation. Recess, driver and gauge geometry are not interchangeable. Bottom geometry beyond the functional region is explicitly chosen.

## Implementation reference and limits

FreeCAD FastenersWB was inspected as a geometric reference but no source code or CSV table was imported into these adaptations. Its pinned `screw_maker.py` carries LGPL version 2-or-later, while the repository root license is GPLv2 and individual CSV files have no embedded grant. Those rights are distinct from BOLTS and from independently retained source facts.

Native models use the shared nominal-basic metric thread and typed modeled/cosmetic representation choice. Matching source dimensions does not certify ISO 965 fit, gauge acceptance, physical bit engagement, tooling, strength, torque, fatigue, material, coating, stock or print readiness. Native validation and manufacturing checks apply independently. There is no warranty of merchantability or fitness for a particular purpose.
