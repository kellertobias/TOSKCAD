# Receiving-hole recommendations

The metric receiving defaults use the manufacturer technical sheet identified
in [source-ledger.json](source-ledger.json). The bounded numeric adaptation
covers M3, M4, M5, M6, M8 and M10. The source PDF and its artwork are not bundled.

Page 3 supplies coarse pitch and pre-tapping drill diameter. A pre-tapping
drill is not a modeled internal thread. The sheet's 6H/6g statements concern
the manufacturer's catalogue metal threads; they do not certify another
supplier's hardware, a generated thread, an insert or a hanger bolt. The
catalogue's generic thread-length paragraph does not determine the usable
thread length of selected hardware.

Page 4 supplies fine H12 and medium H13 clearance-hole diameters. Medium
Form A countersinks use the nominal 90 degree cone, with the source showing
positive 1 degree and zero negative angle deviation. The tabulated T1 is
approximate. The implementation derives cone depth from the chosen medium
hole diameter, countersink diameter and nominal angle; it does not constrain
that depth to the rounded T1 value as well. Fine Form B countersink rims are
outside this adaptation.

The counterbore depth comes from Form K for DIN 912 socket cap heads, as shown
in the rendered page. Form J for DIN 6912 is a different, shallower seat. The
source's D3 relief column has no value for any adapted size. No relief is
invented. A seat for a socket cap head does not establish room for a washer
or another head type; actual component dimensions need a separate check.

These recommendations supply nominal defaults. Hole extent, receiver
material, tool clearance, thread engagement, blind-hole bottom clearance,
manufacturing tolerance and structural suitability require their own explicit
inputs and validation. No universal engagement ratio or timber pilot rule is
derived from this sheet.
