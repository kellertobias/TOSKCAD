#include "bridge.hpp"
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
#include <BRepAdaptor_Surface.hxx>
#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <StepData_StepModel.hxx>
#include <gp_Circ.hxx>
#include <gp_Ax3.hxx>
#include <gp_Trsf.hxx>
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
#include <TopoDS.hxx>
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
  return std::make_unique<Shape>(Shape{s});
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
    BRepGProp::VolumeProperties(s.value, p);
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
    for (TopExp_Explorer e(display, TopAbs_FACE); e.More(); e.Next()) {
      auto face = TopoDS::Face(e.Current());
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
      }
    }
    return out;
  });
}
void write_step(const Shape &s, rust::Str path) {
  guard([&] {
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
    return checked(prism.Shape());
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
    return checked(result);
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
    return checked(operation.Shape());
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
    uint32_t index = 0;
    for (TopExp_Explorer e(s.value, TopAbs_FACE); e.More(); e.Next()) {
      auto face = TopoDS::Face(e.Current());
      BRepAdaptor_Surface surface(face, true);
      GProp_GProps p;
      BRepGProp::SurfaceProperties(face, p);
      FaceInfo info{index++, false, Point3{0, 0, 0}, Point3{0, 0, 0},
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
} // namespace multicad::kernel
