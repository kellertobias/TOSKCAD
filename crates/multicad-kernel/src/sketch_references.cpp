#include "sketch_references.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Section.hxx>
#include <BRep_Tool.hxx>
#include <GeomAPI_IntCS.hxx>
#include <GeomAdaptor_Curve.hxx>
#include <GeomConvert.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_Plane.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Vertex.hxx>
#include <algorithm>
#include <cmath>
#include <stdexcept>
#include <vector>

namespace multicad::kernel {
namespace {
constexpr size_t maxResults=256,maxPoles=256;
constexpr double tau=6.2831853071795864769;
gp_Pnt p(const Point3 &a){return {a.x,a.y,a.z};}
gp_Vec v(const Point3 &a){return {a.x,a.y,a.z};}
struct Query {
  const Shape &source;
  const Frame3 &frame;
  const double tolerance;
  gp_Pln plane;
  gp_Vec normal;
  TopTools_IndexedMapOfShape faces,edges,vertices;
  SketchReferenceCatalogData result{};
  Query(const Shape &s,const Frame3 &f,double tol):source(s),frame(f),tolerance(tol),
    plane(gp_Ax3(p(f.origin),gp_Dir(v(f.u).Crossed(v(f.v))),gp_Dir(v(f.u)))),normal(v(f.u).Crossed(v(f.v))) {
    TopExp::MapShapes(source.value,TopAbs_FACE,faces);
    TopExp::MapShapes(source.value,TopAbs_EDGE,edges);
    TopExp::MapShapes(source.value,TopAbs_VERTEX,vertices);
    if(faces.Extent()+edges.Extent()+vertices.Extent()>100000)throw std::runtime_error("Sketch reference topology budget exceeded");
  }
  double height(const gp_Pnt &a)const{return gp_Vec(p(frame.origin),a).Dot(normal);}
  Point3 local(const gp_Pnt &a)const {const gp_Vec delta(p(frame.origin),a);return {delta.Dot(v(frame.u)),delta.Dot(v(frame.v)),0.};}
  void add(SketchReferenceData data){if(result.results.size()>=maxResults)throw std::runtime_error("Sketch reference result budget exceeded");result.results.push_back(std::move(data));}
  void support(SketchReferenceData &data,uint8_t kind,uint32_t index){
    for(const auto &s:data.support)if(s.kind==kind&&s.index==index)return;
    data.support.push_back(SketchSupportData{kind,index});
  }
  void support_shape(SketchReferenceData &data,const TopoDS_Shape &s) {
    const int f=faces.FindIndex(s),e=edges.FindIndex(s),vert=vertices.FindIndex(s);
    if(f) support(data,0,f-1);
    if(e) support(data,1,e-1);
    if(vert) support(data,2,vert-1);
  }
  Handle(Geom_Curve) finite_curve(const TopoDS_Edge &edge,double &first,double &last){
    if(BRep_Tool::Degenerated(edge))throw std::runtime_error("Selected finite edge is degenerate");
    const auto c=BRep_Tool::Curve(edge,first,last);
    if(c.IsNull()||!std::isfinite(first)||!std::isfinite(last)||last<=first)throw std::runtime_error("Source edge has no finite exact 3D curve");
    const auto type=GeomAdaptor_Curve(c,first,last).GetType();
    if(type!=GeomAbs_Line&&type!=GeomAbs_Circle&&type!=GeomAbs_Ellipse&&type!=GeomAbs_BSplineCurve&&type!=GeomAbs_BezierCurve)throw std::runtime_error("Source edge family is unsupported; no fitted or sampled curve is substituted");
    return c;
  }
  Handle(Geom_BSplineCurve) rational(const Handle(Geom_Curve)&curve,double first,double last){
    Handle(Geom_TrimmedCurve) trimmed=new Geom_TrimmedCurve(curve,first,last);
    auto spline=GeomConvert::CurveToBSplineCurve(trimmed);
    spline=Handle(Geom_BSplineCurve)::DownCast(spline->Copy());
    if(spline->IsPeriodic())spline->SetNotPeriodic();
    spline->Segment(spline->FirstParameter(),spline->LastParameter(),0.);
    if(spline->NbPoles()>static_cast<int>(maxPoles)||spline->NbKnots()>static_cast<int>(maxPoles+1))throw std::runtime_error("Exact projected spline exceeds its basis budget");
    return spline;
  }
  bool coplanar(const Handle(Geom_Curve)&c,double first,double last){
    GeomAdaptor_Curve a(c,first,last);
    if(a.GetType()==GeomAbs_Line)return std::abs(height(a.Value(first)))<=tolerance&&std::abs(height(a.Value(last)))<=tolerance;
    if(a.GetType()==GeomAbs_Circle) {
      const auto circle=a.Circle();
      return std::abs(height(circle.Location()))+circle.Radius()*normal.Crossed(gp_Vec(circle.Axis().Direction())).Magnitude()<=tolerance;
    }
    const auto spline=rational(c,first,last);
    for(int i=1;i<=spline->NbPoles();++i)if(std::abs(height(spline->Pole(i)))>tolerance)return false;
    return true;
  }
  // Affine projection of the exact rational poles preserves the curve locus.
  // GeomProjLib's generic plane projection has approximation branches and is
  // intentionally not used as reference/manufacturing geometry authority.
  SketchReferenceData curve_data(const TopoDS_Edge &edge,bool requireCoplanar) {
    double first,last;const auto c=finite_curve(edge,first,last);
    if(requireCoplanar&&!coplanar(c,first,last))throw std::runtime_error("Selected finite edge is not coplanar with the sketch plane");
    SketchReferenceData out{};out.geometry=1;out.has_interval=true;out.first=first;out.last=last;
    GeomAdaptor_Curve a(c,first,last);
    out.curve.a=local(a.Value(first));out.curve.b=local(a.Value(last));
    if(a.GetType()==GeomAbs_Line){
      if(std::hypot(out.curve.a.x-out.curve.b.x,out.curve.a.y-out.curve.b.y)<=tolerance){out.geometry=0;out.point=out.curve.a;}
      else out.curve.kind=0;
    } else if(a.GetType()==GeomAbs_Circle&&normal.Crossed(gp_Vec(a.Circle().Axis().Direction())).Magnitude()<=1e-14){
      out.curve.center=local(a.Circle().Location());out.curve.radius=a.Circle().Radius();
      out.curve.clockwise=normal.Dot(gp_Vec(a.Circle().Axis().Direction()))<0.;
      out.curve.kind=std::abs(last-first-tau)<=1e-9?3:2;
    } else {
      const auto spline=rational(c,first,last);out.curve.kind=1;out.curve.spline.degree=spline->Degree();
      for(int i=1;i<=spline->NbPoles();++i){const auto pole=local(spline->Pole(i));out.curve.spline.poles.push_back(pole.x);out.curve.spline.poles.push_back(pole.y);if(spline->IsRational())out.curve.spline.weights.push_back(spline->Weight(i));}
      for(int i=1;i<=spline->NbKnots();++i){out.curve.spline.knots.push_back(spline->Knot(i));out.curve.spline.multiplicities.push_back(spline->Multiplicity(i));}
    }
    return out;
  }
  void edge_intersections(const TopoDS_Edge &edge,uint32_t index){
    double first,last;const auto c=finite_curve(edge,first,last);
    if(coplanar(c,first,last)){result.coincident=true;return;}
    Handle(Geom_Curve) finite=new Geom_TrimmedCurve(c,first,last);
    GeomAPI_IntCS hits(finite,new Geom_Plane(plane));
    if(!hits.IsDone())throw std::runtime_error("Native finite curve/plane intersection failed");
    if(hits.NbSegments()) {result.coincident=true;return;}
    if(hits.NbPoints()>static_cast<int>(maxResults))throw std::runtime_error("Native finite curve/plane contact budget exceeded");
    std::vector<double> parameters;
    for(int i=1;i<=hits.NbPoints();++i){double u,v,w;hits.Parameters(i,u,v,w);parameters.push_back(w);}
    // Endpoints are finite contacts too. Never extend a supporting curve to
    // invent an intersection beyond the authored B-rep trim.
    if(std::abs(height(c->Value(first)))<=tolerance)parameters.push_back(first);
    if(std::abs(height(c->Value(last)))<=tolerance)parameters.push_back(last);
    std::sort(parameters.begin(),parameters.end());
    std::vector<double> accepted;
    GeomAdaptor_Curve adapted(c,first,last);
    const bool closed=adapted.GetType()==GeomAbs_Circle&&std::abs(last-first-tau)<=1e-9;
    for(double w:parameters){
      if(!std::isfinite(w)||w<first-1e-12||w>last+1e-12)continue;
      w=std::clamp(w,first,last);
      if(closed&&std::abs(w-last)<=1e-12)w=first;
      if(std::any_of(accepted.begin(),accepted.end(),[&](double old){return std::abs(old-w)<=1e-11;}))continue;
      gp_Pnt position;gp_Vec derivative;c->D1(w,position,derivative);
      if(std::abs(height(position))>tolerance)throw std::runtime_error("Native plane contact exceeds the reference tolerance");
      if(derivative.Magnitude()<=1e-14)throw std::runtime_error("Native plane contact has an undefined tangent");
      const auto signedDirection=derivative.Dot(normal)/derivative.Magnitude();
      SketchReferenceData out{};out.point=local(position);out.has_parameter=true;out.parameter=w;out.has_interval=true;out.first=first;out.last=last;
      out.tangent=std::abs(signedDirection)<=1e-9;out.crossing=out.tangent?0:(signedDirection<0.?-1:1);
      support(out,1,index);add(std::move(out));accepted.push_back(w);
    }
  }
  bool coincident_face(const TopoDS_Face &face){
    const BRepAdaptor_Surface s(face);
    return s.GetType()==GeomAbs_Plane&&normal.Crossed(gp_Vec(s.Plane().Axis().Direction())).Magnitude()<=1e-14&&s.Plane().Distance(p(frame.origin))<=tolerance;
  }
  void section(const TopoDS_Shape &selected,int selectedFace){
    if(selectedFace>=0&&coincident_face(TopoDS::Face(selected))){result.coincident=true;return;}
    BRepAlgoAPI_Section op(selected,plane,false);
    op.SetNonDestructive(true);op.SetRunParallel(false);op.SetFuzzyValue(0.);op.Approximation(false);op.Build();
    if(!op.IsDone()||op.HasErrors())throw std::runtime_error("Native finite B-rep/plane section failed");
    auto supports=[&](SketchReferenceData &out,const TopoDS_Shape &produced){
      if(selectedFace>=0)support(out,0,selectedFace);
      support_shape(out,produced);
      if(produced.ShapeType()==TopAbs_EDGE){
        TopoDS_Shape ancestor;if(op.HasAncestorFaceOn1(produced,ancestor))support_shape(out,ancestor);
        TopTools_IndexedMapOfShape endpoints;TopExp::MapShapes(produced,TopAbs_VERTEX,endpoints);
        // Source edge -> generated section endpoint is exact Boolean history,
        // not a nearest-boundary test. It distinguishes hole-split sections.
        for(int i=1;i<=edges.Extent();++i){
          bool contributes=false;
          TopTools_IndexedMapOfShape originalEndpoints;TopExp::MapShapes(edges(i),TopAbs_VERTEX,originalEndpoints);
          for(int j=1;j<=originalEndpoints.Extent();++j)if(endpoints.Contains(originalEndpoints(j)))contributes=true;
          for(const auto &generated:op.Generated(edges(i)))if(endpoints.Contains(generated))contributes=true;
          for(const auto &modified:op.Modified(edges(i)))if(endpoints.Contains(modified))contributes=true;
          if(contributes)support(out,1,i-1);
        }
      }
      if(out.support.empty())for(int i=1;i<=faces.Extent();++i)for(const auto &generated:op.Generated(faces(i)))if(generated.IsSame(produced))support(out,0,i-1);
      if(out.support.empty())throw std::runtime_error("Native section has no exact source ancestry; repair or select an explicit face");
    };
    TopTools_IndexedMapOfShape sectionEdges,edgeVertices;
    TopExp::MapShapes(op.Shape(),TopAbs_EDGE,sectionEdges);
    if(sectionEdges.Extent()>static_cast<int>(maxResults))throw std::runtime_error("Native plane section exceeds its edge budget");
    for(int i=1;i<=sectionEdges.Extent();++i){const auto edge=TopoDS::Edge(sectionEdges(i));auto out=curve_data(edge,true);out.has_interval=false;supports(out,edge);add(std::move(out));TopExp::MapShapes(edge,TopAbs_VERTEX,edgeVertices);}
    TopTools_IndexedMapOfShape sectionVertices;TopExp::MapShapes(op.Shape(),TopAbs_VERTEX,sectionVertices);
    for(int i=1;i<=sectionVertices.Extent();++i)if(!edgeVertices.Contains(sectionVertices(i))){SketchReferenceData out{};out.point=local(BRep_Tool::Pnt(TopoDS::Vertex(sectionVertices(i))));out.tangent=true;supports(out,sectionVertices(i));add(std::move(out));}
  }
};
}
SketchReferenceCatalogData sketch_reference_geometry(const Shape &source,uint8_t kind,uint32_t index,const Frame3 &frame,uint8_t method,double tolerance){
  try{
    if(kind>3||method>2||!std::isfinite(tolerance)||tolerance<1e-7||tolerance>1e-3)throw std::runtime_error("Invalid sketch reference query");
    Query q(source,frame,tolerance);
    if(kind==0){if(method!=2)throw std::runtime_error("Whole-solid references support finite plane sections only; select a finite face or edge for projection");q.section(source.value,-1);}
    else if(kind==1){
      if(index>=static_cast<uint32_t>(q.faces.Extent()))throw std::runtime_error("Selected reference face is missing");
      const auto face=TopoDS::Face(q.faces(index+1));
      if(method==2)q.section(face,index);
      else {
        if(method==0&&!q.coincident_face(face))throw std::runtime_error("Selected finite face is not on the sketch plane");
        TopTools_IndexedMapOfShape boundary;TopExp::MapShapes(face,TopAbs_EDGE,boundary);
        if(boundary.Extent()>static_cast<int>(maxResults))throw std::runtime_error("Finite face boundary exceeds its reference budget");
        for(int i=1;i<=boundary.Extent();++i){const auto edge=TopoDS::Edge(boundary(i));auto out=q.curve_data(edge,method==0);q.support(out,0,index);q.support_shape(out,edge);q.add(std::move(out));}
      }
    } else if(kind==2){
      if(index>=static_cast<uint32_t>(q.edges.Extent()))throw std::runtime_error("Selected reference edge is missing");
      const auto edge=TopoDS::Edge(q.edges(index+1));
      if(method==2)q.edge_intersections(edge,index);else {auto out=q.curve_data(edge,method==0);q.support(out,1,index);q.add(std::move(out));}
    } else {
      if(index>=static_cast<uint32_t>(q.vertices.Extent()))throw std::runtime_error("Selected reference vertex is missing");
      const auto position=BRep_Tool::Pnt(TopoDS::Vertex(q.vertices(index+1)));
      if(method==0&&std::abs(q.height(position))>tolerance)throw std::runtime_error("Selected point is not on the sketch plane");
      if(method!=2||std::abs(q.height(position))<=tolerance){SketchReferenceData out{};out.point=q.local(position);q.support(out,2,index);q.add(std::move(out));}
    }
    return std::move(q.result);
  }catch(const Standard_Failure &e){throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native sketch reference query failed");}
}
}
