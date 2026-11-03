#include "threads.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepLib.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_ConicalSurface.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <cmath>
#include <stdexcept>
#include <vector>
#include <algorithm>
#include <limits>

namespace multicad::kernel {
namespace {
struct ThreadFace { TopoDS_Shape face; uint32_t source; uint8_t role; };
class ThreadRidgeLoss : public std::runtime_error {
public:
  ThreadRidgeLoss():std::runtime_error("Native Boolean did not retain the requested thread crests and gaps"){}
};
template<class Operation>
std::vector<ThreadFace> carry_thread_faces(Operation &operation,
    const std::vector<ThreadFace> &sources) {
  TopTools_IndexedMapOfShape output;
  TopExp::MapShapes(operation.Shape(),TopAbs_FACE,output);
  std::vector<ThreadFace> result;
  for(const auto &source:sources) {
    auto append=[&](const TopoDS_Shape &face) {
      if(output.Contains(face))result.push_back({face,source.source,source.role});
    };
    append(source.face);
    for(const auto &face:operation.Modified(source.face))append(face);
    if(result.size()>16384)throw std::runtime_error("Thread face ancestry budget exceeded");
  }
  return result;
}
void boolean_inputs(BRepAlgoAPI_BooleanOperation &operation,
    const TopoDS_Shape &a,const TopoDS_Shape &b) {
  TopTools_ListOfShape arguments,tools;arguments.Append(a);tools.Append(b);
  operation.SetArguments(arguments);operation.SetTools(tools);
  operation.SetNonDestructive(true);operation.SetRunParallel(false);
  operation.Build();
  if(!operation.IsDone()||operation.HasErrors()||operation.Shape().IsNull())
    throw std::runtime_error("Native thread Boolean failed");
}
}
static std::unique_ptr<Shape> build_helical_thread(const ThreadData &s,bool segmented) {
 try {
  const double pi=std::acos(-1.),lead=s.pitch*s.starts;
  const double root=s.radius-s.depth,z0=-2*lead,z1=s.height+2*lead;
  if(s.starts<1||s.starts>8||!std::isfinite(lead)||lead<=0||s.height<s.pitch||
      s.height/s.pitch+2*s.starts>128||root+s.taper*z0<=0||root+s.taper*z1<=0)
    throw std::runtime_error("Thread exceeds the validated radius, length or start budget");
  const gp_Ax2 axis(gp_Pnt(0,0,z0),gp_Dir(0,0,1));
  TopoDS_Shape solid=s.taper==0?
      BRepPrimAPI_MakeCylinder(axis,root,z1-z0).Shape():
      BRepPrimAPI_MakeCone(axis,root+s.taper*z0,root+s.taper*z1,z1-z0).Shape();
  std::vector<ThreadFace> ancestry;
  for(TopExp_Explorer it(solid,TopAbs_FACE);it.More();it.Next()) {
    const auto type=BRepAdaptor_Surface(TopoDS::Face(it.Current())).GetType();
    if(type==GeomAbs_Cylinder||type==GeomAbs_Cone)ancestry.push_back({it.Current(),0,3});
  }
  const double alpha=std::atan(s.taper),c=std::cos(alpha),sign=s.left?-1.:1.;
  Handle(Geom_Surface) surface;
  if(s.taper==0)surface=new Geom_CylindricalSurface(gp_Ax3(),root);
  else surface=new Geom_ConicalSurface(gp_Ax3(),alpha,root);
  for(uint32_t start=0;start<s.starts;++start) {
    const double phase=2*pi*start/s.starts,z=-lead;
    Handle(Geom2d_Line) line=new Geom2d_Line(gp_Pnt2d(phase,z/c),gp_Dir2d(sign*2*pi,lead/c));
    const double end=(s.height+2*lead)/lead*std::hypot(2*pi,lead/c);
    // Preserve the established continuous representation when it passes the
    // physical guards. A failed clipping result may be rebuilt with bounded
    // per-turn faces on the same helix, without changing its extent or phase.
    BRepBuilderAPI_MakeWire spineBuilder;
    const double turn=segmented?std::hypot(2*pi,lead/c):end;
    for(uint32_t span=0;;++span) {
      const double begin=span*turn;
      if(begin>=end)break;
      // Derive boundaries from the integer span index, avoiding accumulated
      // addition error. Merge only end-parameter-scale floating-point roundoff
      // into the preceding edge; never move the exact fractional endpoint.
      const double next=(span+1)*turn;
      const double finish=next<end-16*std::numeric_limits<double>::epsilon()*end?next:end;
      auto edge=BRepBuilderAPI_MakeEdge(line,surface,begin,finish).Edge();
      if(!BRepLib::BuildCurve3d(edge,s.tolerance))throw std::runtime_error("Native helix has no bounded 3D curve");
      spineBuilder.Add(edge);
      if(finish==end)break;
    }
    const auto spine=spineBuilder.Wire();
    const auto point=[&](double radius,double dz) {
      const double r=radius+s.taper*(z+dz);
      return gp_Pnt(r*std::cos(phase),r*std::sin(phase),z+dz);
    };
    const gp_Pnt points[]={
      point(root-s.overlap,-s.crest_width/2-(s.depth+s.overlap)*s.lower_slope),
      point(s.radius,-s.crest_width/2),
      point(s.radius,s.crest_width/2),
      point(root-s.overlap,s.crest_width/2+(s.depth+s.overlap)*s.upper_slope)
    };
    BRepBuilderAPI_MakeWire profile;std::vector<TopoDS_Edge> edges;
    for(uint32_t i=0;i<4;++i){auto e=BRepBuilderAPI_MakeEdge(points[i],points[(i+1)%4]).Edge();profile.Add(e);edges.push_back(profile.Edge());}
    if(!profile.IsDone())throw std::runtime_error("Native thread profile is disconnected");
    BRepOffsetAPI_MakePipeShell pipe(spine);
    pipe.SetMode(gp_Dir(0,0,1));pipe.SetTolerance(s.tolerance,s.tolerance,1e-9);
    pipe.Add(profile.Wire(),false,false);pipe.Build();
    if(!pipe.IsDone()||!pipe.MakeSolid()||!BRepCheck_Analyzer(pipe.Shape()).IsValid())
      throw std::runtime_error("Native helical ridge is not a valid solid");
    for(uint32_t i=0;i<4;++i)for(const auto &face:pipe.Generated(edges[i]))
      if(face.ShapeType()==TopAbs_FACE)ancestry.push_back({face,start*4+i,2});
    BRepAlgoAPI_Fuse join;boolean_inputs(join,solid,pipe.Shape());
    ancestry=carry_thread_faces(join,ancestry);solid=join.Shape();
  }
  // Joining before trimming is essential: a separately trimmed ridge can be
  // silently lost by a later union despite a valid, one-solid core result.
  const double bound=s.radius+std::abs(s.taper)*(s.height+3*lead)+s.pitch;
  const auto limit=BRepPrimAPI_MakeBox(gp_Pnt(-bound,-bound,0),2*bound,2*bound,s.height).Shape();
  BRepAlgoAPI_Common trim;boolean_inputs(trim,solid,limit);
  ancestry=carry_thread_faces(trim,ancestry);solid=trim.Shape();
  if(!BRepCheck_Analyzer(solid).IsValid())throw std::runtime_error("Finished thread is invalid");
  int count=0;for(TopExp_Explorer it(solid,TopAbs_SOLID);it.More();it.Next())++count;
  if(count!=1)throw std::runtime_error("Thread must remain one connected solid");
  // Validate physical crests and gaps, not merely a successful Boolean call.
  // Probe every quarter-pitch band and the original regression sample. This
  // checks the whole axial extent, rather than accepting one surviving crest.
  const uint32_t bands=static_cast<uint32_t>(std::ceil(4*s.height/s.pitch));
  BRepClass3d_SolidClassifier classifier(solid);
  for(uint32_t band=0;band<=bands;++band) {
    const double sampleZ=band==bands?s.height*.47:s.height*(band+.5)/bands;
    for(uint32_t start=0;start<s.starts;++start) {
      const double angle=sign*2*pi*sampleZ/lead+2*pi*start/s.starts;
      const double radius=s.radius+s.taper*sampleZ-std::min(s.depth*.05,s.crest_width*.1);
      const auto state=[&](double a){classifier.Perform(gp_Pnt(radius*std::cos(a),radius*std::sin(a),sampleZ),s.tolerance*4);return classifier.State();};
      if(state(angle)!=TopAbs_IN||state(angle+pi/s.starts)!=TopAbs_OUT)
        throw ThreadRidgeLoss();
    }
  }
  auto result=std::make_unique<Shape>(Shape{solid,{},{}});
  TopTools_IndexedMapOfShape faces;TopExp::MapShapes(solid,TopAbs_FACE,faces);
  for(int i=1;i<=faces.Extent();++i) {
    const BRepAdaptor_Surface surface(TopoDS::Face(faces(i)));
    bool cap=false;
    if(surface.GetType()==GeomAbs_Plane) {
      const auto plane=surface.Plane();const double z=plane.Location().Z();
      if(std::abs(plane.Axis().Direction().Z())>1-1e-10) {
        if(std::abs(z)<=s.tolerance*16){result->seeds.push_back({static_cast<uint32_t>(i-1),0,0});cap=true;}
        else if(std::abs(z-s.height)<=s.tolerance*16){result->seeds.push_back({static_cast<uint32_t>(i-1),0,1});cap=true;}
      }
    }
    if(!cap) {
      bool found=false;
      for(const auto &source:ancestry)if(source.face.IsSame(faces(i))){result->seeds.push_back({static_cast<uint32_t>(i-1),source.source,source.role});found=true;}
      if(!found)throw std::runtime_error("Thread face has no exact native ancestry");
    }
  }
  return result;
 } catch(const Standard_Failure &e) {
  throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native thread operation failed");
 }
}
std::unique_ptr<Shape> helical_thread(const ThreadData &s) {
  try {
    auto result=build_helical_thread(s,false);
    volume(*result);
    return result;
  } catch(const ThreadRidgeLoss &) {
    // Retry a proven physical clipping failure on the same authored helix.
  } catch(const VolumeIntegrationFailure &) {
    // The continuous B-rep passed the physical guards but cannot satisfy the
    // shared volume integration guard. Reconstruct the same helix with bounded
    // faces. Other native, validity and ancestry errors are never retried.
  }
  auto result=build_helical_thread(s,true);
  volume(*result);
  return result;
}
}
