#include "drawing.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeHalfSpace.hxx>
#include <BRepBndLib.hxx>
#include <BRepGProp.hxx>
#include <BRepLib.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <GCPnts_QuasiUniformDeflection.hxx>
#include <Geom_BSplineCurve.hxx>
#include <HLRBRep_Algo.hxx>
#include <HLRBRep_HLRToShape.hxx>
#include <HLRAlgo_Projector.hxx>
#include <Standard_Failure.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <gp_Elips.hxx>
#include <gp_Circ.hxx>
#include <gp_Pln.hxx>
#include <cmath>
#include <stdexcept>

namespace multicad::kernel {
static gp_Pnt point(const Point3 &p) { return {p.x,p.y,p.z}; }
static gp_Dir dir(const Point3 &p) { return {p.x,p.y,p.z}; }
std::unique_ptr<DrawingJob> drawing_job(const DrawingOptionsData &o) {
  auto job=std::make_unique<DrawingJob>();
  const gp_Dir u=dir(o.frame.u),v=dir(o.frame.v);
  job->frame=gp_Ax2(point(o.frame.origin),u.Crossed(v),u);
  job->section=o.section; job->detail=o.detail; job->hidden=o.hidden; job->tangents=o.tangents;
  job->offset=o.section_offset; job->cx=o.detail_center.x; job->cy=o.detail_center.y;
  job->radius=o.detail_radius; job->tolerance=o.tolerance;
  job->max_curves=o.max_curves; job->max_points=o.max_points;
  return job;
}
void drawing_add(DrawingJob &job,const Shape &shape,bool hatch,bool cross,double angle,double spacing) {
  if(job.inputs.size()>=4096) throw std::runtime_error("Drawing source budget exceeded");
  job.inputs.push_back({shape.value,hatch,cross,angle,spacing});
}
static TopoDS_Shape common(const TopoDS_Shape &a,const TopoDS_Shape &b) {
  BRepAlgoAPI_Common op(a,b); op.SetRunParallel(false); op.Build();
  if(!op.IsDone() || op.HasErrors()) throw std::runtime_error("Drawing intersection failed");
  return op.Shape();
}
static double volume(const TopoDS_Shape &s) {GProp_GProps p;BRepGProp::VolumeProperties(s,p);return std::abs(p.Mass());}
static gp_Pnt local(const gp_Pnt &p,const gp_Ax2 &f) {
  const gp_Vec d(f.Location(),p);return {d.Dot(gp_Vec(f.XDirection())),d.Dot(gp_Vec(f.YDirection())),0.};
}
static TopoDS_Shape section_shape(const TopoDS_Shape &shape,const DrawingJob &job) {
  const auto origin=job.frame.Location().Translated(gp_Vec(job.frame.Direction())*job.offset);
  const auto face=BRepBuilderAPI_MakeFace(gp_Pln(origin,job.frame.Direction())).Face();
  auto half=BRepPrimAPI_MakeHalfSpace(face,origin.Translated(-gp_Vec(job.frame.Direction()))).Solid();
  return common(shape,half);
}
static void append_point(rust::Vec<double> &out,const gp_Pnt &p) {out.push_back(p.X());out.push_back(p.Y());}
static void append_dir(rust::Vec<double> &out,const gp_Dir &d) {out.push_back(d.X());out.push_back(d.Y());}
struct Collector {
  const DrawingJob &job;
  rust::Vec<DrawingCurveData> result;
  uint32_t points=0;
  TopoDS_Shape detail;
  explicit Collector(const DrawingJob &j):job(j) {
    if(job.detail) {
      auto edge=BRepBuilderAPI_MakeEdge(gp_Circ(gp_Ax2(gp_Pnt(job.cx,job.cy,0.),gp::DZ()),job.radius)).Edge();
      detail=BRepBuilderAPI_MakeFace(BRepBuilderAPI_MakeWire(edge).Wire()).Face();
    }
  }
  void add(TopoDS_Shape shape,uint32_t source,uint8_t role,bool clip=true) {
    if(shape.IsNull()) return;
    if(!BRepLib::BuildCurves3d(shape))throw std::runtime_error("Projected edge has no valid 3D curve representation");
    if(clip && job.detail) shape=common(shape,detail);
    if(!BRepLib::BuildCurves3d(shape))throw std::runtime_error("Clipped drawing edge has no valid curve representation");
    for(TopExp_Explorer it(shape,TopAbs_EDGE);it.More();it.Next()) {
      auto edge=TopoDS::Edge(it.Current());if(BRep_Tool::Degenerated(edge))continue;
      if(result.size()>=job.max_curves)throw std::runtime_error("Drawing vector budget exceeded");
      BRepAdaptor_Curve c(edge);const double first=c.FirstParameter(),last=c.LastParameter();
      if(!std::isfinite(first)||!std::isfinite(last)||last<=first)throw std::runtime_error("Drawing curve has no finite trim domain");
      DrawingCurveData data{};data.source_index=source;data.role=role;
      if(c.GetType()==GeomAbs_Line) {
        data.kind=0;append_point(data.data,c.Value(first));append_point(data.data,c.Value(last));
      } else if(c.GetType()==GeomAbs_Circle || c.GetType()==GeomAbs_Ellipse) {
        data.kind=1;
        gp_Ax2 axes;double major,minor;
        if(c.GetType()==GeomAbs_Circle) {auto circle=c.Circle();axes=circle.Position();major=minor=circle.Radius();}
        else {auto ellipse=c.Ellipse();axes=ellipse.Position();major=ellipse.MajorRadius();minor=ellipse.MinorRadius();}
        append_point(data.data,axes.Location());append_dir(data.data,axes.XDirection());append_dir(data.data,axes.YDirection());
        data.data.push_back(major);data.data.push_back(minor);data.data.push_back(first);data.data.push_back(last);
      } else if(c.GetType()==GeomAbs_BSplineCurve) {
        data.kind=2;auto spline=c.BSpline();data.degree=spline->Degree();data.periodic=spline->IsPeriodic();
        if(static_cast<uint32_t>(spline->NbPoles())>job.max_points || static_cast<uint32_t>(spline->NbKnots())>job.max_points)throw std::runtime_error("Exact drawing spline budget exceeded");
        data.data.push_back(first);data.data.push_back(last);
        for(int i=1;i<=spline->NbPoles();++i) {append_point(data.data,spline->Pole(i));data.data.push_back(spline->Weight(i));}
        for(int i=1;i<=spline->NbKnots();++i) {data.knots.push_back(spline->Knot(i));data.multiplicities.push_back(spline->Multiplicity(i));}
      } else {throw std::runtime_error("Projected curve family is unsupported; no edge was dropped");}
      if(c.GetType()==GeomAbs_Line) {append_point(data.preview,c.Value(first));append_point(data.preview,c.Value(last));}
      else {
        GCPnts_QuasiUniformDeflection sample(c,job.tolerance,first,last);
        if(!sample.IsDone() || sample.NbPoints()<2)throw std::runtime_error("Drawing preview sampling failed");
        if(static_cast<uint64_t>(points)+sample.NbPoints()>job.max_points)throw std::runtime_error("Drawing point budget exceeded");
        for(int i=1;i<=sample.NbPoints();++i)append_point(data.preview,sample.Value(i));
      }
      points+=data.preview.size()/2;
      if(points>job.max_points)throw std::runtime_error("Drawing point budget exceeded");
      result.push_back(std::move(data));
    }
  }
};
static TopoDS_Edge projected_line(const gp_Pnt &a,const gp_Pnt &b,const gp_Ax2 &f) {return BRepBuilderAPI_MakeEdge(local(a,f),local(b,f)).Edge();}
static void hatch_face(Collector &out,const TopoDS_Face &face,const DrawingInput &input,uint32_t source) {
  const auto &job=out.job;
  Bnd_Box bounds;BRepBndLib::Add(face,bounds,false);
  if(bounds.IsVoid())return;
  double xmin,ymin,zmin,xmax,ymax,zmax;bounds.Get(xmin,ymin,zmin,xmax,ymax,zmax);
  const double diagonal=gp_Pnt(xmin,ymin,zmin).Distance(gp_Pnt(xmax,ymax,zmax));
  // Exact line/face common operations clip each hatch to the cut region,
  // including holes. The hatch spacing is supplied in model units.
  for(int pass=0;pass<(input.cross?2:1);++pass) {
    const double angle=input.angle+pass*1.57079632679489661923;
    const gp_Vec direction=gp_Vec(job.frame.XDirection())*std::cos(angle)+gp_Vec(job.frame.YDirection())*std::sin(angle);
    const gp_Vec perpendicular=gp_Vec(job.frame.XDirection())*(-std::sin(angle))+gp_Vec(job.frame.YDirection())*std::cos(angle);
    gp_Pnt center((xmin+xmax)*.5,(ymin+ymax)*.5,(zmin+zmax)*.5);
    // A world-axis bounding-box center need not lie on an oblique cut face
    // (for example a triangular corner section). Keep every clipping line in
    // the actual face plane; otherwise the exact common operation is empty.
    const auto plane=BRepAdaptor_Surface(face).Plane();
    const gp_Vec normal(plane.Axis().Direction());
    center.Translate(normal*(-gp_Vec(plane.Location(),center).Dot(normal)));
    const double requested=std::ceil(diagonal/input.spacing)+2.;
    if(!std::isfinite(requested)||requested>20000)throw std::runtime_error("Section hatch line budget exceeded");
    const int count=static_cast<int>(requested);
    for(int i=-count;i<=count;++i) {
      const auto p=center.Translated(perpendicular*(i*input.spacing));
      auto line=BRepBuilderAPI_MakeEdge(p.Translated(direction*(-diagonal-1.)),p.Translated(direction*(diagonal+1.))).Edge();
      auto clipped=common(face,line);
      for(TopExp_Explorer it(clipped,TopAbs_EDGE);it.More();it.Next()) {
        BRepAdaptor_Curve curve(TopoDS::Edge(it.Current()));
        auto a=curve.Value(curve.FirstParameter()),b=curve.Value(curve.LastParameter());
        if(a.Distance(b)>1e-7)out.add(projected_line(a,b,job.frame),source,3);
      }
    }
  }
}
rust::Vec<DrawingCurveData> drawing_run(DrawingJob &job) {
  try {
    Collector out(job);Handle(HLRBRep_Algo) algo=new HLRBRep_Algo();
    std::vector<TopoDS_Shape> shapes; bool any=false;
    for(const auto &input:job.inputs) {
      auto shape=job.section?section_shape(input.shape,job):input.shape;
      shapes.push_back(shape);
      if(volume(shape)>0.) {algo->Add(shape,0);any=true;}
    }
    if(shapes.empty())throw std::runtime_error("Drawing has no source solids");
    if(!any)return {};
    algo->Projector(HLRAlgo_Projector(job.frame));algo->Update();algo->Hide();
    HLRBRep_HLRToShape curves(algo);
    for(uint32_t i=0;i<shapes.size();++i) {
      const auto &shape=shapes[i];if(volume(shape)<=0.)continue;
      out.add(curves.VCompound(shape),i,0);out.add(curves.OutLineVCompound(shape),i,0);
      if(job.tangents)out.add(curves.Rg1LineVCompound(shape),i,0);
      if(job.hidden) {out.add(curves.HCompound(shape),i,1);out.add(curves.OutLineHCompound(shape),i,1);if(job.tangents)out.add(curves.Rg1LineHCompound(shape),i,1);}
      const auto &input=job.inputs[i];
      if(!job.section || volume(input.shape)-volume(shape)<=1e-9)continue;
      const auto plane_origin=job.frame.Location().Translated(gp_Vec(job.frame.Direction())*job.offset);
      for(TopExp_Explorer it(shape,TopAbs_FACE);it.More();it.Next()) {
        const auto face=TopoDS::Face(it.Current());BRepAdaptor_Surface surface(face);
        if(surface.GetType()!=GeomAbs_Plane)continue;
        const auto plane=surface.Plane();
        if(!plane.Axis().Direction().IsParallel(job.frame.Direction(),1e-10)||plane.Distance(plane_origin)>1e-7)continue;
        const auto &f=job.frame; const auto &u=f.XDirection(), &v=f.YDirection(), &n=f.Direction();
        const gp_Vec origin(gp::Origin(),f.Location());
        gp_Trsf transform;
        transform.SetValues(u.X(),u.Y(),u.Z(),-origin.Dot(gp_Vec(u)),v.X(),v.Y(),v.Z(),-origin.Dot(gp_Vec(v)),n.X(),n.Y(),n.Z(),-origin.Dot(gp_Vec(n)));
        out.add(BRepBuilderAPI_Transform(face,transform,true).Shape(),i,2);
        if(input.hatch)hatch_face(out,face,input,i);
      }
    }
    return std::move(out.result);
  } catch(const Standard_Failure &e) {throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native drawing failed");}
}
}
