#include "planar_contours.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepTools.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <BRep_Tool.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_MapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Wire.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Pln.hxx>
#include <gp_Circ.hxx>
#include <cmath>
#include <stdexcept>
namespace multicad::kernel {
static gp_Pnt p(const Point3 &p) {return {p.x,p.y,p.z};}
static gp_Vec v(const Point3 &p) {return {p.x,p.y,p.z};}
static Point3 local(const gp_Pnt &a,const Frame3 &frame) {
  const gp_Vec delta(p(frame.origin),a);
  return {delta.Dot(v(frame.u)),delta.Dot(v(frame.v)),0.};
}
rust::Vec<PlanarWireData> planar_face_contours(const Shape &shape,uint32_t index,const Frame3 &frame,double tolerance) {
  try {
    TopTools_IndexedMapOfShape faces;TopExp::MapShapes(shape.value,TopAbs_FACE,faces);
    TopTools_IndexedMapOfShape sourceEdges;TopExp::MapShapes(shape.value,TopAbs_EDGE,sourceEdges);
    if(index>=static_cast<uint32_t>(faces.Extent()))throw std::runtime_error("Selected planar face no longer exists");
    const auto face=TopoDS::Face(faces(index+1));
    BRepAdaptor_Surface surface(face);
    if(surface.GetType()!=GeomAbs_Plane)throw std::runtime_error("Manufacturing contours require an explicitly selected planar face; curved surfaces are unsupported");
    const auto plane=surface.Plane();
    const auto normal=v(frame.u).Crossed(v(frame.v));
    if(std::abs(normal.Dot(gp_Vec(plane.Axis().Direction())))<1.-1e-10 || plane.Distance(p(frame.origin))>tolerance)
      throw std::runtime_error("The chosen manufacturing frame is not on the selected face plane; no silent projection is performed");
    const auto outer=BRepTools::OuterWire(face);
    if(outer.IsNull())throw std::runtime_error("Selected face has no finite outer boundary");
    rust::Vec<PlanarWireData> result;TopTools_MapOfShape seen;
    uint32_t total=0;
    for(TopExp_Explorer wires(face,TopAbs_WIRE);wires.More();wires.Next()) {
      if(result.size()>=256)throw std::runtime_error("Planar face exceeds its wire budget");
      const auto wire=TopoDS::Wire(wires.Current());
      PlanarWireData output{};output.outer=wire.IsSame(outer);
      uint32_t expected=0;for(TopExp_Explorer edges(wire,TopAbs_EDGE);edges.More();edges.Next())++expected;
      for(BRepTools_WireExplorer ordered(wire,face);ordered.More();ordered.Next()) {
        const auto edge=ordered.Current();
        if(++total>16384)throw std::runtime_error("Planar face exceeds its exact edge budget");
        if(!seen.Add(edge))throw std::runtime_error("Selected face contains a duplicated boundary edge");
        if(BRep_Tool::Degenerated(edge))throw std::runtime_error("Selected face has a degenerate boundary; repair the source before manufacturing export");
        if(edge.Orientation()!=TopAbs_FORWARD && edge.Orientation()!=TopAbs_REVERSED)throw std::runtime_error("Internal boundary orientations are unsupported");
        BRepAdaptor_Curve curve(edge);
        double first=curve.FirstParameter(),last=curve.LastParameter();
        if(!std::isfinite(first)||!std::isfinite(last)||last<=first)throw std::runtime_error("Planar boundary has no finite trim range");
        if(edge.Orientation()==TopAbs_REVERSED)std::swap(first,last);
        const int sourceIndex=sourceEdges.FindIndex(edge);
        if(sourceIndex<1)throw std::runtime_error("Planar boundary edge has no current native topology identity");
        PlanarSegmentData data{};data.edge_index=static_cast<uint32_t>(sourceIndex-1);data.a=local(curve.Value(first),frame);data.b=local(curve.Value(last),frame);
        if(curve.GetType()==GeomAbs_Line) {data.kind=0;}
        else if(curve.GetType()==GeomAbs_Circle) {
          data.kind=1;const auto circle=curve.Circle();
          const double alignment=normal.Dot(gp_Vec(circle.Axis().Direction()));
          if(std::abs(alignment)<1.-1e-10)throw std::runtime_error("Circular boundary is not coplanar with the selected manufacturing frame");
          data.center=local(circle.Location(),frame);
          data.sweep=(last-first)*(alignment>0.?1.:-1.);
        } else throw std::runtime_error("Planar manufacturing boundary supports exact lines and circular arcs only; ellipse/spline conversion is not installed");
        output.segments.push_back(std::move(data));
      }
      if(output.segments.empty()||output.segments.size()!=expected)throw std::runtime_error("Planar boundary is disconnected or could not be ordered completely");
      result.push_back(std::move(output));
    }
    return result;
  } catch(const Standard_Failure &e) {throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native planar boundary extraction failed");}
}
}
