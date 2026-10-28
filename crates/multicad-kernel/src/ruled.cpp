#include "ruled.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepFill.hxx>
#include <BRepLib.hxx>
#include <BRep_Tool.hxx>
#include <GeomConvert.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Solid.hxx>
#include <stdexcept>
namespace multicad::kernel {
namespace {
TopoDS_Edge rational(const TopoDS_Edge &edge) {
  double first, last;
  auto curve = BRep_Tool::Curve(edge, first, last);
  // BRepFill's profiler approximates Geom_Conic inputs. Convert explicitly
  // using exact rational conic conversion, bypassing that approximation path.
  auto exact = GeomConvert::CurveToBSplineCurve(
      new Geom_TrimmedCurve(curve, first, last));
  BRepBuilderAPI_MakeEdge built(exact);
  if (!built.IsDone())
    throw std::runtime_error("Exact rational circle edge construction failed");
  return built.Edge();
}
TopoDS_Face side(const TopoDS_Edge &a, const TopoDS_Edge &b, uint8_t kind) {
  if (kind == 3)
    return BRepFill::Face(rational(a), rational(b));
  if (kind != 0)
    throw std::runtime_error(
        "Taper ruled adapter supports only line/circle sections");
  BRepBuilderAPI_MakePolygon wire;
  wire.Add(BRep_Tool::Pnt(TopExp::FirstVertex(a)));
  wire.Add(BRep_Tool::Pnt(TopExp::LastVertex(a)));
  wire.Add(BRep_Tool::Pnt(TopExp::LastVertex(b)));
  wire.Add(BRep_Tool::Pnt(TopExp::FirstVertex(b)));
  wire.Close();
  if (!wire.IsDone())
    throw std::runtime_error("Taper planar side wire construction failed");
  BRepBuilderAPI_MakeFace face(wire.Wire(), true);
  if (!face.IsDone())
    throw std::runtime_error("Taper side is not an exact planar quad");
  return face.Face();
}
} // namespace
std::unique_ptr<Shape> ruled_profile_solid(
    const TopoDS_Face &start, const TopoDS_Face &end,
    const std::vector<std::pair<TopoDS_Edge, uint32_t>> &startEdges,
    const std::vector<std::pair<TopoDS_Edge, uint32_t>> &endEdges,
    rust::Slice<const CurvedEdgeData> kinds, double tolerance) {
  if (startEdges.size() != endEdges.size() || startEdges.size() != kinds.size())
    throw std::runtime_error("Taper edge correspondence changed");
  std::vector<TopoDS_Face> sides;
  BRepBuilderAPI_Sewing sewing(tolerance, true, true, false, false);
  sewing.SetMaxTolerance(tolerance);
  sewing.Add(start);
  sewing.Add(end);
  for (size_t i = 0; i < startEdges.size(); ++i) {
    if (startEdges[i].second != endEdges[i].second)
      throw std::runtime_error("Taper authored edge mapping changed");
    sides.push_back(
        side(startEdges[i].first, endEdges[i].first, kinds[i].kind));
    sewing.Add(sides.back());
  }
  sewing.Perform();
  const auto shell = sewing.SewedShape();
  if (shell.IsNull() || shell.ShapeType() != TopAbs_SHELL ||
      sewing.NbFreeEdges() != 0 || sewing.NbMultipleEdges() != 0 ||
      sewing.NbDeletedFaces() != 0)
    throw std::runtime_error(
        "Taper sections do not form one watertight manifold shell (free=" +
        std::to_string(sewing.NbFreeEdges()) +
        ", multiple=" + std::to_string(sewing.NbMultipleEdges()) +
        ", deleted=" + std::to_string(sewing.NbDeletedFaces()) + ")");
  BRepBuilderAPI_MakeSolid make(TopoDS::Shell(shell));
  if (!make.IsDone())
    throw std::runtime_error("Taper solid construction failed");
  auto solid = make.Solid();
  if (!BRepLib::OrientClosedSolid(solid) ||
      !BRepCheck_Analyzer(solid).IsValid())
    throw std::runtime_error("Taper solid is not valid and closed");
  auto result = std::make_unique<Shape>(Shape{solid, {}, {}});
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(solid, TopAbs_FACE, faces);
  auto seed = [&](const TopoDS_Face &face, uint8_t role, uint32_t source) {
    const auto actual = sewing.IsModified(face) ? sewing.Modified(face) : face;
    auto append = [&](const TopoDS_Shape &f) {
      const int index = faces.FindIndex(f);
      if (index > 0)
        result->seeds.push_back(
            NativeSeed{static_cast<uint32_t>(index - 1), source, role});
    };
    if (actual.ShapeType() == TopAbs_FACE)
      append(actual);
    else
      for (TopExp_Explorer f(actual, TopAbs_FACE); f.More(); f.Next())
        append(f.Current());
  };
  seed(start, 0, 0);
  seed(end, 1, 0);
  for (size_t i = 0; i < sides.size(); ++i)
    seed(sides[i], 2, startEdges[i].second);
  return result;
}
} // namespace multicad::kernel
