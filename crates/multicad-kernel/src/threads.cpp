#include "threads.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRep_Builder.hxx>
#include <BRepFill_Edge3DLaw.hxx>
#include <BRepFill_ShapeLaw.hxx>
#include <BRepFill_Sweep.hxx>
#include <GeomFill_CurveAndTrihedron.hxx>
#include <GeomFill_TrihedronLaw.hxx>
#include <GeomFill_ConstantBiNormal.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepLib.hxx>
#include <BRepTools.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_ConicalSurface.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <Geom_TrimmedCurve.hxx>
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
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <ostream>
#include <streambuf>
#include <tuple>

namespace multicad::kernel {
using ThreadKey=std::tuple<std::array<uint64_t,13>,uint32_t,bool,bool>;
struct ThreadFactoryTag {
  ThreadKey key;
  TopoDS_Shape original;
};
namespace {
constexpr size_t maxThreadEntries=16;
constexpr uint64_t maxThreadBytes=32*1024*1024;
uint64_t exact_bits(double value) {
  static_assert(sizeof(value)==sizeof(uint64_t));
  uint64_t bits;std::memcpy(&bits,&value,sizeof(bits));return bits;
}
ThreadKey thread_key(const ThreadData &s) {
  // No padding bytes, approximate equality, input normalization or owner IDs.
  return {{exact_bits(s.radius),exact_bits(s.pitch),exact_bits(s.height),
      exact_bits(s.depth),exact_bits(s.crest_width),exact_bits(s.rounding),
      exact_bits(s.lower_slope),exact_bits(s.upper_slope),exact_bits(s.taper),
      exact_bits(s.overlap),exact_bits(s.tolerance),exact_bits(s.crest_limit),exact_bits(s.root_rounding)},s.starts,s.left,s.axial_flat};
}
struct ThreadEntry {
  std::unique_ptr<Shape> shape;
  uint64_t bytes,used;
};
struct ThreadCache {
  std::map<ThreadKey,ThreadEntry> entries;
  ThreadFactoryStats stats{};
  uint64_t clock=0,bytes=0,byte_limit=maxThreadBytes;
  size_t entry_limit=maxThreadEntries;
};
ThreadCache &thread_cache() {
  // Native handles are retained and destroyed on their owning thread only.
  thread_local ThreadCache cache;
  return cache;
}
std::unique_ptr<Shape> thread_wrapper(const Shape &source) {
  // All persistent owner naming lives in Rust, never in this neutral wrapper.
  // Numeric source-seed roles survive; inherited operand history does not.
  auto result=std::make_unique<Shape>(Shape{source.value,{},source.seeds});
  result->bounds_cache=source.bounds_cache;
  result->volume_cache=source.volume_cache;
  result->face_cache=source.face_cache;
  result->validity_cache=source.validity_cache;
  result->mesh_identity=source.mesh_identity;
  result->thread_factory_tag=source.thread_factory_tag;
  return result;
}
class CountingStream : public std::streambuf {
public:
  explicit CountingStream(uint64_t limit):limit_(limit){}
  uint64_t bytes()const{return bytes_;}
protected:
  std::streamsize xsputn(const char *,std::streamsize count) override {
    if(count>0)add(static_cast<uint64_t>(count));
    return count;
  }
  int_type overflow(int_type value) override {
    if(!traits_type::eq_int_type(value,traits_type::eof()))add(1);
    return traits_type::not_eof(value);
  }
private:
  void add(uint64_t count){bytes_=count>limit_||bytes_>limit_-count?limit_+1:bytes_+count;}
  uint64_t limit_,bytes_=0;
};
// Developer-only bounded timings; never write into the MCP stdout transport.
struct ThreadProfile {
  using Clock=std::chrono::steady_clock;
  bool enabled=false;
  uint64_t id=0;
  const char *mode="none";
  Clock::time_point total,attempt;
  explicit ThreadProfile(const ThreadData &s) {
    const auto flag=std::getenv("MULTICAD_NATIVE_THREAD_PROFILE");
    if(!flag||std::strcmp(flag,"1")!=0)return;
    static std::atomic<uint64_t> calls{0};
    id=calls.fetch_add(1,std::memory_order_relaxed);
    enabled=id<64;
    if(!enabled)return;
    total=Clock::now();
    std::fprintf(stderr,"multicad.native.thread id=%llu R=%.17g P=%.17g L=%.17g rounding=%.17g\n",static_cast<unsigned long long>(id),s.radius,s.pitch,s.height,s.rounding);
  }
  Clock::time_point mark()const{return enabled?Clock::now():Clock::time_point{};}
  void stage(const char *name,Clock::time_point begin)const {
    if(enabled)std::fprintf(stderr,"multicad.native.thread id=%llu mode=%s stage=%s milliseconds=%.6f\n",static_cast<unsigned long long>(id),mode,name,std::chrono::duration<double,std::milli>(Clock::now()-begin).count());
  }
  void start(bool segmented){mode=segmented?"bounded":"continuous";attempt=mark();}
  void retry(const char *reason)const {
    if(enabled)std::fprintf(stderr,"multicad.native.thread id=%llu mode=%s retry=%s milliseconds=%.6f\n",static_cast<unsigned long long>(id),mode,reason,std::chrono::duration<double,std::milli>(Clock::now()-attempt).count());
  }
  void probe(const char *kind,double z,double angle,double radius,TopAbs_State inside,TopAbs_State outside)const {
    if(enabled)std::fprintf(stderr,"multicad.native.thread id=%llu mode=%s failed_probe=%s z=%.17g angle=%.17g radius=%.17g inside=%d outside=%d\n",static_cast<unsigned long long>(id),mode,kind,z,angle,radius,int(inside),int(outside));
  }
  ~ThreadProfile(){stage("total",total);}
};
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
// The conical helix tangent has a radial component. Constant-binormal
// transport would introduce an unwanted radius-dependent azimuth offset.
// This law retains the authored radial/axial meridian at every parameter.
class MeridianFrame final: public GeomFill_TrihedronLaw {
  double phase_,omega_;
public:
  MeridianFrame(double phase,double omega):phase_(phase),omega_(omega){}
  Handle(GeomFill_TrihedronLaw) Copy()const override {
    Handle(MeridianFrame) copy=new MeridianFrame(phase_,omega_);
    if(!myCurve.IsNull())copy->SetCurve(myCurve);
    return copy;
  }
  Standard_Boolean D0(Standard_Real t,gp_Vec &T,gp_Vec &N,gp_Vec &B)override {
    const double u=phase_+omega_*t,hand=omega_<0?-1.:1.;
    N=gp_Vec(-hand*std::cos(u),-hand*std::sin(u),0);B=gp_Vec(0,0,1);
    T=gp_Vec(-hand*std::sin(u),hand*std::cos(u),0);return true;
  }
  Standard_Boolean D1(Standard_Real t,gp_Vec &T,gp_Vec &dT,gp_Vec &N,gp_Vec &dN,gp_Vec &B,gp_Vec &dB)override {
    D0(t,T,N,B);dT=omega_*N;dN=-omega_*T;dB=gp_Vec(0,0,0);return true;
  }
  Standard_Boolean D2(Standard_Real t,gp_Vec &T,gp_Vec &dT,gp_Vec &ddT,gp_Vec &N,gp_Vec &dN,gp_Vec &ddN,gp_Vec &B,gp_Vec &dB,gp_Vec &ddB)override {
    D1(t,T,dT,N,dN,B,dB);ddT=-omega_*omega_*T;ddN=-omega_*omega_*N;ddB=gp_Vec(0,0,0);return true;
  }
  Standard_Integer NbIntervals(GeomAbs_Shape)const override{return 1;}
  void Intervals(TColStd_Array1OfReal &a,GeomAbs_Shape)const override {
    a(a.Lower())=myTrimmed->FirstParameter();a(a.Lower()+1)=myTrimmed->LastParameter();
  }
  void GetAverageLaw(gp_Vec &T,gp_Vec &N,gp_Vec &B)override {
    D0((myTrimmed->FirstParameter()+myTrimmed->LastParameter())/2,T,N,B);
  }
};
struct MeridianSweep {
  TopoDS_Shape solid;
  std::vector<std::vector<TopoDS_Shape>> faces;
};
Handle(BRepFill_Edge3DLaw) meridian_location(const TopoDS_Wire &spine,double phase,double omega) {
  Handle(GeomFill_CurveAndTrihedron) elementary=new GeomFill_CurveAndTrihedron(new MeridianFrame(phase,omega));
  return new BRepFill_Edge3DLaw(spine,elementary);
}
MeridianSweep sweep_meridian(const TopoDS_Wire &spine,const TopoDS_Wire &profile,
    const std::vector<TopoDS_Edge> &edges,double phase,double omega,
    double anchorRadius,double anchorZ,double tolerance) {
  const double co=std::cos(phase),si=std::sin(phase),hand=omega<0?-1.:1.;
  gp_Trsf local;
  local.SetValues(-hand*co,-hand*si,0,hand*anchorRadius,0,0,1,-anchorZ,-hand*si,hand*co,0,0);
  BRepBuilderAPI_Transform transform(profile,local,true);
  Handle(BRepFill_ShapeLaw) section=new BRepFill_ShapeLaw(TopoDS::Wire(transform.Shape()));
  auto location=meridian_location(spine,phase,omega);
  BRepFill_Sweep sweep(section,location,true);
  TopoDS_Wire boundaries[2];
  for(int end=0;end<2;++end) {
    const auto &law=location->Law(end==0?1:location->NbLaw());
    double first,last;law->GetDomain(first,last);
    gp_Mat matrix;gp_Vec translation;
    if(!law->D0(end==0?first:last,matrix,translation))throw std::runtime_error("Tapered end frame is unavailable");
    gp_Trsf pose;pose.SetValues(matrix.Value(1,1),matrix.Value(1,2),matrix.Value(1,3),translation.X(),
      matrix.Value(2,1),matrix.Value(2,2),matrix.Value(2,3),translation.Y(),
      matrix.Value(3,1),matrix.Value(3,2),matrix.Value(3,3),translation.Z());
    BRepBuilderAPI_Transform boundary(transform.Shape(),pose,true);
    boundaries[end]=TopoDS::Wire(boundary.Shape());
  }
  sweep.SetBounds(boundaries[0],boundaries[1]);
  sweep.SetTolerance(tolerance,tolerance,1e-5,1e-9);sweep.SetForceApproxC1(false);
  TopTools_MapOfShape reversed;BRepFill_DataMapOfShapeHArray2OfShape tapes,rails;
  sweep.Build(reversed,tapes,rails,BRepFill_Modified,GeomAbs_C2,GeomFill_Location,11,100);
  if(!sweep.IsDone()||!std::isfinite(sweep.ErrorOnSurface())||sweep.ErrorOnSurface()>tolerance)
    throw std::runtime_error("Tapered meridian sweep exceeded its native approximation tolerance");
  auto shell=TopoDS::Shell(sweep.Shape());
  const auto sections=sweep.Sections();
  if(sections.IsNull())throw std::runtime_error("Tapered meridian sweep has no boundary sections");
  BRep_Builder builder;
  for(int column:{sections->LowerCol(),sections->UpperCol()}) {
    BRepBuilderAPI_MakeWire boundary;
    for(int row=sections->LowerRow();row<=sections->UpperRow();++row) {
      const auto &shape=sections->Value(row,column);
      if(shape.IsNull()||shape.ShapeType()!=TopAbs_EDGE)
        throw std::runtime_error("Tapered meridian sweep boundary is incomplete");
      boundary.Add(TopoDS::Edge(shape));
    }
    if(!boundary.IsDone()||!boundary.Wire().Closed())
      throw std::runtime_error("Tapered meridian sweep boundary is not closed");
    BRepBuilderAPI_MakeFace makeCap(boundary.Wire(),true);
    if(!makeCap.IsDone())throw std::runtime_error("Tapered meridian sweep cap is not planar");
    auto cap=makeCap.Face();bool matched=false,reverse=false;
    for(TopExp_Explorer a(cap,TopAbs_EDGE);a.More();a.Next())
      for(TopExp_Explorer b(shell,TopAbs_EDGE);b.More();b.Next())if(a.Current().IsSame(b.Current())) {
        const bool same=a.Current().Orientation()==b.Current().Orientation();
        if(matched&&same!=reverse)throw std::runtime_error("Tapered meridian cap orientation is inconsistent");
        matched=true;reverse=same;
      }
    if(!matched)throw std::runtime_error("Tapered meridian cap does not share sweep edges");
    if(reverse)cap.Reverse();builder.Add(shell,cap);
  }
  shell.Closed(true);
  BRepBuilderAPI_MakeSolid makeSolid(shell);
  if(!makeSolid.IsDone())throw std::runtime_error("Tapered meridian sweep is not a solid");
  auto solid=makeSolid.Solid();
  solid.Orientation(TopAbs_FORWARD);solid.Closed(true);
  BRepClass3d_SolidClassifier orientation(solid);
  orientation.PerformInfinitePoint(tolerance);
  if(orientation.State()==TopAbs_IN) {
    shell.Reverse();BRepBuilderAPI_MakeSolid oriented(shell);
    if(!oriented.IsDone())throw std::runtime_error("Tapered meridian sweep orientation rebuild failed");
    solid=oriented.Solid();solid.Orientation(TopAbs_FORWARD);solid.Closed(true);
  } else if(orientation.State()!=TopAbs_OUT)throw std::runtime_error("Tapered meridian sweep orientation is ambiguous");
  MeridianSweep result{solid,{}};
  for(const auto &edge:edges) {
    const auto mapped=transform.ModifiedShape(edge);
    int index=0;
    for(int i=1;i<=section->NbLaw();++i)if(section->Edge(i).IsSame(mapped)){index=i;break;}
    if(index==0)throw std::runtime_error("Tapered meridian sweep lost profile ancestry");
    result.faces.emplace_back();
    for(TopExp_Explorer it(sweep.Tape(index),TopAbs_FACE);it.More();it.Next())result.faces.back().push_back(it.Current());
    if(result.faces.back().empty())throw std::runtime_error("Tapered meridian sweep lost generated faces");
  }
  return result;
}
}
bool thread_meridian_frame_probe() {
  const double pi=std::acos(-1.),lead=25.4/19.,phase=.37;
  double legacyError=0;unsigned samples=0;
  for(double k:{-1./32.,1./32.})for(double hand:{-1.,1.}) {
    const double alpha=std::atan(k),co=std::cos(alpha),norm=std::hypot(2*pi,lead/co);
    Handle(Geom_Surface) cone=new Geom_ConicalSurface(gp_Ax3(),alpha,4.);
    Handle(Geom2d_Line) line=new Geom2d_Line(gp_Pnt2d(phase,-lead/co),gp_Dir2d(hand*2*pi,lead/co));
    BRepBuilderAPI_MakeWire builder;
    // Late, nonzero original curve parameters catch accidental per-edge resets.
    for(int span=8;span<12;++span) {
      auto edge=BRepBuilderAPI_MakeEdge(line,cone,span*norm/2,(span+1)*norm/2).Edge();
      if(!BRepLib::BuildCurve3d(edge,1e-6))return false;
      builder.Add(edge);
    }
    const double omega=hand*2*pi/norm;
    auto location=meridian_location(builder.Wire(),phase,omega);
    Handle(GeomFill_CurveAndTrihedron) oldFrame=new GeomFill_CurveAndTrihedron(new GeomFill_ConstantBiNormal(gp_Dir(0,0,1)));
    Handle(BRepFill_Edge3DLaw) legacy=new BRepFill_Edge3DLaw(builder.Wire(),oldFrame);
    for(int i=1;i<=location->NbLaw();++i) {
      const auto &law=location->Law(i);double first,last;law->GetDomain(first,last);
      for(double fraction:{0.,.25,.5,.75,1.}) {
        const double t=first+(last-first)*fraction,u=phase+omega*t;
        gp_Mat matrix;gp_Vec translation;
        if(!law->D0(t,matrix,translation))return false;
        const gp_Vec expected[]={gp_Vec(-hand*std::cos(u),-hand*std::sin(u),0),gp_Vec(0,0,1),gp_Vec(-hand*std::sin(u),hand*std::cos(u),0)};
        for(int column=1;column<=3;++column)
          if((gp_Vec(matrix.Column(column))-expected[column-1]).Magnitude()>1e-12)return false;
        gp_XYZ offset(-hand*.37,.29,0);offset.Multiply(matrix);
        const auto point=gp_Vec(offset)+translation;
        const auto intended=translation+gp_Vec(.37*std::cos(u),.37*std::sin(u),.29);
        if((point-intended).Magnitude()>1e-12)return false;
        gp_Mat old;gp_Vec unused;if(!legacy->Law(i)->D0(t,old,unused))return false;
        legacyError=std::max(legacyError,(gp_Vec(old.Column(1))-expected[0]).Magnitude());
        ++samples;
      }
    }
  }
  return samples==80&&legacyError>1e-5;
}
static std::unique_ptr<Shape> build_helical_thread(const ThreadData &s,bool segmented,ThreadProfile &trace) {
 try {
  trace.start(segmented);
  auto measured=trace.mark();
  const double pi=std::acos(-1.),lead=s.pitch*s.starts;
  const double root=s.radius-s.depth,z0=-2*lead,z1=s.height+2*lead;
  double rootCenterZ=0,rootCenterRadius=0;
  const double lowerAngle=std::atan(s.lower_slope),upperAngle=std::atan(s.upper_slope);
  if(s.root_rounding>0) {
    if(s.rounding!=0||s.taper!=0||s.starts!=1||s.axial_flat||s.crest_limit!=0)
      throw std::runtime_error("Root-only rounding requires an unclipped parallel single-start flat-crest helix");
    const double intersection=(s.pitch-s.crest_width)/(s.lower_slope+s.upper_slope);
    const double half=(lowerAngle+upperAngle)/2,tilt=(upperAngle-lowerAngle)/2;
    rootCenterZ=s.crest_width/2+intersection*s.upper_slope-s.root_rounding*std::sin(tilt)/std::sin(half);
    rootCenterRadius=s.radius-intersection+s.root_rounding*std::cos(tilt)/std::sin(half);
    if(std::abs(rootCenterRadius-s.root_rounding-root)>s.tolerance*16)
      throw std::runtime_error("Root circle and core must share the same exact minimum radius");
  }
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
  trace.stage("core",measured);
  const double alpha=std::atan(s.taper),c=std::cos(alpha),sign=s.left?-1.:1.;
  Handle(Geom_Surface) surface;
  if(s.taper==0)surface=new Geom_CylindricalSurface(gp_Ax3(),root);
  else surface=new Geom_ConicalSurface(gp_Ax3(),alpha,root);
  for(uint32_t start=0;start<s.starts;++start) {
    measured=trace.mark();
    const double phase=2*pi*start/s.starts,z=-lead;
    Handle(Geom2d_Line) line=new Geom2d_Line(gp_Pnt2d(phase,z/c),gp_Dir2d(sign*2*pi,lead/c));
    const double end=(s.height+2*lead)/lead*std::hypot(2*pi,lead/c);
    // Keep the exact authored helix, extent and phase. New rounded sweeps use
    // half-turn parallel and quarter-turn tapered domains. Root-only asymmetric
    // sectors use quarter-turn domains to keep fused face wires valid. Wider trimmed
    // circular faces can fail native classification and Booleans despite
    // correct surfaces and volume. Flat profiles preserve
    // their established continuous representation and per-turn fallback.
    BRepBuilderAPI_MakeWire spineBuilder;
    const double turn=segmented?std::hypot(2*pi,lead/c)*((s.axial_flat||s.root_rounding>0)?.25:s.rounding>0?(s.taper==0?.5:.25):1.):end;
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
    trace.stage("spine",measured);
    const auto point=[&](double radius,double dz) {
      const double r=radius+s.taper*(z+dz);
      return gp_Pnt(r*std::cos(phase),r*std::sin(phase),z+dz);
    };
    const auto sweep=[&](const std::vector<TopoDS_Edge> &source,int rootHalf,int rootSector=-1) {
      auto measured=trace.mark();
      BRepBuilderAPI_MakeWire profile;std::vector<TopoDS_Edge> edges;
      for(const auto &edge:source){profile.Add(edge);edges.push_back(profile.Edge());}
      if(!profile.IsDone())throw std::runtime_error("Native thread profile is disconnected");
      TopoDS_Shape ridge;std::vector<std::vector<TopoDS_Shape>> generated;
      if((s.rounding>0||s.axial_flat)&&s.taper!=0) {
        auto exact=sweep_meridian(spine,profile.Wire(),edges,phase,
          sign*2*pi/std::hypot(2*pi,lead/c),root+s.taper*z,z,s.tolerance);
        ridge=exact.solid;generated=std::move(exact.faces);
      } else {
        BRepOffsetAPI_MakePipeShell pipe(spine);
        pipe.SetMode(gp_Dir(0,0,1));pipe.SetTolerance(s.tolerance,s.tolerance,1e-9);
        pipe.Add(profile.Wire(),false,false);pipe.Build();
        if(!pipe.IsDone()||!pipe.MakeSolid())throw std::runtime_error("Native helical ridge construction failed");
        ridge=pipe.Shape();
        for(const auto &edge:edges) {
          generated.emplace_back();
          for(const auto &face:pipe.Generated(edge))generated.back().push_back(face);
        }
      }
      if(!BRepCheck_Analyzer(ridge).IsValid())
        throw std::runtime_error("Native helical ridge is not a valid solid");
      trace.stage("sweep_and_validity",measured);
      measured=trace.mark();
      for(uint32_t i=0;i<edges.size();++i)for(const auto &face:generated[i])
        if(face.ShapeType()==TopAbs_FACE) {
          if(rootSector>=0) {
            if((rootSector==0&&i==2)||(rootSector==1&&i==1))ancestry.push_back({face,static_cast<uint32_t>(rootSector),4});
            else if(rootSector==0&&i==1)ancestry.push_back({face,start*4+2,2});
            else if(rootSector==1&&i==2)ancestry.push_back({face,start*4,2});
            // Closure edges must be buried. A surviving closure keeps an
            // unnameable source so Rust refuses it instead of calling it root.
            else ancestry.push_back({face,start*4+3,2});
          }
          else if(rootHalf>=0&&i==0)ancestry.push_back({face,static_cast<uint32_t>(rootHalf),4});
          else ancestry.push_back({face,start*4+(rootHalf>=0?3:i),2});
        }
      BRepAlgoAPI_Fuse join;boolean_inputs(join,solid,ridge);
      ancestry=carry_thread_faces(join,ancestry);solid=join.Shape();
      trace.stage("ridge_fuse_and_history",measured);
    };
    const auto edge=[](const gp_Pnt &a,const gp_Pnt &b){return BRepBuilderAPI_MakeEdge(a,b).Edge();};
    const auto arc=[](const gp_Pnt &a,const gp_Pnt &mid,const gp_Pnt &b){
      GC_MakeArcOfCircle circle(a,mid,b);
      if(!circle.IsDone())throw std::runtime_error("Native rounded thread arc construction failed");
      return BRepBuilderAPI_MakeEdge(circle.Value()).Edge();
    };
    if(s.rounding==0) {
      const gp_Pnt points[]={
        point(root-s.overlap,-s.crest_width/2-(s.depth+s.overlap)*s.lower_slope),
        point(s.radius,-s.crest_width/2),
        point(s.radius,s.crest_width/2),
        point(root-s.overlap,s.crest_width/2+(s.depth+s.overlap)*s.upper_slope)
      };
      std::vector<TopoDS_Edge> edges;
      for(uint32_t i=0;i<4;++i)edges.push_back(edge(points[i],points[(i+1)%4]));
      sweep(edges,-1);
      if(s.root_rounding>0) {
        const double left=-(pi/2-upperAngle),right=pi/2-lowerAngle;
        const auto circle=[&](double angle) {
          return point(rootCenterRadius-s.root_rounding*std::cos(angle),rootCenterZ+s.root_rounding*std::sin(angle));
        };
        const auto a=circle(left),b=circle(0),d=circle(right);
        const double az=rootCenterZ+s.root_rounding*std::sin(left),dz=rootCenterZ+s.root_rounding*std::sin(right);
        const double extend=std::min({s.pitch*.02,(az-s.crest_width/2)*.25,(s.pitch-s.crest_width/2-dz)*.25});
        const auto before=point(rootCenterRadius-s.root_rounding*std::sin(upperAngle)+extend/s.upper_slope,az-extend);
        const auto after=point(rootCenterRadius-s.root_rounding*std::sin(lowerAngle)+extend/s.lower_slope,dz+extend);
        const auto belowA=point(root-s.overlap,az-extend),belowD=point(root-s.overlap,dz+extend);
        const auto belowB=point(root-s.overlap,rootCenterZ);
        sweep({edge(belowA,before),edge(before,a),arc(a,circle(left/2),b),edge(b,belowB),edge(belowB,belowA)},-1,0);
        sweep({edge(belowB,b),arc(b,circle(right/2),d),edge(d,after),edge(after,belowD),edge(belowD,belowB)},-1,1);
      }
    } else if(s.taper==0) {
      // Full-period closed profiles self-contact at their valley seam. Sweep
      // three narrower exact profiles instead: the crest/flanks and the two
      // circular root shoulders. All closure edges are buried in the core.
      const double theta=std::atan(s.lower_slope),r=s.rounding;
      const double half=r*std::cos(theta),drop=r*(1-std::sin(theta));
      const double foot=half+(s.depth-drop+s.overlap)*s.lower_slope;
      const auto a=point(root-s.overlap,-foot),b=point(s.radius-drop,-half);
      const auto c=point(s.radius-drop,half),d=point(root-s.overlap,foot);
      sweep({edge(a,b),arc(b,point(s.radius,0),c),edge(c,d),edge(d,a)},-1);
      const double midAngle=(pi/2-theta)/2;
      for(int side=0;side<2;++side) {
        const double sign=side==0?-1.:1.,valley=sign*s.pitch/2;
        const auto low=point(root,valley);
        const auto high=point(root+drop,valley-sign*half);
        const auto middle=point(root+r*(1-std::cos(midAngle)),valley-sign*r*std::sin(midAngle));
        const auto start=side==0?low:high,finish=side==0?high:low;
        const double startZ=side==0?valley:valley-half,finishZ=side==0?valley+half:valley;
        const auto underEnd=point(root-s.overlap,finishZ),underStart=point(root-s.overlap,startZ);
        sweep({arc(start,middle,finish),edge(finish,underEnd),edge(underEnd,underStart),edge(underStart,start)},side);
      }
    } else {
      // ISO 7 tapered axial form: preserve actual tangent circles and axial
      // flank angles. Adding k*dz to a parallel circle would shear it.
      const double theta=std::atan(s.lower_slope),r=s.rounding,k=s.taper;
      const double m=1/s.lower_slope,q=std::sqrt(1+k*k);
      const double H=s.pitch*(m*m-k*k)/(2*m),half=r*std::cos(theta);
      const double center=s.radius-r*q,tangent=center+r*std::sin(theta);
      const auto exact=[&](double radius,double dz) {
        return gp_Pnt((radius+k*z)*std::cos(phase),(radius+k*z)*std::sin(phase),z+dz);
      };
      const double left=(root-s.overlap-tangent-m*half)/(m-k);
      const double right=(tangent+m*half-root+s.overlap)/(m+k);
      const auto a=exact(root+k*left-s.overlap,left),b=exact(tangent,-half);
      const auto c=exact(tangent,half),d=exact(root+k*right-s.overlap,right);
      sweep({edge(a,b),arc(b,exact(center+r,0),c),edge(c,d),edge(d,a)},-1);
      for(int side=0;side<2;++side) {
        const double direction=side==0?1.:-1.;
        const double valley=side==0?-H/(m-k):H/(m+k);
        const double rootCenter=root+k*valley+r*q;
        const double lowAngle=std::atan(k),highAngle=direction*(pi/2-theta);
        const auto circle=[&](double angle) {
          return exact(rootCenter-r*std::cos(angle),valley+r*std::sin(angle));
        };
        const auto low=circle(lowAngle),high=circle(highAngle);
        const auto start=side==0?low:high,finish=side==0?high:low;
        const double startZ=side==0?valley+r*std::sin(lowAngle):valley-half;
        const double finishZ=side==0?valley+half:valley+r*std::sin(lowAngle);
        const auto underStart=exact(root+k*startZ-s.overlap,startZ);
        const auto underEnd=exact(root+k*finishZ-s.overlap,finishZ);
        sweep({arc(start,circle((lowAngle+highAngle)/2),finish),edge(finish,underEnd),
          edge(underEnd,underStart),edge(underStart,start)},side);
      }
    }
  }
  // Joining before trimming is essential: a separately trimmed ridge can be
  // silently lost by a later union despite a valid, one-solid core result.
  if(s.crest_limit>0) {
    if(!s.axial_flat||s.starts!=1)throw std::runtime_error("Crest stock clipping requires a single axial-flat helix");
    const auto stock=BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0,0,-3*lead),gp_Dir(0,0,1)),s.crest_limit,s.height+6*lead).Shape();
    for(TopExp_Explorer it(stock,TopAbs_FACE);it.More();it.Next())
      if(BRepAdaptor_Surface(TopoDS::Face(it.Current())).GetType()==GeomAbs_Cylinder)
        ancestry.push_back({it.Current(),1,2});
    BRepAlgoAPI_Common clip;boolean_inputs(clip,solid,stock);
    ancestry=carry_thread_faces(clip,ancestry);solid=clip.Shape();
  }
  const double bound=s.radius+std::abs(s.taper)*(s.height+3*lead)+s.pitch;
  measured=trace.mark();
  const auto limit=BRepPrimAPI_MakeBox(gp_Pnt(-bound,-bound,0),2*bound,2*bound,s.height).Shape();
  if(s.rounding>0||s.root_rounding>0||s.axial_flat) {
    int count=0;for(TopExp_Explorer it(solid,TopAbs_SOLID);it.More();it.Next())++count;
    if(count!=1||!BRepCheck_Analyzer(solid).IsValid())
      throw std::runtime_error("Untrimmed rounded thread is not one valid solid");
  }
  BRepAlgoAPI_Common trim;boolean_inputs(trim,solid,limit);
  ancestry=carry_thread_faces(trim,ancestry);solid=trim.Shape();
  if(!BRepCheck_Analyzer(solid).IsValid())throw std::runtime_error("Finished thread is invalid");
  int count=0;for(TopExp_Explorer it(solid,TopAbs_SOLID);it.More();it.Next())++count;
  if(count!=1)throw std::runtime_error("Thread must remain one connected solid");
  trace.stage("trim_and_validity",measured);
  measured=trace.mark();
  // Validate physical crests and gaps, not merely a successful Boolean call.
  // Probe every quarter-pitch band and the original regression sample. This
  // checks the whole axial extent, rather than accepting one surviving crest.
  const uint32_t bands=static_cast<uint32_t>(std::ceil(4*s.height/s.pitch));
  BRepClass3d_SolidClassifier classifier(solid);
  for(uint32_t band=0;band<=bands;++band) {
    const double sampleZ=band==bands?s.height*.47:s.height*(band+.5)/bands;
    for(uint32_t start=0;start<s.starts;++start) {
      const double angle=sign*2*pi*sampleZ/lead+2*pi*start/s.starts;
      const double coneCrest=s.radius+s.taper*sampleZ;
      const double crest=s.crest_limit>0?std::min(coneCrest,s.crest_limit):coneCrest;
      const double radius=crest-std::min(s.depth*.05,s.crest_width*.1);
      const auto state=[&](double a){classifier.Perform(gp_Pnt(radius*std::cos(a),radius*std::sin(a),sampleZ),s.tolerance*4);return classifier.State();};
      const auto inside=state(angle);
      const auto outside=inside==TopAbs_IN?state(angle+pi/s.starts):TopAbs_UNKNOWN;
      if(inside!=TopAbs_IN||outside!=TopAbs_OUT) {
        trace.probe("crest_gap",sampleZ,angle,radius,inside,outside);
        trace.stage("physical_guards_failed",measured);
        throw ThreadRidgeLoss();
      }
      if(s.rounding>0) {
        const double half=s.rounding*std::cos(std::atan(s.lower_slope));
        const double epsilon=s.taper==0?std::max(s.tolerance*8,s.rounding*.01):s.tolerance*16;
        for(double direction:{-1.,1.}) {
          const double dz=direction*.65*half;
          const double circle=std::sqrt(s.rounding*s.rounding-dz*dz);
          for(int valley=0;valley<2;++valley) {
            const double m=1/s.lower_slope,k=s.taper;
            const double q=std::sqrt(1+k*k),H=s.pitch*(m*m-k*k)/(2*m);
            const double boundary=valley?root+k*(sampleZ-dz)+s.rounding*q-circle:
              s.radius+k*(sampleZ-dz)-s.rounding*q+circle;
            const double phase=sign*2*pi*(sampleZ-dz-valley*H/(m+k))/lead;
            const auto material=[&](double r){classifier.Perform(gp_Pnt(r*std::cos(phase),r*std::sin(phase),sampleZ),s.tolerance*4);return classifier.State();};
            const auto circleInside=material(boundary-epsilon),circleOutside=material(boundary+epsilon);
            if(circleInside!=TopAbs_IN||circleOutside!=TopAbs_OUT) {
              trace.probe(valley?"root_circle":"crest_circle",sampleZ,phase,boundary,circleInside,circleOutside);
              throw ThreadRidgeLoss();
            }
          }
        }
      }
      if(s.root_rounding>0) {
        const double epsilon=std::max(s.tolerance*16,s.root_rounding*.01);
        for(double arcAngle:{-.65*(pi/2-upperAngle),.65*(pi/2-lowerAngle)}) {
          const double relativeZ=rootCenterZ+s.root_rounding*std::sin(arcAngle);
          const double boundary=rootCenterRadius-s.root_rounding*std::cos(arcAngle);
          const double phase=sign*2*pi*(sampleZ-relativeZ)/lead;
          const auto material=[&](double r){classifier.Perform(gp_Pnt(r*std::cos(phase),r*std::sin(phase),sampleZ),s.tolerance*4);return classifier.State();};
          const auto circleInside=material(boundary-epsilon),circleOutside=material(boundary+epsilon);
          if(circleInside!=TopAbs_IN||circleOutside!=TopAbs_OUT) {
            trace.probe("asymmetric_root_circle",sampleZ,phase,boundary,circleInside,circleOutside);
            throw ThreadRidgeLoss();
          }
        }
      }
    }
  }
  trace.stage("physical_guards",measured);
  measured=trace.mark();
  auto result=std::make_unique<Shape>(Shape{solid,{},{}});
  // The final trimmed solid passed native BRepCheck above; later ancestry
  // bookkeeping does not modify geometry.
  record_validated_shape(*result);
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
  trace.stage("ancestry",measured);
  return result;
 } catch(const Standard_Failure &e) {
  throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native thread operation failed");
 }
}
static std::unique_ptr<Shape> helical_thread_impl(const ThreadData &s,bool useCache,bool request=true) {
  auto &cache=thread_cache();
  if(request)++cache.stats.requests;
  const auto key=thread_key(s);
  if(auto found=cache.entries.find(key);useCache&&found!=cache.entries.end()) {
    ++cache.stats.hits;
    found->second.used=++cache.clock;
    return thread_wrapper(*found->second.shape);
  }
  ++cache.stats.constructions;
  ThreadProfile trace(s);
  const auto completed=[&](std::unique_ptr<Shape> result) {
    // A tag is only eligibility for later retention, not an acceptance proof.
    // Rust must still adopt, validate its policy and bind all owner names.
    try {result->thread_factory_tag=std::make_shared<ThreadFactoryTag>(ThreadFactoryTag{key,result->value});}
    catch(const std::bad_alloc &) {++cache.stats.unretained;}
    return result;
  };
  // Rounded profiles are new geometry. Use bounded circular sweep faces from the
  // outset, avoiding invalid continuous unions and false trimmed-solid native
  // classification. Flat profiles retain their established representation and
  // narrowly typed reconstruction rules below.
  if(s.rounding>0||s.root_rounding>0||s.axial_flat) {
    ++cache.stats.bounded_attempts;
    auto result=build_helical_thread(s,true,trace);
    const auto measured=trace.mark();
    volume(*result);
    trace.stage("volume_guard",measured);
    return completed(std::move(result));
  }
  try {
    ++cache.stats.continuous_attempts;
    auto result=build_helical_thread(s,false,trace);
    const auto measured=trace.mark();
    volume(*result);
    trace.stage("volume_guard",measured);
    return completed(std::move(result));
  } catch(const ThreadRidgeLoss &) {
    trace.retry("ridge_loss");
    // Retry a proven physical clipping failure on the same authored helix.
  } catch(const VolumeIntegrationFailure &) {
    trace.retry("volume_nonconvergence");
    // The continuous B-rep passed the physical guards but cannot satisfy the
    // shared volume integration guard. Reconstruct the same helix with bounded
    // faces. Other native, validity and ancestry errors are never retried.
  }
  ++cache.stats.bounded_attempts;
  auto result=build_helical_thread(s,true,trace);
  const auto measured=trace.mark();
  volume(*result);
  trace.stage("volume_guard",measured);
  return completed(std::move(result));
}
std::unique_ptr<Shape> helical_thread(const ThreadData &s) {return helical_thread_impl(s,true);}
std::unique_ptr<Shape> helical_thread_fresh(const ThreadData &s) {return helical_thread_impl(s,false);}
std::unique_ptr<Shape> cached_helical_thread(const ThreadData &s) {
  auto &cache=thread_cache();++cache.stats.requests;
  if(auto found=cache.entries.find(thread_key(s));found!=cache.entries.end()) {
    ++cache.stats.hits;found->second.used=++cache.clock;
    return thread_wrapper(*found->second.shape);
  }
  return nullptr;
}
// The constructor has already recorded its request through the cache lookup.
// Asset restoration is not a native construction or a continuous/bounded try.
std::unique_ptr<Shape> helical_thread_after_miss(const ThreadData &s) {return helical_thread_impl(s,false,false);}
ThreadFactoryStats thread_factory_stats() {
  const auto &cache=thread_cache();
  auto stats=cache.stats;
  stats.entries=cache.entries.size();stats.serialized_bytes=cache.bytes;
  return stats;
}
bool remember_validated_thread(const Shape &shape) {
  auto &cache=thread_cache();
  const auto &tag=shape.thread_factory_tag;
  if(!tag||!shape.value.IsEqual(tag->original))return false;
  if(auto found=cache.entries.find(tag->key);found!=cache.entries.end()) {
    found->second.used=++cache.clock;return true;
  }
  if(!cache.entry_limit||!cache.byte_limit){++cache.stats.unretained;return false;}
  CountingStream buffer(cache.byte_limit);std::ostream stream(&buffer);
  try {BRepTools::Write(shape.value,stream);}
  catch(const Standard_Failure &) {++cache.stats.unretained;return false;}
  const uint64_t bytes=buffer.bytes();
  if(!stream.good()||!bytes||bytes>cache.byte_limit){++cache.stats.unretained;return false;}
  // Prepare the independent wrapper before evicting any successful entry.
  auto neutral=thread_wrapper(shape);
  while(cache.entries.size()>=cache.entry_limit||cache.bytes>cache.byte_limit-bytes) {
    auto oldest=std::min_element(cache.entries.begin(),cache.entries.end(),[](const auto &a,const auto &b){return a.second.used<b.second.used;});
    cache.bytes-=oldest->second.bytes;cache.entries.erase(oldest);++cache.stats.evictions;
  }
  cache.entries.emplace(tag->key,ThreadEntry{std::move(neutral),bytes,++cache.clock});
  cache.bytes+=bytes;
  return true;
}
bool remember_thread_prototype(const Shape &shape,const ThreadData &s) {
  // Only the private Rust compiled-asset path invokes this after exact digest,
  // source/policy, geometry/role receipt, adoption and new-owner naming checks.
  // The original wrapper remains untouched; project/import APIs never call it.
  auto neutral=thread_wrapper(shape);
  neutral->thread_factory_tag=std::make_shared<ThreadFactoryTag>(ThreadFactoryTag{thread_key(s),shape.value});
  return remember_validated_thread(*neutral);
}
void reset_thread_factory_cache() {thread_cache()=ThreadCache{};}
void thread_factory_test_limits(uint32_t entries,uint64_t bytes) {
  if(entries>maxThreadEntries||bytes>maxThreadBytes)throw std::runtime_error("Thread test cache cannot exceed production limits");
  reset_thread_factory_cache();
  auto &cache=thread_cache();cache.entry_limit=entries;cache.byte_limit=bytes;
}
bool thread_factory_contains(const ThreadData &s) {return thread_cache().entries.find(thread_key(s))!=thread_cache().entries.end();}
}
