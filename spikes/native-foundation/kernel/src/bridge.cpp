#include "bridge.hpp"
#include "multicad-kernel-proof/src/lib.rs.h"
#include <BRepAlgoAPI_Cut.hxx>
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

namespace multicad::proof {
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
  return guard([&] {
    BRepMesh_IncrementalMesh mesher(s.value, tolerance);
    if (!mesher.IsDone())
      throw std::runtime_error("Tessellation failed");
    Mesh out;
    for (TopExp_Explorer e(s.value, TopAbs_FACE); e.More(); e.Next()) {
      auto face = TopoDS::Face(e.Current());
      TopLoc_Location location;
      auto tri = BRep_Tool::Triangulation(face, location);
      if (tri.IsNull())
        throw std::runtime_error("Face missing triangulation");
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
    if (writer.Transfer(s.value, STEPControl_AsIs) != IFSelect_RetDone ||
        writer.Write(std::string(path).c_str()) != IFSelect_RetDone)
      throw std::runtime_error("STEP write failed");
  });
}
std::unique_ptr<Shape> read_step(rust::Str path) {
  return guard([&] {
    STEPControl_Reader reader;
    if (reader.ReadFile(std::string(path).c_str()) != IFSelect_RetDone ||
        reader.TransferRoots() == 0)
      throw std::runtime_error("STEP read failed");
    return checked(reader.OneShape());
  });
}
} // namespace multicad::proof
