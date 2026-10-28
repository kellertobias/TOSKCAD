#include "analytic.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Curve.hxx>
#include <BRep_Tool.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Circ.hxx>
#include <cmath>
#include <stdexcept>

namespace multicad::kernel {
static Point3 point(const gp_Pnt &p) { return {p.X(), p.Y(), p.Z()}; }
static Point3 direction(const gp_Dir &p) { return {p.X(), p.Y(), p.Z()}; }
rust::Vec<AnalyticEdgeData> analytic_edges(const Shape &shape) {
  try {
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(shape.value, TopAbs_EDGE, edges);
    if (edges.Extent() > 100000)
      throw std::runtime_error("Analytic edge budget exceeded");
    rust::Vec<AnalyticEdgeData> result;
    for (int i = 1; i <= edges.Extent(); ++i) {
      const auto edge = TopoDS::Edge(edges(i));
      if (BRep_Tool::Degenerated(edge)) continue;
      BRepAdaptor_Curve curve(edge);
      const auto first = curve.FirstParameter(), last = curve.LastParameter();
      if (!std::isfinite(first) || !std::isfinite(last) || last <= first)
        continue;
      AnalyticEdgeData data{};
      data.index = static_cast<uint32_t>(i - 1);
      if (curve.GetType() == GeomAbs_Line) {
        data.kind = 0;
        data.a = point(curve.Value(first));
        data.b = point(curve.Value(last));
      } else if (curve.GetType() == GeomAbs_Circle) {
        const auto circle = curve.Circle();
        data.kind = 1;
        data.frame = {point(circle.Location()),
                      direction(circle.XAxis().Direction()),
                      direction(circle.YAxis().Direction())};
        data.radius = circle.Radius();
        data.start = first;
        data.sweep = last - first;
      } else {
        // Unsupported analytic families stay unavailable. A sampled polyline
        // is never substituted for exact manufacturing/reference geometry.
        continue;
      }
      result.push_back(data);
    }
    return result;
  } catch (const Standard_Failure &e) {
    throw std::runtime_error(e.GetMessageString() ? e.GetMessageString()
                                                : "Analytic edge query failed");
  }
}
}
