# MultiCAD UX prototype — workflow revision

Open `mockup/index.html` directly, or run `node mockup/serve.mjs` and visit http://127.0.0.1:4173. No install, build step, remote assets, account, or CAD runtime is needed.

## Navigation

The colored application modes are **Parameters, Sketch, Model, Drawings, Render, Export**. “Drawings” names the editable 2D documents; Export packages their deliverables.

The header keeps the file, saved state, explicit Save, Undo/Redo, tool search, and user control. A second row contains contextual ribbon tabs, with **Measure, Library, MCP** always available. The next row contains the ribbon’s tools.

The parts/assemblies tree stays on the left. The **Inspector** on the right is blank when there is no selection or tool. A selection shows material, drawing style, tags, and Copy/Move/Symbol actions. An active tool replaces that content with its controls and preview. Checkboxes support selection of multiple parts. Escape clears the current tool or selection.

Ribbon changes keep the same project geometry. Use the file picker for different examples, or **Explore workflows** at the bottom right for direct entry points.

## Concrete review paths

| Workflow | What to try |
| --- | --- |
| Parameters | Edit names, descriptions, values/expressions, units, and exposure. Add/remove a row. The cabinet/drawer arithmetic is an illustrative live dependency. Exposed rows become dynamic-symbol inputs. |
| Sketch references | Sketch → References → Sketch plane. Choose an origin, face, or offset plane. Project/intersect references demonstrates a neighboring cabinet’s coplanar edge and a crossing wall’s intersection. Select profiles and extrude independent parts from the shared sketch. |
| Architecture | Model → Architecture → Wall. Choose a saved type, edit thickness/material/height, save another type, then click two floor-plane points or use the example segment and Apply. Doors, windows, levels/pedestals, and stairs have reusable definitions too. |
| Measure | Use the global button, choose Faces/Edges/Points, pick A and B in the scene, or use the named example pair. The readout has a minimum separation and axis components. This creates no sketch constraint. |
| Model edges | Model → Common → Fillet / chamfer. Switch the profile within one tool and edit radius/distance. Shell, sweep, loft, draft, and pattern remain discoverable through More features and search. |
| Cabinetry | Select multiple panels with the tree checkboxes. Cabinetry distinguishes stock panels, an envelope-based carcass, shelves/back panel, mating joints, grooves/rabbets, drilling/pockets, grain/edge settings, and cut lists. Stack/glue is a face-to-face relationship between existing panels; the layers retain separate stock identities. |
| Sheet metal | Use Control desk. Select Front panel, then Sheet metal. Base sheet/conversion identifies a stationary face and constant thickness. Bends include direction, material side, radius, and relief. Corners/seams and hems/jogs are distinct features. Unwrap part opens an associated flat pattern; folded geometry is preserved. Manufacturing review and bend schedules are separate from modeling. |
| Weldments | Use Workshop table. Member connection shows the two members, Miter/Cope/Butt arrangements, included angle, gap, end treatments, and weld definition. In the equal-width coplanar miter example, changing the angle updates both saw settings. Connections retain separate stock-member identities for cut lists. Welding order is authored; fixture positions remain optional. |
| Rail relationships | Open the worked example from Explore workflows, the tool-case tree, or Assembly → Rail → panel holes. The diagram follows source holes through assembly placement to projected sketch points and panel drilling. Change the source pitch from 32 to 40 mm. Choose receiving material and drilling depth, then Apply. |
| Drawing pages | Drawings → Add page. Configure paper, scale, number, title, author, project, revision, checker, and projection. Switch pages in the bottom strip. Add multiple views, each with its own scale, then dimensions/notes/schedules. |
| Material hatches | Select a model part → Material or Drawing hatch style. Material definitions include section pattern, angle, and spacing. Drawing sections inherit the style; the title block retains project and author information. |
| 3D drawing guides | Use Building section study → Model → Drawing guides. Set a floor-plan cut, section, or elevation; repeat by level if needed. Apply a guide, then Generate sheets. The new pages retain source-guide IDs and elevations. |
| Rendering | Render provides lightweight display styles, background, camera, and appearance. GLB handoff targets Blender for final rendering. |
| Printing | Model → FDM features contains editable breakaway tabs, thin bridges, and custom removable support geometry. Export → 3D printing contains build orientation, mesh quality, part packaging, aid inclusion, and the 3MF recipe. No slicer or G-code workflow. |
| Export sets | Select a part and edit its comma-separated tags. Export → New export set saves an All/Any tag query and chosen formats. Matching parts resolve immediately. Preview shows exactly that scope and the output recipes. PDF pages, 1:1 DXF contours, cut/BOM CSV, STEP/OBJ/3MF/STL/GLB are represented. |
| MCP | Open MCP. Preview the explicitly labeled sample external-client transaction, inspect its tools/target IDs/units/base revision, then Apply transaction and Undo. Activity history records Previewed, Applied, and Undone. Connection controls are separate. There is no built-in AI chat. |
| Symbols | Create a static or dynamic symbol from selected parts or an assembly, choose exposed parameters, place a library instance, and explicitly review definition-version updates. |

## Prototype boundaries

All geometry, measurements, validity/manufacturing checks, imports, synchronization, and exports are **illustrative**. Meshes and solids are SVG scenes. The prototype does not implement a CAD kernel, general expression solver, manufacturing rule engine, MCP endpoint, renderer, slicer, or file exporters. A package preview never produces a manufacturing file. Save records sample state in browser storage; it is not a CAD project format or a production persistence implementation.

This HTML/CSS/JavaScript artifact makes the UX reviewable. It does not change the Rust/native application direction or choose a production UI framework.

## Verification

Run `node mockup/verify.mjs` for render and state-transition checks. Browser checks cover live parameter editing, wall creation, the combined edge tool, multiple drawing pages/title blocks, measurement pairs, part tags/export-set resolution, and MCP preview/apply/undo history. Desktop layout is checked at 1280 × 800, with the normal narrow in-app panel checked separately.
