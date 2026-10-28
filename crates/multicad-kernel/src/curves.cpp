#include "curves.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepTopAdaptor_FClass2d.hxx>
#include <GCE2d_MakeSegment.hxx>
#include <GProp_GProps.hxx>
#include <Geom2dAPI_InterCurveCurve.hxx>
#include <Geom2dInt_GInter.hxx>
#include <Geom2d_BSplineCurve.hxx>
#include <Geom2d_Circle.hxx>
#include <Geom2d_TrimmedCurve.hxx>
#include <GeomAPI.hxx>
#include <Standard_Failure.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <TColStd_Array1OfReal.hxx>
#include <TColgp_Array1OfPnt2d.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <cmath>
#include <gp_Ax1.hxx>
#include <gp_Ax3.hxx>
#include <gp_Pln.hxx>
#include <stdexcept>

namespace multicad::kernel {
namespace {
constexpr size_t maxEdges = 64, maxPoles = 256, maxHits = 1024;
template <class F> auto guarded(F fn) -> decltype(fn()) {
  try {
    return fn();
  } catch (const Standard_Failure &e) {
    throw std::runtime_error(e.GetMessageString() ? e.GetMessageString()
                                                  : "OCCT curve query failed");
  }
}
gp_Pnt2d p2(const Point3 &p) { return gp_Pnt2d(p.x, p.y); }
Point3 data(const gp_Pnt2d &p) { return Point3{p.X(), p.Y(), 0.}; }
gp_Pln plane(const Frame3 &f) {
  gp_Vec u(f.u.x, f.u.y, f.u.z), v(f.v.x, f.v.y, f.v.z);
  return gp_Pln(gp_Ax3(gp_Pnt(f.origin.x, f.origin.y, f.origin.z),
                       gp_Dir(u.Crossed(v)), gp_Dir(u)));
}
Handle(Geom2d_Curve) curve(const CurvedEdgeData &edge) {
  if (edge.kind == 0) {
    GCE2d_MakeSegment segment(p2(edge.a), p2(edge.b));
    if (!segment.IsDone())
      throw std::runtime_error("Degenerate exact line");
    return segment.Value();
  }
  if (edge.kind == 2 || edge.kind == 3) {
    const auto direction = edge.kind == 2 ? gp_Dir2d(edge.a.x - edge.center.x,
                                                     edge.a.y - edge.center.y)
                                          : gp_Dir2d(1., 0.);
    Handle(Geom2d_Circle) circle = new Geom2d_Circle(
        gp_Ax2d(p2(edge.center), direction), edge.radius, !edge.clockwise);
    if (edge.kind == 3)
      return circle;
    const double first =
        std::atan2(edge.a.y - edge.center.y, edge.a.x - edge.center.x);
    const double last =
        std::atan2(edge.b.y - edge.center.y, edge.b.x - edge.center.x);
    const double tau = 2. * std::acos(-1.);
    const double sweep =
        std::fmod((edge.clockwise ? first - last : last - first) + tau, tau);
    if (!(sweep > 0. && sweep < tau))
      throw std::runtime_error(
          "Arc sweep must be positive and smaller than a full circle");
    return new Geom2d_TrimmedCurve(circle, 0., sweep);
  }
  if (edge.kind != 1)
    throw std::runtime_error("Unsupported exact curved edge");
  const auto &s = edge.spline;
  const auto count = s.poles.size() / 2;
  if (s.degree < 1 || s.degree > 25 || count < s.degree + 1 ||
      count > maxPoles || s.poles.size() != count * 2 || s.knots.size() < 2 ||
      s.knots.size() > maxPoles + 1 ||
      s.knots.size() != s.multiplicities.size() ||
      (!s.weights.empty() && s.weights.size() != count))
    throw std::runtime_error("Invalid spline array lengths or degree");
  TColgp_Array1OfPnt2d poles(1, static_cast<int>(count));
  TColStd_Array1OfReal knots(1, static_cast<int>(s.knots.size()));
  TColStd_Array1OfInteger mults(1, static_cast<int>(s.knots.size()));
  for (size_t i = 0; i < count; ++i)
    poles.SetValue(static_cast<int>(i + 1),
                   gp_Pnt2d(s.poles[2 * i], s.poles[2 * i + 1]));
  for (size_t i = 0; i < s.knots.size(); ++i) {
    knots.SetValue(static_cast<int>(i + 1), s.knots[i]);
    mults.SetValue(static_cast<int>(i + 1),
                   static_cast<int>(s.multiplicities[i]));
  }
  if (s.weights.empty())
    return new Geom2d_BSplineCurve(poles, knots, mults,
                                   static_cast<int>(s.degree), false);
  TColStd_Array1OfReal weights(1, static_cast<int>(count));
  for (size_t i = 0; i < count; ++i)
    weights.SetValue(static_cast<int>(i + 1), s.weights[i]);
  return new Geom2d_BSplineCurve(poles, weights, knots, mults,
                                 static_cast<int>(s.degree), false);
}
CurveQueryData query(const Handle(Geom2d_Curve) & a,
                     const Handle(Geom2d_Curve) & b, double tolerance,
                     bool self) {
  Geom2dAPI_InterCurveCurve inter;
  if (self)
    inter.Init(a, tolerance);
  else
    inter.Init(a, b, tolerance);
  if (!inter.Intersector().IsDone())
    throw std::runtime_error("Exact curve intersection did not converge");
  if (inter.NbPoints() > static_cast<int>(maxHits) ||
      inter.NbSegments() > static_cast<int>(maxHits))
    throw std::runtime_error("Curve intersection budget exceeded");
  CurveQueryData out;
  out.overlaps = static_cast<uint32_t>(inter.NbSegments());
  for (int i = 1; i <= inter.NbPoints(); ++i) {
    const auto &hit = inter.Intersector().Point(i);
    out.points.push_back(CurveIntersectionData{
        data(inter.Point(i)), hit.ParamOnFirst(), hit.ParamOnSecond()});
  }
  return out;
}
struct FaceBuild {
  TopoDS_Face face;
  std::vector<std::pair<TopoDS_Edge, uint32_t>> sources;
  double outerSign = 1.;
};
FaceBuild build(const Frame3 &f, rust::Slice<const CurvedEdgeData> edges,
                rust::Slice<const uint32_t> offsets, double tolerance) {
  if (edges.empty() || edges.size() > maxEdges || offsets.size() < 2 ||
      offsets.size() > 18 || offsets[0] != 0 ||
      offsets[offsets.size() - 1] != edges.size() ||
      !std::isfinite(tolerance) || tolerance < 1e-7 || tolerance > 1e-3)
    throw std::runtime_error("Invalid curved profile budgets or offsets");
  std::vector<Handle(Geom2d_Curve)> curves;
  std::vector<size_t> wireOf(edges.size());
  for (const auto &edge : edges)
    curves.push_back(curve(edge));
  for (size_t w = 0; w + 1 < offsets.size(); ++w) {
    if (offsets[w] >= offsets[w + 1] || offsets[w + 1] > edges.size())
      throw std::runtime_error("Invalid curved profile wire offsets");
    for (uint32_t i = offsets[w]; i < offsets[w + 1]; ++i) {
      wireOf[i] = w;
      const auto next = i + 1 == offsets[w + 1] ? offsets[w] : i + 1;
      if (curves[i]
              ->Value(curves[i]->LastParameter())
              .Distance(curves[next]->Value(curves[next]->FirstParameter())) >
          tolerance)
        throw std::runtime_error(
            "Exact profile edges are not connected within policy tolerance");
      if (edges[i].kind == 1) {
        const auto hits = query(curves[i], curves[i], tolerance, true);
        if (hits.overlaps > 0)
          throw std::runtime_error(
              "Self-overlapping spline cannot bound a profile");
        for (const auto &hit : hits.points) {
          const auto start = curves[i]->Value(curves[i]->FirstParameter()),
                     end = curves[i]->Value(curves[i]->LastParameter());
          const auto first = curves[i]->FirstParameter();
          const auto last = curves[i]->LastParameter();
          const auto parameterTolerance = (last - first) * 1e-12;
          const bool seamParameters =
              (std::abs(hit.first - first) <= parameterTolerance &&
               std::abs(hit.second - last) <= parameterTolerance) ||
              (std::abs(hit.second - first) <= parameterTolerance &&
               std::abs(hit.first - last) <= parameterTolerance);
          if (start.Distance(end) > tolerance ||
              p2(hit.point).Distance(start) > tolerance || !seamParameters)
            throw std::runtime_error(
                "Self-intersecting spline cannot bound a profile");
        }
      }
    }
  }
  for (size_t i = 0; i < edges.size(); ++i)
    for (size_t j = i + 1; j < edges.size(); ++j) {
      const auto hits = query(curves[i], curves[j], tolerance, false);
      if (hits.overlaps > 0)
        throw std::runtime_error("Overlapping or unresolved tangent boundaries "
                                 "require explicit repair");
      const auto w = wireOf[i];
      const bool adjacent =
          wireOf[j] == w &&
          (j == i + 1 || (i == offsets[w] && j + 1 == offsets[w + 1]));
      for (const auto &hit : hits.points) {
        const auto p = p2(hit.point);
        const bool endA =
            p.Distance(curves[i]->Value(curves[i]->FirstParameter())) <=
                tolerance ||
            p.Distance(curves[i]->Value(curves[i]->LastParameter())) <=
                tolerance;
        const bool endB =
            p.Distance(curves[j]->Value(curves[j]->FirstParameter())) <=
                tolerance ||
            p.Distance(curves[j]->Value(curves[j]->LastParameter())) <=
                tolerance;
        if (!adjacent || !endA || !endB)
          throw std::runtime_error("Crossing or touching profile boundaries "
                                   "require explicit repair");
      }
    }
  FaceBuild result;
  std::vector<TopoDS_Wire> wires;
  std::vector<TopoDS_Face> bounded;
  for (size_t w = 0; w + 1 < offsets.size(); ++w) {
    BRepBuilderAPI_MakeWire builder;
    for (uint32_t i = offsets[w]; i < offsets[w + 1]; ++i) {
      const auto spatial = GeomAPI::To3d(curves[i], plane(f));
      BRepBuilderAPI_MakeEdge edge(spatial, spatial->FirstParameter(),
                                   spatial->LastParameter());
      if (!edge.IsDone())
        throw std::runtime_error("Exact curve edge construction failed");
      builder.Add(edge.Edge());
      if (!builder.IsDone())
        throw std::runtime_error("Exact wire construction failed");
      result.sources.emplace_back(builder.Edge(), edges[i].source_index);
    }
    auto wire = builder.Wire();
    // Keep the supplied winding for classification. Inside=true would orient
    // only the face's copied wire, leaving `wire` unsuitable for hole
    // insertion.
    BRepBuilderAPI_MakeFace probe(plane(f), wire, false);
    if (!probe.IsDone())
      throw std::runtime_error("Exact planar face construction failed");
    if (BRepTopAdaptor_FClass2d(probe.Face(), tolerance)
            .PerformInfinitePoint() == TopAbs_IN) {
      wire.Reverse();
      if (w == 0)
        result.outerSign = -1.;
    }
    BRepBuilderAPI_MakeFace positive(plane(f), wire, true);
    if (!positive.IsDone() || !BRepCheck_Analyzer(positive.Face()).IsValid())
      throw std::runtime_error("Exact bounded face is invalid");
    wires.push_back(wire);
    bounded.push_back(positive.Face());
  }
  for (size_t w = 1; w < bounded.size(); ++w) {
    const auto p =
        curves[offsets[w]]->Value(curves[offsets[w]]->FirstParameter());
    if (BRepClass_FaceClassifier(bounded[0], p, tolerance).State() != TopAbs_IN)
      throw std::runtime_error(
          "Profile hole must lie strictly inside the outer boundary");
    for (size_t other = 1; other < bounded.size(); ++other)
      if (other != w) {
        if (BRepClass_FaceClassifier(bounded[other], p, tolerance).State() !=
            TopAbs_OUT)
          throw std::runtime_error(
              "Nested or touching holes require separate regions");
      }
  }
  BRepBuilderAPI_MakeFace face(plane(f), wires[0], true);
  for (size_t w = 1; w < wires.size(); ++w) {
    wires[w].Reverse();
    face.Add(wires[w]);
  }
  if (!face.IsDone() || !BRepCheck_Analyzer(face.Face()).IsValid())
    throw std::runtime_error("Exact curved profile face is invalid");
  result.face = face.Face();
  return result;
}
template <class Sweep>
std::unique_ptr<Shape> sweepShape(Sweep &sweep, const FaceBuild &face,
                                  bool caps) {
  if (!sweep.IsDone() || sweep.Shape().IsNull() ||
      !BRepCheck_Analyzer(sweep.Shape()).IsValid())
    throw std::runtime_error("Exact profile sweep is invalid");
  auto result = std::make_unique<Shape>(Shape{sweep.Shape(), {}, {}});
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(result->value, TopAbs_FACE, faces);
  auto seed = [&](const TopoDS_Shape &shape, uint8_t role, uint32_t source) {
    const auto index = faces.FindIndex(shape);
    if (index > 0)
      result->seeds.push_back(
          NativeSeed{static_cast<uint32_t>(index - 1), source, role});
  };
  if (caps) {
    seed(sweep.FirstShape(), 0, 0);
    seed(sweep.LastShape(), 1, 0);
  }
  for (const auto &source : face.sources)
    for (const auto &generated : sweep.Generated(source.first))
      seed(generated, 2, source.second);
  return result;
}
} // namespace
CurveSampleData curve_evaluate(const CurvedEdgeData &edge, double parameter) {
  return guarded([&] {
    auto c = curve(edge);
    if (!std::isfinite(parameter) || parameter < c->FirstParameter() ||
        parameter > c->LastParameter())
      throw std::runtime_error("Curve parameter is outside its native domain");
    gp_Pnt2d p;
    gp_Vec2d d;
    c->D1(parameter, p, d);
    return CurveSampleData{data(p), Point3{d.X(), d.Y(), 0.}};
  });
}
CurveQueryData curve_intersections(const CurvedEdgeData &first,
                                   const CurvedEdgeData &second,
                                   double tolerance, bool self) {
  return guarded([&] {
    if (!std::isfinite(tolerance) || tolerance < 1e-7 || tolerance > 1e-3)
      throw std::runtime_error("Invalid query tolerance");
    if (self && first.kind != 1)
      return CurveQueryData{};
    return query(curve(first), curve(second), tolerance, self);
  });
}
double curved_profile_area(const Frame3 &f,
                           rust::Slice<const CurvedEdgeData> edges,
                           rust::Slice<const uint32_t> offsets,
                           double tolerance) {
  return guarded([&] {
    auto result = build(f, edges, offsets, tolerance);
    GProp_GProps area;
    BRepGProp::SurfaceProperties(result.face, area);
    return area.Mass();
  });
}
double curved_profile_signed_area(const Frame3 &f,
                                  rust::Slice<const CurvedEdgeData> edges,
                                  rust::Slice<const uint32_t> offsets,
                                  double tolerance) {
  return guarded([&] {
    auto result = build(f, edges, offsets, tolerance);
    GProp_GProps area;
    BRepGProp::SurfaceProperties(result.face, area);
    return area.Mass() * result.outerSign;
  });
}
uint8_t curved_profile_classify(const Frame3 &f,
                                rust::Slice<const CurvedEdgeData> edges,
                                rust::Slice<const uint32_t> offsets,
                                double tolerance, const Point3 &point) {
  return guarded([&] {
    auto result = build(f, edges, offsets, tolerance);
    const auto state =
        BRepClass_FaceClassifier(result.face, p2(point), tolerance).State();
    if (state == TopAbs_IN)
      return uint8_t{2};
    if (state == TopAbs_ON)
      return uint8_t{1};
    if (state == TopAbs_OUT)
      return uint8_t{0};
    throw std::runtime_error("Exact face classification is unresolved");
  });
}
std::unique_ptr<Shape>
curved_profile_extrude_vector(const Frame3 &f,
                              rust::Slice<const CurvedEdgeData> edges,
                              rust::Slice<const uint32_t> offsets,
                              double tolerance, const Point3 &displacement) {
  return guarded([&] {
    auto face = build(f, edges, offsets, tolerance);
    BRepPrimAPI_MakePrism prism(
        face.face, gp_Vec(displacement.x, displacement.y, displacement.z));
    return sweepShape(prism, face, true);
  });
}
std::unique_ptr<Shape>
curved_profile_revolve(const Frame3 &f, rust::Slice<const CurvedEdgeData> edges,
                       rust::Slice<const uint32_t> offsets, double tolerance,
                       const Point3 &origin, const Point3 &direction,
                       double angle) {
  return guarded([&] {
    auto face = build(f, edges, offsets, tolerance);
    const double sign = angle < 0. ? -1. : 1.;
    gp_Ax1 axis(
        gp_Pnt(origin.x, origin.y, origin.z),
        gp_Dir(sign * direction.x, sign * direction.y, sign * direction.z));
    BRepPrimAPI_MakeRevol sweep(face.face, axis, std::abs(angle), true);
    return sweepShape(sweep, face, std::abs(angle) != 2. * std::acos(-1.));
  });
}
} // namespace multicad::kernel
