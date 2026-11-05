#include "worm.hpp"
#include "threads.hpp"
#include "worm_verification.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepLib.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Builder.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_BSplineSurface.hxx>
#include <Geom_BezierSurface.hxx>
#include <GeomConvert.hxx>
#include <Standard_Failure.hxx>
#include <TColgp_Array2OfPnt.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Pln.hxx>
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <functional>
#include <limits>
#include <stdexcept>
#include <vector>

namespace multicad::kernel {
namespace {
constexpr double pi=3.14159265358979323846;
constexpr int degree=16,size=degree+1;
const double slope=std::tan(20*pi/180)*std::sqrt(1.01);
constexpr double lead_parameter=.5;
using V=std::array<double,3>;
using Matrix=std::array<std::array<double,size>,size>;
using Surface=Handle(Geom_BSplineSurface);
using I=worm_verification::Interval;
struct Recipe {
  double module,radius,center,half,width,tolerance;
  explicit Recipe(const WormWheelData&s):module(s.module),radius(s.teeth/2.),center(5+radius),
    half(pi/4+s.thinning/(2*s.module)),width((s.width+16*s.tolerance)/s.module),tolerance(s.tolerance) {
    if(!std::isfinite(s.module)||s.module<.5||s.module>2||s.teeth<32||s.teeth>64||
       !std::isfinite(s.width)||s.width<.5*s.module||s.width>2*s.module||width>2.002||
       !std::isfinite(s.bore)||s.bore<0||s.bore>(radius-1.25)*s.module-s.minimum||
       !std::isfinite(s.thinning)||s.thinning<0||s.thinning>.1*s.module||
       !std::isfinite(s.tolerance)||s.tolerance<=0||!std::isfinite(s.minimum)||s.minimum<=0)
      throw std::runtime_error("ZA wheel is outside its declared single-start q10/20-degree envelope");
  }
};
double square_root(double x){return std::sqrt(x);}
I square_root(I x){return worm_verification::sqrt(x);}
double sine(double x){return std::sin(x);}
I sine(I x){return worm_verification::sin(x);}
double cosine(double x){return std::cos(x);}
I cosine(I x){return worm_verification::cos(x);}
double arcsine(double x){return std::asin(x);}
I arcsine(I x){return worm_verification::asin(x);}
template<class T>using Vec=std::array<T,3>;
template<class T>Vec<T> rotate(Vec<T> p,T phase){const T c=cosine(phase),s=sine(phase);return{c*p[0]-s*p[1],s*p[0]+c*p[1],p[2]};}
template<class T>struct Contact{T q,z,w;};
template<class T>Contact<T> contact(const Recipe&s,T r,T v,int side){
  const T q=square_root(r*r-v*v),k=T(-side*slope);
  const T z=T(side)*(T(s.half)-T(slope)*(r-T(5)))-T(lead_parameter)*arcsine(v/r);
  const T nx=T(lead_parameter)*v/r+k*q;
  return{q,z,r*(q-T(5))/nx};
}
template<class T>Vec<T> flank(const Recipe&s,T r,T v,int side){const auto c=contact(s,r,v,side);return rotate(Vec<T>{T(s.center)-c.q,c.w,v},-(c.z+c.w)/T(s.radius));}
template<class T>Vec<T> tip_edge(const Recipe&s,T u,T v,int side){const auto c=contact(s,T(6.25),v,side);const T travel=(T(1)-u)*c.w;return rotate(Vec<T>{T(s.center)-c.q,travel,v},-(c.z+travel)/T(s.radius));}
template<class T>Vec<T> root_gap(const Recipe&s,T u,T v){const T q=square_root(T(6.25*6.25)-v*v),crest=T(s.half-1.25*slope);const T phase=T(lead_parameter/s.radius)*arcsine(v/T(6.25))+(T(2)*u-T(1))*crest/T(s.radius);return rotate(Vec<T>{T(s.center)-q,T(0),v},phase);}
template<class T>Vec<T> outer_closure(const Recipe&s,T u,T v){const auto a=flank(s,T(3.75),v,1),b=flank(s,T(3.75),v,-1);return{(T(1)-u)*a[0]+u*b[0],(T(1)-u)*a[1]+u*b[1],v};}
// The closed cutting space has genuine generated flanks and swept sharp-tip
// transitions. Its temporary outside closure is beyond the retained blank.
template<class T>Vec<T> patch(const Recipe&s,int kind,T u,T t){
  const T v=(t-T(.5))*T(s.width);Vec<T> p;
  switch(kind){
    case 0:p=flank(s,T(3.75)+T(2.5)*u,v,1);break;
    case 1:p=tip_edge(s,u,v,1);break;
    case 2:p=root_gap(s,u,v);break;
    case 3:p=tip_edge(s,T(1)-u,v,-1);break;
    case 4:p=flank(s,T(6.25)-T(2.5)*u,v,-1);break;
    case 5:p=outer_closure(s,T(1)-u,v);break;
    default:throw std::runtime_error("Unknown generated ZA boundary");
  }
  for(auto&x:p)x=x*T(s.module);return p;
}
struct Interpolation {
  std::array<double,size> nodes{};
  Matrix inverse{};
  Interpolation(){
    Matrix b{};for(int i=0;i<size;++i){nodes[i]=(1-std::cos(pi*i/degree))/2;inverse[i][i]=1;std::uint64_t binomial=1;for(int j=0;j<size;++j){if(j)binomial=binomial*std::uint64_t(degree+1-j)/std::uint64_t(j);b[i][j]=double(binomial)*std::pow(nodes[i],j)*std::pow(1-nodes[i],degree-j);}}
    for(int j=0;j<size;++j){int best=j;for(int i=j+1;i<size;++i)if(std::abs(b[i][j])>std::abs(b[best][j]))best=i;std::swap(b[j],b[best]);std::swap(inverse[j],inverse[best]);const double pivot=b[j][j];if(!std::isfinite(pivot)||std::abs(pivot)<1e-12)throw std::runtime_error("ZA interpolation matrix is singular");for(int l=0;l<size;++l){b[j][l]/=pivot;inverse[j][l]/=pivot;}for(int i=0;i<size;++i)if(i!=j){const double f=b[i][j];for(int l=0;l<size;++l){b[i][l]-=f*b[j][l];inverse[i][l]-=f*inverse[j][l];}}}
  }
};
bool profiling(){const char*p=std::getenv("MULTICAD_NATIVE_WORM_PROFILE");return p&&std::strcmp(p,"1")==0;}
Surface approximation(const Recipe&s,int kind){
  static const Interpolation table;
  const auto center=patch(s,kind,.5,.5);
  worm_verification::Lattice<size> values{},temporary{},poles{};
  for(int i=0;i<size;++i)for(int j=0;j<size;++j){values[i][j]=patch(s,kind,table.nodes[i],table.nodes[j]);for(int c=0;c<2;++c)values[i][j][c]-=center[c];}
  for(int i=0;i<size;++i)for(int j=0;j<size;++j)for(int l=0;l<size;++l)for(int c=0;c<2;++c)temporary[i][j][c]+=table.inverse[i][l]*values[l][j][c];
  for(int i=0;i<size;++i)for(int j=0;j<size;++j){for(int l=0;l<size;++l)for(int c=0;c<2;++c)poles[i][j][c]+=temporary[i][l][c]*table.inverse[j][l];for(int c=0;c<2;++c)poles[i][j][c]+=center[c];poles[i][j][2]=(double(j)/degree-.5)*s.width*s.module;for(double x:poles[i][j])if(!std::isfinite(x)||std::abs(x)>128*s.module)throw std::runtime_error("ZA interpolation control point exceeds its bounded chart");}
  const auto certificate=worm_verification::verify(poles,[&](I u,I v){return patch(s,kind,u,v);},s.module,s.tolerance);
  TColgp_Array2OfPnt native(1,size,1,size);for(int i=0;i<size;++i)for(int j=0;j<size;++j)native.SetValue(i+1,j+1,gp_Pnt(poles[i][j][0],poles[i][j][1],poles[i][j][2]));
  Handle(Geom_BezierSurface) bezier=new Geom_BezierSurface(native);
  auto result=GeomConvert::SurfaceToBSplineSurface(bezier);
  // A half-span in each direction stabilizes OCCT's cached polynomial
  // evaluator. This changes the basis, not the represented polynomial.
  result->InsertUKnot(.5,1,1e-12);result->InsertVKnot(.5,1,1e-12);
  if(result->IsURational()||result->IsVRational()||result->NbUPoles()!=size+1||result->NbVPoles()!=size+1||result->NbUKnots()!=3||result->NbVKnots()!=3)
    throw std::runtime_error("ZA native half-knot structure changed");
  worm_verification::Lattice<size+1> refined{};for(int i=0;i<size+1;++i)for(int j=0;j<size+1;++j){const auto p=result->Pole(i+1,j+1);refined[i][j]={p.X(),p.Y(),p.Z()};}
  std::array<double,3> uk{},vk{};std::array<int,3> um{},vm{};for(int i=0;i<3;++i){uk[i]=result->UKnot(i+1);vk[i]=result->VKnot(i+1);um[i]=result->UMultiplicity(i+1);vm[i]=result->VMultiplicity(i+1);}
  const auto refinement=worm_verification::verifyHalfKnots(poles,refined,uk,um,vk,vm,result->UDegree(),result->VDegree());
  const double bound=worm_verification::combined_bound(certificate,refinement,s.tolerance);
  if(profiling())std::fprintf(stderr,"multicad.native.worm patch=%d node_residual_mm=%.17g source_bound_mm=%.17g\n",kind,certificate.max_node_l2,bound);
  return result;
}
struct NamedFace{TopoDS_Face face;std::uint32_t source;std::uint8_t role;};
using Faces=std::vector<NamedFace>;
void add_modified(Faces&out,const TopoDS_Shape&shape,std::uint32_t source,std::uint8_t role){if(shape.ShapeType()==TopAbs_FACE)out.push_back({TopoDS::Face(shape),source,role});else for(TopExp_Explorer e(shape,TopAbs_FACE);e.More();e.Next())out.push_back({TopoDS::Face(e.Current()),source,role});}
template<class Operation>Faces carry(Operation&operation,const Faces&input){Faces out;for(const auto&f:input){if(!operation.IsDeleted(f.face))out.push_back(f);for(const auto&x:operation.Modified(f.face))add_modified(out,x,f.source,f.role);for(const auto&x:operation.Generated(f.face))if(x.ShapeType()==TopAbs_FACE)add_modified(out,x,f.source,f.role);}return out;}
Faces after_sewing(BRepBuilderAPI_Sewing&sewing,const Faces&input){Faces out;for(const auto&f:input)add_modified(out,sewing.IsModified(f.face)?sewing.Modified(f.face):f.face,f.source,f.role);return out;}
std::unique_ptr<Shape> finish(const TopoDS_Shape&shape,const Faces&sources){
  if(!BRepCheck_Analyzer(shape).IsValid())throw std::runtime_error("Generated ZA native boundary is invalid");int solids=0;for(TopExp_Explorer e(shape,TopAbs_SOLID);e.More();e.Next())++solids;if(solids!=1)throw std::runtime_error("Generated ZA geometry must be one connected solid");
  auto result=std::make_unique<Shape>(Shape{shape,{},{}});TopTools_IndexedMapOfShape faces;TopExp::MapShapes(shape,TopAbs_FACE,faces);
  for(int i=1;i<=faces.Extent();++i){bool found=false;for(const auto&source:sources)if(source.face.IsSame(faces(i))){NativeSeed seed{std::uint32_t(i-1),source.source,source.role};bool duplicate=false;for(const auto&known:result->seeds)if(known.face==seed.face&&known.source_index==seed.source_index&&known.role==seed.role)duplicate=true;if(!duplicate)result->seeds.push_back(seed);found=true;}if(!found)throw std::runtime_error("Generated ZA face has no native construction history");}
  return result;
}
Faces seed_faces(const Shape&s){TopTools_IndexedMapOfShape faces;TopExp::MapShapes(s.value,TopAbs_FACE,faces);Faces out;for(const auto&seed:s.seeds){if(seed.face>=std::uint32_t(faces.Extent()))throw std::runtime_error("ZA input ancestry names an absent face");out.push_back({TopoDS::Face(faces(seed.face+1)),seed.source_index,seed.role});}return out;}
std::unique_ptr<Shape> bore(std::unique_ptr<Shape>input,double radius,double bottom,double length,std::uint8_t role){
  if(radius==0)return input;
  const auto tool=BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0,0,bottom-1),gp_Dir(0,0,1)),radius,length+2).Shape();TopTools_ListOfShape args,tools;args.Append(input->value);tools.Append(tool);BRepAlgoAPI_Cut cut;cut.SetNonDestructive(true);cut.SetArguments(args);cut.SetTools(tools);cut.Build();if(!cut.IsDone())throw std::runtime_error("ZA bore subtraction failed");auto sources=carry(cut,seed_faces(*input));
  for(TopExp_Explorer f(tool,TopAbs_FACE);f.More();f.Next())for(const auto&actual:cut.Modified(f.Current()))if(actual.ShapeType()==TopAbs_FACE&&BRepAdaptor_Surface(TopoDS::Face(actual)).GetType()==GeomAbs_Cylinder)sources.push_back({TopoDS::Face(actual),0,role});
  return finish(cut.Shape(),sources);
}
struct Space{TopoDS_Shape shape;Faces sources;};
Space space(const Recipe&s){
  std::array<Surface,6> surfaces;for(int i=0;i<6;++i)surfaces[i]=approximation(s,i);
  std::array<std::array<TopoDS_Vertex,2>,6> vertices;std::array<TopoDS_Edge,6> along,bottom,top;BRep_Builder topology;
  for(int i=0;i<6;++i)for(int end=0;end<2;++end){vertices[i][end]=BRepBuilderAPI_MakeVertex(surfaces[i]->Value(1,end));topology.UpdateVertex(vertices[i][end],s.tolerance);}
  for(int i=0;i<6;++i){const int previous=(i+5)%6;along[i]=BRepBuilderAPI_MakeEdge(surfaces[i]->UIso(1),vertices[i][0],vertices[i][1],0,1);bottom[i]=BRepBuilderAPI_MakeEdge(surfaces[i]->VIso(0),vertices[previous][0],vertices[i][0],0,1);top[i]=BRepBuilderAPI_MakeEdge(surfaces[i]->VIso(1),vertices[previous][1],vertices[i][1],0,1);}
  auto uv=[&](const TopoDS_Edge&e,const Surface&surface,double x,double y,double dx,double dy){Handle(Geom2d_Line)c=new Geom2d_Line(gp_Pnt2d(x,y),gp_Dir2d(dx,dy));topology.UpdateEdge(e,c,surface,TopLoc_Location(),s.tolerance);topology.Range(e,surface,TopLoc_Location(),0,1);topology.SameParameter(e,true);topology.SameRange(e,true);};
  BRepBuilderAPI_Sewing sewing(s.tolerance,true,true,false,false);sewing.SetMaxTolerance(s.tolerance);Faces sources;std::array<BRepBuilderAPI_MakeWire,2> caps;
  for(int i=0;i<6;++i){const int previous=(i+5)%6;const auto&surface=surfaces[i];uv(bottom[i],surface,0,0,1,0);uv(along[i],surface,1,0,0,1);uv(top[i],surface,0,1,1,0);uv(along[previous],surface,0,0,0,1);BRepBuilderAPI_MakeWire wire;wire.Add(bottom[i]);wire.Add(along[i]);wire.Add(TopoDS::Edge(top[i].Reversed()));wire.Add(TopoDS::Edge(along[previous].Reversed()));if(!wire.IsDone())throw std::runtime_error("ZA generated-space wire did not close");BRepBuilderAPI_MakeFace face(surface,wire.Wire(),true);if(!face.IsDone())throw std::runtime_error("ZA generated-space face failed");sewing.Add(face.Face());sources.push_back({face.Face(),std::uint32_t(i),6});caps[0].Add(bottom[i]);caps[1].Add(top[i]);}
  for(int end=0;end<2;++end){BRepBuilderAPI_MakeFace face(gp_Pln(gp_Pnt(0,0,(end-.5)*s.width*s.module),gp_Dir(0,0,1)),caps[end].Wire(),true);if(!face.IsDone())throw std::runtime_error("ZA cutter axial cap failed");sewing.Add(face.Face());sources.push_back({face.Face(),0,std::uint8_t(end)});}
  sewing.Perform();const auto sewn=sewing.SewedShape();if(sewn.ShapeType()!=TopAbs_SHELL||sewing.NbFreeEdges()||sewing.NbMultipleEdges()||sewing.NbDeletedFaces())throw std::runtime_error("ZA generated-space shell is not watertight");BRepBuilderAPI_MakeSolid made(TopoDS::Shell(sewn));auto solid=made.Solid();if(!BRepLib::OrientClosedSolid(solid)||!BRepCheck_Analyzer(solid).IsValid())throw std::runtime_error("ZA generated-space solid is invalid");return{solid,after_sewing(sewing,sources)};
}
std::unique_ptr<Shape> wheel(const WormWheelData&data){
  const auto started=std::chrono::steady_clock::now();const Recipe s(data);const auto tool=space(s);
  const auto wedge=BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0,0,-data.width/2),gp_Dir(0,0,1)),(s.radius+1)*s.module,data.width,2*pi/data.teeth).Shape();gp_Trsf center;center.SetRotation(gp_Ax1(gp_Pnt(0,0,0),gp_Dir(0,0,1)),-pi/data.teeth);const auto blank=BRepBuilderAPI_Transform(wedge,center,true).Shape();
  Faces blank_sources;for(TopExp_Explorer f(blank,TopAbs_FACE);f.More();f.Next()){const auto face=TopoDS::Face(f.Current());BRepAdaptor_Surface geometry(face);if(geometry.GetType()==GeomAbs_Cylinder)blank_sources.push_back({face,7,6});else if(geometry.GetType()==GeomAbs_Plane){const auto plane=geometry.Plane();if(std::abs(plane.Axis().Direction().Z())<1e-10)blank_sources.push_back({face,0,250});else blank_sources.push_back({face,0,std::uint8_t(plane.Location().Z()>0?1:0)});}else throw std::runtime_error("ZA sector has an unexpected primitive boundary");}
  TopTools_ListOfShape args,tools;args.Append(blank);tools.Append(tool.shape);BRepAlgoAPI_Cut cut;cut.SetNonDestructive(true);cut.SetArguments(args);cut.SetTools(tools);cut.Build();if(!cut.IsDone())throw std::runtime_error("ZA generating-space sector subtraction failed");Faces all_sources=tool.sources;all_sources.insert(all_sources.end(),blank_sources.begin(),blank_sources.end());auto sector=finish(cut.Shape(),carry(cut,all_sources));const auto sector_sources=seed_faces(*sector);
  BRepBuilderAPI_Sewing sewing(data.tolerance,true,true,false,false);sewing.SetMaxTolerance(data.tolerance);Faces repeated;
  for(std::uint32_t tooth=0;tooth<data.teeth;++tooth){gp_Trsf turn;turn.SetRotation(gp_Ax1(gp_Pnt(0,0,0),gp_Dir(0,0,1)),2*pi*tooth/data.teeth);BRepBuilderAPI_Transform transformed(sector->value,turn,true);if(!transformed.IsDone())throw std::runtime_error("ZA periodic sector placement failed");for(const auto&source:sector_sources){if(source.role==250)continue;const auto actual=transformed.ModifiedShape(source.face);if(actual.ShapeType()!=TopAbs_FACE)throw std::runtime_error("ZA transformed face history is absent");sewing.Add(actual);repeated.push_back({TopoDS::Face(actual),source.role==6?tooth*8+source.source:tooth,source.role});}}
  sewing.Perform();const auto sewn=sewing.SewedShape();if(sewn.ShapeType()!=TopAbs_SHELL||sewing.NbFreeEdges()||sewing.NbMultipleEdges()||sewing.NbDeletedFaces())throw std::runtime_error("ZA periodic wheel shell is not watertight");BRepBuilderAPI_MakeSolid made(TopoDS::Shell(sewn));auto closed=made.Solid();if(!BRepLib::OrientClosedSolid(closed))throw std::runtime_error("ZA wheel shell orientation failed");auto result=finish(closed,after_sewing(sewing,repeated));result=bore(std::move(result),data.bore,-data.width/2,data.width,2);
  BRepClass3d_SolidClassifier classifier(result->value);for(double fraction:{-.4,0.,.4}){const double v=fraction*data.width/s.module;std::array<double,2> angles{};for(int side=0;side<2;++side){const int sign=side?1:-1;double lo=3.75,hi=6.25;for(int j=0;j<64;++j){const double r=(lo+hi)/2;const auto p=flank(s,r,v,sign);if(std::hypot(p[0],p[1])>s.radius)lo=r;else hi=r;}const auto p=flank(s,(lo+hi)/2,v,sign);angles[side]=std::atan2(p[1],p[0]);}const double middle=(angles[0]+angles[1])/2;for(std::uint32_t tooth=0;tooth<data.teeth;++tooth)for(int gap=0;gap<2;++gap){const double phase=middle+2*pi*tooth/data.teeth+(gap?0:pi/data.teeth);classifier.Perform(gp_Pnt(s.radius*s.module*std::cos(phase),s.radius*s.module*std::sin(phase),v*s.module),data.tolerance*4);if(classifier.State()!=(gap?TopAbs_OUT:TopAbs_IN))throw std::runtime_error("Generated ZA wheel lost reference material or tooth space");}}
  const auto geometry_done=std::chrono::steady_clock::now();prime_point_volume(*result);
  if(profiling())std::fprintf(stderr,"multicad.native.worm wheel_teeth=%u geometry_ms=%.6f volume_ms=%.6f\n",data.teeth,std::chrono::duration<double,std::milli>(geometry_done-started).count(),std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-geometry_done).count());
  return result;
}
}
std::unique_ptr<Shape> za_worm(const ThreadData&data,double radius){
  try{
    const double module=data.pitch/pi;
    auto near=[](double a,double b){return std::isfinite(a)&&std::abs(a-b)<=std::max(1.,std::abs(b))*1e-12;};
    if(module<.5||module>2||data.starts!=1||data.left||data.axial_flat||data.taper!=0||data.rounding!=0||data.root_rounding!=0||data.knuckle_root_radius!=0||!near(data.radius,6*module)||!near(data.depth,2.25*module)||!near(data.lower_slope,slope)||!near(data.upper_slope,slope)||data.height<16*module||data.height>32*module||!std::isfinite(radius)||radius<0||radius>=3.75*module)
      throw std::runtime_error("ZA worm needs the declared q10 right-hand single-start axial-straight form");
    const double thinning=data.pitch/2-2*module*slope-data.crest_width;if(thinning< -1e-12||thinning>.1*module+1e-12)throw std::runtime_error("ZA worm reference thinning is outside its declared interval");
    auto result=helical_thread_fresh(data);result=bore(std::move(result),radius,0,data.height,5);prime_point_volume(*result);return result;
  }catch(const Standard_Failure&e){throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native ZA worm construction failed");}
}
std::unique_ptr<Shape> za_worm_wheel(const WormWheelData&data){try{return wheel(data);}catch(const Standard_Failure&e){throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native generated ZA wheel construction failed");}}
}
