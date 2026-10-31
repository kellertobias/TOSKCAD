#include "drawing.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeHalfSpace.hxx>
#include <TopoDS_Compound.hxx>
#include <BRep_Builder.hxx>
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
#include <mutex>
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
  job->section_only=o.section_only; job->depth=o.depth; job->depth_mm=o.depth_mm;
  job->window_u=o.window_u; job->u_min=o.u_min; job->u_max=o.u_max;
  job->window_v=o.window_v; job->v_min=o.v_min; job->v_max=o.v_max;
  return job;
}
void drawing_add(DrawingJob &job,const Shape &shape,bool hatch,bool cross,double angle,double spacing) {
  if(job.inputs.size()>=4096) throw std::runtime_error("Drawing source budget exceeded");
  job.inputs.push_back({shape.value,hatch,cross,angle,spacing});
}
static TopoDS_Shape common(const TopoDS_Shape &a,const TopoDS_Shape &b) {
  // The two-shape constructor performs Build immediately. Configure first and
  // build once: sections and every hatch line otherwise intersect twice.
  BRepAlgoAPI_Common op;
  TopTools_ListOfShape arguments,tools;arguments.Append(a);tools.Append(b);
  op.SetArguments(arguments);op.SetTools(tools);
  op.SetNonDestructive(true); op.SetRunParallel(false); op.Build();
  if(!op.IsDone() || op.HasErrors()) throw std::runtime_error("Drawing intersection failed");
  return op.Shape();
}
static double volume(const TopoDS_Shape &s) {GProp_GProps p;BRepGProp::VolumeProperties(s,p);return std::abs(p.Mass());}
static gp_Pnt local(const gp_Pnt &p,const gp_Ax2 &f) {
  const gp_Vec d(f.Location(),p);return {d.Dot(gp_Vec(f.XDirection())),d.Dot(gp_Vec(f.YDirection())),0.};
}
// Frame-coordinate bounds (u, v, n) of every input, widened so that box faces
// on unlimited sides never touch geometry.
struct FrameBounds { double lo[3], hi[3]; };
static FrameBounds frame_bounds(const DrawingJob &job) {
  Bnd_Box box;for(const auto &input:job.inputs)BRepBndLib::Add(input.shape,box,false);
  if(box.IsVoid())throw std::runtime_error("Drawing sources have no extent");
  double x0,y0,z0,x1,y1,z1;box.Get(x0,y0,z0,x1,y1,z1);
  FrameBounds b{{1e300,1e300,1e300},{-1e300,-1e300,-1e300}};
  const gp_Dir axes[3]={job.frame.XDirection(),job.frame.YDirection(),job.frame.Direction()};
  for(int c=0;c<8;++c) {
    const gp_Vec d(job.frame.Location(),gp_Pnt(c&1?x1:x0,c&2?y1:y0,c&4?z1:z0));
    for(int i=0;i<3;++i){const double t=d.Dot(gp_Vec(axes[i]));b.lo[i]=std::min(b.lo[i],t);b.hi[i]=std::max(b.hi[i],t);}
  }
  const double margin=1.+0.01*gp_Pnt(x0,y0,z0).Distance(gp_Pnt(x1,y1,z1));
  for(int i=0;i<3;++i){b.lo[i]-=margin;b.hi[i]+=margin;}
  return b;
}
static bool limited(const DrawingJob &job) {return job.section_only||job.depth||job.window_u||job.window_v;}
static void lateral(const DrawingJob &job,const FrameBounds &b,double u[2],double v[2]) {
  u[0]=job.window_u?job.u_min:b.lo[0];u[1]=job.window_u?job.u_max:b.hi[0];
  v[0]=job.window_v?job.v_min:b.lo[1];v[1]=job.window_v?job.v_max:b.hi[1];
}
static gp_Pnt frame_point(const DrawingJob &job,double u,double v,double n) {
  return job.frame.Location().Translated(gp_Vec(job.frame.XDirection())*u+gp_Vec(job.frame.YDirection())*v+gp_Vec(job.frame.Direction())*n);
}
// The retained region of a finite-depth or windowed section: a box from the
// far limit (or beyond all geometry) up to the cut plane.
static TopoDS_Shape limited_section(const TopoDS_Shape &shape,const DrawingJob &job,const FrameBounds &b) {
  double u[2],v[2];lateral(job,b,u,v);
  const double far_limit=job.depth?job.offset-job.depth_mm:b.lo[2];
  if(job.offset-far_limit<=1e-9 || u[1]-u[0]<=1e-9 || v[1]-v[0]<=1e-9 || u[0]>=b.hi[0] || u[1]<=b.lo[0] || v[0]>=b.hi[1] || v[1]<=b.lo[1]) return TopoDS_Compound();
  const gp_Ax2 axes(frame_point(job,u[0],v[0],far_limit),job.frame.Direction(),job.frame.XDirection());
  return common(shape,BRepPrimAPI_MakeBox(axes,u[1]-u[0],v[1]-v[0],job.offset-far_limit).Solid());
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
    // Intersect one compound of parallel hatch lines with the face. A separate
    // Boolean per line repeatedly rebuilds the same face's intersection data.
    TopoDS_Compound lines;BRep_Builder builder;builder.MakeCompound(lines);
    for(int i=-count;i<=count;++i) {
      const auto p=center.Translated(perpendicular*(i*input.spacing));
      auto line=BRepBuilderAPI_MakeEdge(p.Translated(direction*(-diagonal-1.)),p.Translated(direction*(diagonal+1.))).Edge();
      builder.Add(lines,line);
    }
    auto clipped=common(face,lines);
    for(TopExp_Explorer it(clipped,TopAbs_EDGE);it.More();it.Next()) {
      BRepAdaptor_Curve curve(TopoDS::Edge(it.Current()));
      auto a=curve.Value(curve.FirstParameter()),b=curve.Value(curve.LastParameter());
      if(a.Distance(b)>1e-7)out.add(projected_line(a,b,job.frame),source,3);
    }
  }
}
// OCCT 7.9.3 HLR extraction reaches BRepLib_MakeEdge2d, whose BRepLib::Plane
// uses an explicitly non-thread-safe process-global lazy handle. Independent
// document workers must share this boundary; a per-job mutex cannot protect it.
// Keep compilation and the asynchronous Rust job/cancellation lifecycle outside.
static std::mutex drawing_projection_mutex;
// World to view-frame coordinates, so section faces project onto paper exactly.
static gp_Trsf to_frame(const gp_Ax2 &f) {
  const auto &u=f.XDirection(), &v=f.YDirection(), &n=f.Direction();
  const gp_Vec origin(gp::Origin(),f.Location());
  gp_Trsf transform;
  transform.SetValues(u.X(),u.Y(),u.Z(),-origin.Dot(gp_Vec(u)),v.X(),v.Y(),v.Z(),-origin.Dot(gp_Vec(v)),n.X(),n.Y(),n.Z(),-origin.Dot(gp_Vec(n)));
  return transform;
}
// Section only: each solid's exact intersection with the bounded cut plane.
// Nothing behind or in front of the plane is drawn.
static rust::Vec<DrawingCurveData> section_only(DrawingJob &job) {
  Collector out(job);
  const auto bounds=frame_bounds(job);
  double u[2],v[2];lateral(job,bounds,u,v);
  if(u[1]-u[0]<=1e-9||v[1]-v[0]<=1e-9)return {};
  const gp_Pln plane(gp_Ax3(frame_point(job,0.,0.,job.offset),job.frame.Direction(),job.frame.XDirection()));
  const auto cut=BRepBuilderAPI_MakeFace(plane,u[0],u[1],v[0],v[1]).Face();
  const auto transform=to_frame(job.frame);
  for(uint32_t i=0;i<job.inputs.size();++i) {
    const auto &input=job.inputs[i];
    const auto section=common(input.shape,cut);
    for(TopExp_Explorer it(section,TopAbs_FACE);it.More();it.Next()) {
      const auto face=TopoDS::Face(it.Current());
      out.add(BRepBuilderAPI_Transform(face,transform,true).Shape(),i,2);
      if(input.hatch)hatch_face(out,face,input,i);
    }
  }
  return std::move(out.result);
}
rust::Vec<DrawingCurveData> drawing_run(DrawingJob &job) {
  const std::lock_guard<std::mutex> guard(drawing_projection_mutex);
  try {
    if(job.section&&job.section_only)return section_only(job);
    Collector out(job);Handle(HLRBRep_Algo) algo=new HLRBRep_Algo();
    std::vector<TopoDS_Shape> shapes; std::vector<double> volumes; bool any=false;
    const bool bounded=job.section&&limited(job);
    const FrameBounds bounds=bounded?frame_bounds(job):FrameBounds{};
    for(const auto &input:job.inputs) {
      auto shape=bounded?limited_section(input.shape,job,bounds):job.section?section_shape(input.shape,job):input.shape;
      shapes.push_back(shape);
      volumes.push_back(volume(shape));
      if(volumes.back()>0.) {algo->Add(shape,0);any=true;}
    }
    if(shapes.empty())throw std::runtime_error("Drawing has no source solids");
    if(!any)return {};
    algo->Projector(HLRAlgo_Projector(job.frame));algo->Update();algo->Hide();
    HLRBRep_HLRToShape curves(algo);
    for(uint32_t i=0;i<shapes.size();++i) {
      const auto &shape=shapes[i];if(volumes[i]<=0.)continue;
      out.add(curves.VCompound(shape),i,0);out.add(curves.OutLineVCompound(shape),i,0);
      if(job.tangents)out.add(curves.Rg1LineVCompound(shape),i,0);
      if(job.hidden) {out.add(curves.HCompound(shape),i,1);out.add(curves.OutLineHCompound(shape),i,1);if(job.tangents)out.add(curves.Rg1LineHCompound(shape),i,1);}
      const auto &input=job.inputs[i];
      if(!job.section || volume(input.shape)-volumes[i]<=1e-9)continue;
      const auto plane_origin=job.frame.Location().Translated(gp_Vec(job.frame.Direction())*job.offset);
      for(TopExp_Explorer it(shape,TopAbs_FACE);it.More();it.Next()) {
        const auto face=TopoDS::Face(it.Current());BRepAdaptor_Surface surface(face);
        if(surface.GetType()!=GeomAbs_Plane)continue;
        const auto plane=surface.Plane();
        if(!plane.Axis().Direction().IsParallel(job.frame.Direction(),1e-10)||plane.Distance(plane_origin)>1e-7)continue;
        out.add(BRepBuilderAPI_Transform(face,to_frame(job.frame),true).Shape(),i,2);
        if(input.hatch)hatch_face(out,face,input,i);
      }
    }
    return std::move(out.result);
  } catch(const Standard_Failure &e) {throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native drawing failed");}
}
}
