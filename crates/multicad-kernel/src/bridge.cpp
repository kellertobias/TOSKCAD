#include "bridge.hpp"
#include "measure.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_BooleanOperation.hxx>
#include <gp_Pln.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepBuilderAPI_Copy.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <GCPnts_QuasiUniformDeflection.hxx>
#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <StepData_StepModel.hxx>
#include <gp_Circ.hxx>
#include <gp_Ax3.hxx>
#include <gp_Trsf.hxx>
#include <gp_GTrsf.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <STEPControl_Reader.hxx>
#include <STEPControl_Writer.hxx>
#include <Standard_Failure.hxx>
#include <TopExp_Explorer.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopoDS.hxx>
#include <array>
#include <mutex>
#include <set>
#include <stdexcept>

namespace multicad::kernel {
// Standard_Failure is translated explicitly; no native exception crosses FFI.
template <class F> auto guard(F fn) -> decltype(fn()) {
  try {
    return fn();
  } catch (const Standard_Failure &e) {
    throw std::runtime_error(e.GetMessageString() ? e.GetMessageString()
                                                  : "OCCT operation failed");
  }
}
static std::unique_ptr<Shape> checked(const TopoDS_Shape &s) {
  if (s.IsNull() || !BRepCheck_Analyzer(s).IsValid())
    throw std::runtime_error("OCCT produced an invalid shape");
  return std::make_unique<Shape>(Shape{s, {}, {}});
}
static const std::array<TopAbs_ShapeEnum, 3> topologyKinds = {
    TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX};
static TopTools_IndexedMapOfShape topology_map(const TopoDS_Shape &s,
                                             uint8_t kind) {
  TopTools_IndexedMapOfShape map;
  TopExp::MapShapes(s, topologyKinds.at(kind), map);
  if (map.Extent() > 100000)
    throw std::runtime_error("Topology budget exceeded");
  return map;
}
// These ordinals live only within this result. Rust assigns persistent source
// intent and follows these exact OCCT history relations; no geometric matching.
template <class Operation>
static void record_history(Shape &result, const Shape &input, uint8_t operand,
                           Operation &operation) {
  for (uint8_t kind = 0; kind < 3; ++kind) {
    auto in = topology_map(input.value, kind);
    auto out = topology_map(result.value, kind);
    for (int i = 1; i <= in.Extent(); ++i) {
      std::set<int> descendants;
      auto append = [&](const TopoDS_Shape &candidate) {
        auto index = out.FindIndex(candidate);
        if (index > 0)
          descendants.insert(index);
      };
      append(in(i));
      for (const auto &candidate : operation.Modified(in(i))) append(candidate);
      for (const auto &candidate : operation.Generated(in(i))) append(candidate);
      for (auto index : descendants) {
        if (result.ancestry.size() >= 1000000)
          throw std::runtime_error("Topology history budget exceeded");
        result.ancestry.push_back(NativeAncestry{kind, operand,
            static_cast<uint32_t>(index - 1), static_cast<uint32_t>(i - 1)});
      }
    }
  }
}
std::unique_ptr<Shape> extrude(double w, double d, double h) {
  return guard([&] {
    BRepBuilderAPI_MakePolygon polygon;
    polygon.Add(gp_Pnt(0, 0, 0));
    polygon.Add(gp_Pnt(w, 0, 0));
    polygon.Add(gp_Pnt(w, d, 0));
    polygon.Add(gp_Pnt(0, d, 0));
    polygon.Close();
    BRepBuilderAPI_MakeFace face(polygon.Wire());
    BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(0, 0, h));
    if (!prism.IsDone())
      throw std::runtime_error("Extrusion failed");
    return checked(prism.Shape());
  });
}
std::unique_ptr<Shape> cut(const Shape &shape, double x, double w, double d,
                           double h) {
  return guard([&] {
    auto tool = BRepPrimAPI_MakeBox(gp_Pnt(x, 0, 0), w, d, h).Shape();
    BRepAlgoAPI_Cut operation(shape.value, tool);
    if (!operation.IsDone())
      throw std::runtime_error("Boolean failed");
    return checked(operation.Shape());
  });
}
bool valid(const Shape &s) {
  return guard([&] {
    return !s.value.IsNull() && BRepCheck_Analyzer(s.value).IsValid();
  });
}
double volume(const Shape &s) {
  return guard([&] {
    GProp_GProps p;
    // Clipped curved faces need adaptive integration; the default overload
    // uses a fixed rule and can lose visible precision on valid exact B-reps.
    // Gauss-Kronrod reports whole-solid relative error, avoiding false failures
    // from near-zero individual face contributions. Spline spans are enabled.
    const double relativeError = BRepGProp::VolumePropertiesGK(s.value, p, 1e-10, true, true);
    if (!std::isfinite(relativeError) || relativeError < 0. || relativeError > 1e-7)
      throw std::runtime_error("Native volume integration did not converge");
    return p.Mass();
  });
}
Mesh mesh(const Shape &s, double tolerance) {
  return tessellate(s, tolerance, 0.5, 1000000);
}
Mesh tessellate(const Shape &s, double tolerance, double angle, uint32_t limit) {
  return guard([&] {
    // Meshing mutates triangulations. Detach geometry so a display request
    // cannot modify a cached modeling operand or its manufacturing geometry.
    BRepBuilderAPI_Copy copy(s.value, true, false);
    auto display = copy.Shape();
    BRepMesh_IncrementalMesh mesher(display, tolerance, false, angle, false);
    if (!mesher.IsDone())
      throw std::runtime_error("Tessellation failed");
    Mesh out;
    uint64_t triangles = 0;
    auto exactFaces = topology_map(s.value, 0);
    for (int faceIndex = 1; faceIndex <= exactFaces.Extent(); ++faceIndex) {
      auto face = TopoDS::Face(copy.ModifiedShape(exactFaces(faceIndex)));
      TopLoc_Location location;
      auto tri = BRep_Tool::Triangulation(face, location);
      if (tri.IsNull())
        throw std::runtime_error("Face missing triangulation");
      triangles += tri->NbTriangles();
      if (triangles > limit || out.vertices.size() / 3 + tri->NbNodes() > UINT32_MAX)
        throw std::runtime_error("Display mesh exceeds triangle/index budget");
      const auto offset = static_cast<uint32_t>(out.vertices.size() / 3);
      for (int i = 1; i <= tri->NbNodes(); ++i) {
        auto p = tri->Node(i).Transformed(location.Transformation());
        out.vertices.push_back(p.X());
        out.vertices.push_back(p.Y());
        out.vertices.push_back(p.Z());
      }
      for (int i = 1; i <= tri->NbTriangles(); ++i) {
        int a, b, c;
        tri->Triangle(i).Get(a, b, c);
        if (face.Orientation() == TopAbs_REVERSED)
          std::swap(b, c);
        out.indices.push_back(offset + a - 1);
        out.indices.push_back(offset + b - 1);
        out.indices.push_back(offset + c - 1);
        out.face_indices.push_back(static_cast<uint32_t>(faceIndex - 1));
      }
    }
    return out;
  });
}
// STEP reader/writer initialization and shape-fix parameters use shared OCCT
// translator state. Acquire before constructing either translator and retain
// through destruction; unrelated geometry and owned handles remain independent.
static std::mutex step_exchange_mutex;
void write_step(const Shape &s, rust::Str path) {
  guard([&] {
    const std::lock_guard<std::mutex> exchange_lock(step_exchange_mutex);
    STEPControl_Writer writer;
    writer.Model()->SetLocalLengthUnit(1.0);
    writer.Model()->SetWriteLengthUnit(1.0);
    if (writer.Transfer(s.value, STEPControl_AsIs) != IFSelect_RetDone ||
        writer.Write(std::string(path).c_str()) != IFSelect_RetDone)
      throw std::runtime_error("STEP write failed");
  });
}
std::unique_ptr<Shape> read_step(rust::Str path) {
  return guard([&] {
    const std::lock_guard<std::mutex> exchange_lock(step_exchange_mutex);
    STEPControl_Reader reader;
    if (reader.ReadFile(std::string(path).c_str()) != IFSelect_RetDone)
      throw std::runtime_error("STEP read failed");
    reader.SetSystemLengthUnit(1.0);
    if (reader.TransferRoots() == 0)
      throw std::runtime_error("STEP contains no transferable geometry");
    return checked(reader.OneShape());
  });
}
static gp_Pnt point(const Point3 &p) { return gp_Pnt(p.x, p.y, p.z); }
static gp_Vec vec(const Point3 &p) { return gp_Vec(p.x, p.y, p.z); }
static Point3 data(const gp_Pnt &p) { return Point3{p.X(), p.Y(), p.Z()}; }
static gp_Ax2 axes(const Frame3 &f) {
  return gp_Ax2(point(f.origin), gp_Dir(vec(f.u).Crossed(vec(f.v))),
                gp_Dir(vec(f.u)));
}
std::unique_ptr<Shape> cylinder(const Frame3 &f, double radius, double height) {
  return guard([&] {
    return checked(BRepPrimAPI_MakeCylinder(axes(f), radius, height).Shape());
  });
}
std::unique_ptr<Shape> profile_extrude(
    const Frame3 &f, rust::Slice<const CurveEdge> edges,
    rust::Slice<const uint32_t> offsets, double height) {
  return guard([&] {
    if (offsets.size() < 2 || offsets[0] != 0 ||
        offsets[offsets.size() - 1] != edges.size())
      throw std::runtime_error("Invalid profile wire offsets");
    BRepBuilderAPI_MakeFace face;
    std::vector<std::pair<TopoDS_Edge, uint32_t>> sourceEdges;
    for (size_t wireIndex = 0; wireIndex + 1 < offsets.size(); ++wireIndex) {
      if (offsets[wireIndex] >= offsets[wireIndex + 1] ||
          offsets[wireIndex + 1] > edges.size())
        throw std::runtime_error("Empty/invalid profile wire");
      BRepBuilderAPI_MakeWire wire;
      for (uint32_t i = offsets[wireIndex]; i < offsets[wireIndex + 1]; ++i) {
        const auto &e = edges[i];
        if (e.kind == 0) {
          wire.Add(BRepBuilderAPI_MakeEdge(point(e.a), point(e.b)).Edge());
        } else if (e.kind == 1) {
          GC_MakeArcOfCircle arc(point(e.a), point(e.mid), point(e.b));
          if (!arc.IsDone())
            throw std::runtime_error("Arc construction failed");
          wire.Add(BRepBuilderAPI_MakeEdge(arc.Value()).Edge());
        } else if (e.kind == 2) {
          auto ax = axes(f);
          ax.SetLocation(point(e.a));
          wire.Add(BRepBuilderAPI_MakeEdge(gp_Circ(ax, e.radius)).Edge());
        } else {
          throw std::runtime_error("Unknown profile edge");
        }
        if (!wire.IsDone())
          throw std::runtime_error("Profile edges are not connected");
        sourceEdges.emplace_back(wire.Edge(), e.source_index);
      }
      auto w = wire.Wire();
      if (wireIndex == 0) {
        face = BRepBuilderAPI_MakeFace(gp_Pln(gp_Ax3(axes(f))), w, true);
      } else {
        w.Reverse();
        face.Add(w);
      }
    }
    if (!face.IsDone() || !BRepCheck_Analyzer(face.Face()).IsValid())
      throw std::runtime_error(
          "Profile is invalid, self-intersecting, or has invalid holes");
    BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(axes(f).Direction()) * height);
    if (!prism.IsDone())
      throw std::runtime_error("Profile extrusion failed");
    auto result = checked(prism.Shape());
    auto faces = topology_map(result->value, 0);
    auto seed = [&](const TopoDS_Shape &shape, uint8_t role, uint32_t index) {
      const auto faceIndex = faces.FindIndex(shape);
      if (faceIndex > 0)
        result->seeds.push_back(NativeSeed{
            static_cast<uint32_t>(faceIndex - 1), index, role});
    };
    seed(prism.FirstShape(), 0, 0);
    seed(prism.LastShape(), 1, 0);
    for (const auto &entry : sourceEdges)
      for (const auto &generated : prism.Generated(entry.first))
        seed(generated, 2, entry.second);
    return result;
  });
}
std::unique_ptr<Shape> boolean_shapes(const Shape &a, const Shape &b,
                                      uint8_t operation, double fuzzy) {
  return guard([&]() -> std::unique_ptr<Shape> {
    BRepAlgoAPI_BooleanOperation op;
    TopTools_ListOfShape arguments, tools;
    arguments.Append(a.value);
    tools.Append(b.value);
    op.SetArguments(arguments);
    op.SetTools(tools);
    if (operation == 0)
      op.SetOperation(BOPAlgo_FUSE);
    else if (operation == 1)
      op.SetOperation(BOPAlgo_CUT);
    else if (operation == 2)
      op.SetOperation(BOPAlgo_COMMON);
    else
      throw std::runtime_error("Unknown Boolean operation");
    op.SetNonDestructive(true);
    op.SetRunParallel(false);
    op.SetFuzzyValue(fuzzy);
    op.Build();
    if (!op.IsDone() || op.HasErrors())
      throw std::runtime_error("Solid Boolean failed");
    auto result = op.Shape();
    if (!TopExp_Explorer(result, TopAbs_SOLID).More())
      return nullptr;
    auto owned = checked(result);
    record_history(*owned, a, 0, op);
    record_history(*owned, b, 1, op);
    return owned;
  });
}
std::unique_ptr<Shape> transform_shape(const Shape &s,
                                     rust::Slice<const double> r,
                                     const Point3 &t) {
  return guard([&] {
    if (r.size() != 9)
      throw std::runtime_error("Rotation requires nine coefficients");
    gp_Trsf tr;
    tr.SetValues(r[0], r[1], r[2], t.x, r[3], r[4], r[5], t.y,
                 r[6], r[7], r[8], t.z);
    BRepBuilderAPI_Transform operation(s.value, tr, true);
    if (!operation.IsDone())
      throw std::runtime_error("Rigid transform failed");
    auto result = checked(operation.Shape());
    record_history(*result, s, 0, operation);
    return result;
  });
}
std::unique_ptr<Shape> affine_transform_shape(const Shape &s,
                                            rust::Slice<const double> m,
                                            const Point3 &t) {
  return guard([&] {
    if (m.size() != 9)
      throw std::runtime_error("Affine transform requires nine coefficients");
    // Only literal scalar matrices take this path. Near-uniform or conjugated
    // matrices keep every supplied coefficient in the general transform.
    if (m[0] == m[4] && m[4] == m[8] && m[1] == 0. && m[2] == 0. &&
        m[3] == 0. && m[5] == 0. && m[6] == 0. && m[7] == 0.) {
      gp_Trsf tr;
      tr.SetScale(gp::Origin(), m[0]);
      tr.SetTranslationPart(gp_Vec(t.x, t.y, t.z));
      BRepBuilderAPI_Transform operation(s.value, tr, true);
      if (!operation.IsDone()) throw std::runtime_error("Uniform affine transform failed");
      auto result = checked(operation.Shape());
      record_history(*result, s, 0, operation);
      return result;
    }
    gp_GTrsf tr(gp_Mat(m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]),
                gp_XYZ(t.x, t.y, t.z));
    if (tr.IsNegative() || tr.IsSingular())
      throw std::runtime_error("Affine transform is singular or reflects the solid");
    BRepBuilderAPI_GTransform operation(s.value, tr, true);
    if (!operation.IsDone()) throw std::runtime_error("General affine transform failed");
    auto result = checked(operation.Shape());
    // OCCT 7.9.3 GTransform::Modified() does not expose its collected list.
    // Its ModifiedShape() composes the exact NURBS-conversion and transform
    // histories. Affine maps preserve topology; require a complete bijection.
    for (uint8_t kind = 0; kind < 3; ++kind) {
      const auto input = topology_map(s.value, kind);
      const auto output = topology_map(result->value, kind);
      std::set<int> mapped;
      for (int i = 1; i <= input.Extent(); ++i) {
        const auto modified = operation.ModifiedShape(input(i));
        const int index = output.FindIndex(modified);
        if (index <= 0 || !mapped.insert(index).second)
          throw std::runtime_error("Affine transform lacks unique exact topology ancestry");
        result->ancestry.push_back(NativeAncestry{kind, 0,
            static_cast<uint32_t>(index - 1), static_cast<uint32_t>(i - 1)});
      }
      if (mapped.size() != static_cast<size_t>(output.Extent()))
        throw std::runtime_error("Affine transform added unsupported topology");
    }
    return result;
  });
}
Bounds3 bounds(const Shape &s) {
  return guard([&] {
    Bnd_Box b;
    BRepBndLib::AddOptimal(s.value, b, false, false);
    if (b.IsVoid() || b.IsOpen())
      throw std::runtime_error("Shape has no finite bounds");
    double x, y, z, a, c, d;
    b.Get(x, y, z, a, c, d);
    return Bounds3{Point3{x, y, z}, Point3{a, c, d}};
  });
}
uint32_t solid_count(const Shape &s) {
  return guard([&] {
    uint32_t count = 0;
    for (TopExp_Explorer e(s.value, TopAbs_SOLID); e.More(); e.Next())
      ++count;
    return count;
  });
}
rust::Vec<FaceInfo> faces(const Shape &s) {
  return guard([&] {
    rust::Vec<FaceInfo> out;
    auto map = topology_map(s.value, 0);
    for (int index = 1; index <= map.Extent(); ++index) {
      auto face = TopoDS::Face(map(index));
      BRepAdaptor_Surface surface(face, true);
      const auto p = measured_face(face);
      FaceInfo info{static_cast<uint32_t>(index - 1), false, Point3{0, 0, 0}, Point3{0, 0, 0},
                    data(p.CentreOfMass()), p.Mass()};
      if (surface.GetType() == GeomAbs_Plane) {
        auto plane = surface.Plane();
        auto n = plane.Axis().Direction();
        if (face.Orientation() == TopAbs_REVERSED)
          n.Reverse();
        info.planar = true;
        info.origin = data(plane.Location());
        info.normal = Point3{n.X(), n.Y(), n.Z()};
      }
      out.push_back(info);
    }
    return out;
  });
}
rust::Vec<TopologyNode> topology(const Shape &s) {
  return guard([&] {
    rust::Vec<TopologyNode> result;
    auto faces = topology_map(s.value, 0);
    TopTools_IndexedDataMapOfShapeListOfShape edgeFaces, vertexFaces;
    TopExp::MapShapesAndAncestors(s.value, TopAbs_EDGE, TopAbs_FACE, edgeFaces);
    TopExp::MapShapesAndAncestors(s.value, TopAbs_VERTEX, TopAbs_FACE, vertexFaces);
    for (uint8_t kind = 0; kind < 3; ++kind) {
      auto elements = topology_map(s.value, kind);
      for (int i = 1; i <= elements.Extent(); ++i) {
        std::set<uint32_t> incident;
        auto append = [&](const TopTools_ListOfShape &list) {
          for (const auto &face : list) {
            auto index = faces.FindIndex(face);
            if (index > 0 && !(kind == 0 && index == i))
              incident.insert(static_cast<uint32_t>(index - 1));
          }
        };
        if (kind == 0) {
          for (TopExp_Explorer e(elements(i), TopAbs_EDGE); e.More(); e.Next())
            if (edgeFaces.Contains(e.Current())) append(edgeFaces.FindFromKey(e.Current()));
        } else {
          const auto &ancestors = kind == 1 ? edgeFaces : vertexFaces;
          if (ancestors.Contains(elements(i))) append(ancestors.FindFromKey(elements(i)));
        }
        TopologyNode node{kind, static_cast<uint32_t>(i - 1), {}};
        for (auto index : incident) node.incident_faces.push_back(index);
        result.push_back(std::move(node));
      }
    }
    return result;
  });
}
rust::Vec<Ancestry> ancestry(const Shape &s) {
  rust::Vec<Ancestry> result;
  for (const auto &entry : s.ancestry)
    result.push_back(Ancestry{entry.kind, entry.operand, entry.output, entry.input});
  return result;
}
rust::Vec<Seed> seeds(const Shape &s) {
  rust::Vec<Seed> result;
  for (const auto &entry : s.seeds)
    result.push_back(Seed{entry.face, entry.source_index, entry.role});
  return result;
}
rust::Vec<EdgeInfo> display_edges(const Shape &s, double deflection) {
  return guard([&] {
    rust::Vec<EdgeInfo> result;
    auto edges = topology_map(s.value, 1);
    uint64_t points = 0;
    for (int i = 1; i <= edges.Extent(); ++i) {
      auto edge = TopoDS::Edge(edges(i));
      if (BRep_Tool::Degenerated(edge)) continue;
      BRepAdaptor_Curve curve(edge);
      GCPnts_QuasiUniformDeflection sample(curve, deflection);
      if (!sample.IsDone() || sample.NbPoints() < 2)
        throw std::runtime_error("B-rep edge display sampling failed");
      points += sample.NbPoints();
      if (points > 1000000)
        throw std::runtime_error("B-rep edge display budget exceeded");
      EdgeInfo info{static_cast<uint32_t>(i - 1), {}};
      for (int p = 1; p <= sample.NbPoints(); ++p) {
        auto v = sample.Value(p);
        info.points.push_back(v.X()); info.points.push_back(v.Y()); info.points.push_back(v.Z());
      }
      result.push_back(std::move(info));
    }
    return result;
  });
}
rust::Vec<VertexInfo> vertices(const Shape &s) {
  return guard([&] {
    rust::Vec<VertexInfo> result;
    auto vertices = topology_map(s.value, 2);
    for (int i = 1; i <= vertices.Extent(); ++i)
      result.push_back(VertexInfo{static_cast<uint32_t>(i - 1),
          data(BRep_Tool::Pnt(TopoDS::Vertex(vertices(i))))});
    return result;
  });
}
} // namespace multicad::kernel
