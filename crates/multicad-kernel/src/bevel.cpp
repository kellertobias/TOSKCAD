#include "bevel.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepFill.hxx>
#include <BRepLib.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <BRep_Builder.hxx>
#include <Geom2d_BSplineCurve.hxx>
#include <Geom2d_Line.hxx>
#include <Geom2d_TrimmedCurve.hxx>
#include <Geom_ConicalSurface.hxx>
#include <Geom_Curve.hxx>
#include <Standard_Failure.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <TColStd_Array1OfReal.hxx>
#include <TColgp_Array1OfPnt2d.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Pln.hxx>
#include <algorithm>
#include <array>
#include <cmath>
#include <functional>
#include <limits>
#include <stdexcept>
#include <vector>

namespace multicad::kernel {
namespace {
constexpr double pi=3.14159265358979323846;
constexpr int order=4, max_spans=127;
struct V {
  double x,y,z;
  V operator+(V b)const{return{x+b.x,y+b.y,z+b.z};}
  V operator-(V b)const{return{x-b.x,y-b.y,z-b.z};}
  V operator*(double a)const{return{x*a,y*a,z*a};}
};
double dot(V a,V b){return a.x*b.x+a.y*b.y+a.z*b.z;}
V cross(V a,V b){return{a.y*b.z-a.z*b.y,a.z*b.x-a.x*b.z,a.x*b.y-a.y*b.x};}
V unit(V a){const double n=std::sqrt(dot(a,a));if(!std::isfinite(n)||n<1e-14)throw std::runtime_error("Degenerate spherical bevel direction");return a*(1/n);}
V polar(double gamma,double phi){return{std::sin(gamma)*std::cos(phi),std::sin(gamma)*std::sin(phi),std::cos(gamma)};}
gp_Pnt point(V a){return{a.x,a.y,a.z};}
V spherical(const BevelData&s,double t){
  const double b=std::sin(s.base),c=std::cos(s.base),v=b*t;
  const double x=b*std::cos(t)*std::cos(v)+std::sin(t)*std::sin(v);
  const double y=b*std::sin(t)*std::cos(v)-std::cos(t)*std::sin(v);
  return{x*std::cos(s.half_base)+y*std::sin(s.half_base),x*std::sin(s.half_base)-y*std::cos(s.half_base),c*std::cos(v)};
}
V derivative(const BevelData&s,double t){
  const double b=std::sin(s.base),c=std::cos(s.base),v=b*t;
  const double x=c*c*std::cos(t)*std::sin(v),y=c*c*std::sin(t)*std::sin(v);
  return{x*std::cos(s.half_base)+y*std::sin(s.half_base),x*std::sin(s.half_base)-y*std::cos(s.half_base),-b*c*std::sin(v)};
}
struct Root {double rounding,center_gamma,touch_gamma,touch_t,phi,arc;bool meridian;V center,a,b,touch,root;};
Root root(const BevelData&s){
  Root r{};r.rounding=std::asin(s.fillet/s.distance);r.center_gamma=s.root+r.rounding;
  r.meridian=r.center_gamma<=std::acos(std::cos(s.base)*std::cos(r.rounding));
  if(r.meridian){
    r.touch_gamma=std::acos(std::cos(r.center_gamma)/std::cos(r.rounding));r.touch_t=0;
    r.phi=s.half_base+std::asin(std::sin(r.rounding)/std::sin(r.center_gamma));
    r.center=polar(r.center_gamma,r.phi);r.touch=polar(r.touch_gamma,s.half_base);
    if(r.touch_gamma>s.base+1e-12)throw std::runtime_error("Bevel meridian blend crosses its base cone");
  }else{
    const double sigma=std::acos(std::cos(r.center_gamma)/std::cos(s.base))-r.rounding;
    r.touch_t=sigma/std::sin(s.base);r.touch=spherical(s,r.touch_t);
    const V normal=unit(cross(r.touch,unit(derivative(s,r.touch_t))));
    r.center=r.touch*std::cos(r.rounding)+normal*std::sin(r.rounding);
    r.phi=std::atan2(r.center.y,r.center.x);r.touch_gamma=std::acos(r.touch.z);
  }
  r.root=polar(s.root,r.phi);
  r.a=unit(r.root-r.center*std::cos(r.rounding));r.b=unit(cross(r.center,r.a));
  r.arc=std::atan2(dot(r.touch,r.b),dot(r.touch,r.a));
  const double tip_t=std::acos(std::cos(s.tip)/std::cos(s.base))/std::sin(s.base);
  const auto tip=spherical(s,tip_t);const double tip_half=std::atan2(tip.y,tip.x);
  const double toe=s.distance-s.width,scale=toe/std::cos(s.root-s.pitch);
  if(!std::isfinite(r.arc)||std::abs(r.arc)<1e-8||std::abs(r.arc)>=pi||
      r.phi<=0||r.phi>=pi/s.teeth-s.minimum/scale||r.touch_gamma>=s.pitch||
      r.touch_gamma>=s.tip||tip_half<=s.minimum/(s.distance*std::sin(s.tip))||
      tip_half>=r.phi||s.root+r.rounding>=s.tip)
    throw std::runtime_error("Spherical bevel root, tip or straight flank is consumed");
  return r;
}
// Outward-rounded interval Taylor coefficients enclose all fourth derivatives
// over a span. This is independent of the subsequent native spline samples.
struct I {double lo,hi;I(double v=0):lo(v),hi(v){}I(double a,double b):lo(a),hi(b){}};
double down(double v){return std::nextafter(v,-std::numeric_limits<double>::infinity());}
double up(double v){return std::nextafter(v,std::numeric_limits<double>::infinity());}
I operator+(I a,I b){return{down(a.lo+b.lo),up(a.hi+b.hi)};}
I operator-(I a){return{-a.hi,-a.lo};}
I operator-(I a,I b){return a+(-b);}
I operator*(I a,I b){const std::array<double,4> p={a.lo*b.lo,a.lo*b.hi,a.hi*b.lo,a.hi*b.hi};return{down(*std::min_element(p.begin(),p.end())),up(*std::max_element(p.begin(),p.end()))};}
I inverse(I a){if(a.lo<=0&&a.hi>=0)throw std::runtime_error("Interval crossed a singular bevel parameter");return{down(1/a.hi),up(1/a.lo)};}
I operator/(I a,I b){return a*inverse(b);}
I square_root(I a){if(a.lo<=0)throw std::runtime_error("Interval crossed a bevel pole");return{down(std::sqrt(a.lo)),up(std::sqrt(a.hi))};}
double magnitude(I a){return std::max(std::abs(a.lo),std::abs(a.hi));}
double middle(I a){return a.lo+(a.hi-a.lo)/2;}
// Bounded Taylor evaluation avoids relying on libm point accuracy or rounded
// phase+n*pi/2 shifts when enclosing the Hermite remainder. The native chart's
// constants are stored doubles; root-direction solving and OCCT arithmetic are
// separately checked by residual/material tests, not certified by this bound.
I trigonometric(I x,bool cosine){
  if(magnitude(x)>8)throw std::runtime_error("Bevel trigonometric enclosure exceeds bounded argument range");
  I term=cosine?I(1):x,sum=term,xx=x*x;
  for(int k=1;k<25;++k){const int n=cosine?2*k:2*k+1;term=(-term)*xx/(I(n)*I(n-1));sum=sum+term;}
  const int degree=cosine?50:51;I remainder(1);for(int n=1;n<=degree;++n)remainder=remainder*I(magnitude(x))/I(n);
  return sum+I(-remainder.hi,remainder.hi);
}
I arctangent(I x){
  const I z=x/(I(1)+square_root(I(1)+x*x));
  if(magnitude(z)>=0.8)throw std::runtime_error("Bevel azimuth enclosure exceeds bounded quadrant");
  I power=z,sum=z,zz=z*z;
  for(int k=1;k<25;++k){power=(-power)*zz;sum=sum+power/I(2*k+1);}
  I remainder(1);for(int n=0;n<51;++n)remainder=remainder*I(magnitude(z));remainder=remainder/I(51);
  return I(2)*(sum+I(-remainder.hi,remainder.hi));
}

struct J {std::array<I,order+1> c;J(double v=0){c.fill(I(0));c[0]=I(v);}};
J operator+(J a,J b){J r;for(int i=0;i<=order;++i)r.c[i]=a.c[i]+b.c[i];return r;}
J operator-(J a,J b){J r;for(int i=0;i<=order;++i)r.c[i]=a.c[i]-b.c[i];return r;}
J operator*(J a,J b){J r;for(int n=0;n<=order;++n)for(int i=0;i<=n;++i)r.c[n]=r.c[n]+a.c[i]*b.c[n-i];return r;}
J reciprocal(J a){J b;b.c[0]=inverse(a.c[0]);for(int n=1;n<=order;++n){I sum;for(int i=1;i<=n;++i)sum=sum+a.c[i]*b.c[n-i];b.c[n]=(-sum)/a.c[0];}return b;}
J operator/(J a,J b){return a*reciprocal(b);}
J sqrt_jet(J a){J b;b.c[0]=square_root(a.c[0]);for(int n=1;n<=order;++n){I sum;for(int i=1;i<n;++i)sum=sum+b.c[i]*b.c[n-i];b.c[n]=(a.c[n]-sum)/(I(2)*b.c[0]);}return b;}
J constant(I value){J r;r.c[0]=value;return r;}
J trig(bool cosine,I rate,double lo,double hi){
  J r;I factor(1);const I argument=rate*I(lo,hi),s=trigonometric(argument,false),c=trigonometric(argument,true);
  for(int n=0;n<=order;++n){if(n)factor=factor*rate/I(n);const int cycle=(n+(cosine?1:0))%4;const I value=cycle==0?s:cycle==1?c:cycle==2?-s:-c;r.c[n]=value*factor;}
  return r;
}
J differentiate(J a){J r;for(int i=0;i<order;++i)r.c[i]=a.c[i+1]*I(i+1);return r;}
using Q=std::array<J,3>;
Q sphere_jet(const BevelData&s,double lo,double hi){
  const I b=trigonometric(I(s.base),false),c=trigonometric(I(s.base),true);
  const J ct=trig(true,I(1),lo,hi),st=trig(false,I(1),lo,hi),cv=trig(true,b,lo,hi),sv=trig(false,b,lo,hi);
  const J x=constant(b)*ct*cv+st*sv,y=constant(b)*st*cv-ct*sv;
  const J ch=constant(trigonometric(I(s.half_base),true)),sh=constant(trigonometric(I(s.half_base),false));
  return{x*ch+y*sh,x*sh-y*ch,constant(c)*cv};
}
Q circle_jet(const Root&r,double lo,double hi){
  const J c=trig(true,I(1),lo,hi),s=trig(false,I(1),lo,hi);Q result;
  const std::array<double,3> u={r.center.x,r.center.y,r.center.z},a={r.a.x,r.a.y,r.a.z},b={r.b.x,r.b.y,r.b.z};
  for(int i=0;i<3;++i)result[i]=J(std::cos(r.rounding)*u[i])+J(std::sin(r.rounding))*(J(a[i])*c+J(b[i])*s);
  return result;
}
Q meridian_jet(double phi,double lo,double hi){const J s=trig(false,I(1),lo,hi);return{s*constant(trigonometric(I(phi),true)),s*constant(trigonometric(I(phi),false)),trig(true,I(1),lo,hi)};}
using UV=std::array<J,2>;
UV project(const BevelData&s,const Q&q){
  const J radius2=q[0]*q[0]+q[1]*q[1];
  const J radius=sqrt_jet(radius2);
  if(q[0].c[0].lo<=0)throw std::runtime_error("Bevel curve crosses its bounded azimuth chart");
  J azimuth;azimuth.c[0]=arctangent(q[1].c[0]/q[0].c[0]);
  const J angular=(q[0]*differentiate(q[1])-q[1]*differentiate(q[0]))/radius2;
  for(int i=1;i<=order;++i)azimuth.c[i]=angular.c[i-1]/I(i);
  const J cp=constant(trigonometric(I(s.pitch),true)),sp=constant(trigonometric(I(s.pitch),false));
  const J v=J(-s.distance)*radius/(cp*(radius*sp+q[2]*cp));
  return{azimuth,v};
}
struct Piece {uint32_t source;uint8_t type;double a,b,gamma,u0,u1;std::function<Q(double,double)> curve;};
Handle(Geom2d_Curve) approximation(const BevelData&s,const Piece&p,double phase,bool mirror){
  struct Span{double a,b;};std::vector<Span> spans;
  const double radius=s.distance*std::sin(s.tip)/std::cos(s.tip-s.pitch),numeric=256*std::numeric_limits<double>::epsilon()*std::max(1.,s.distance)/std::pow(std::sin(s.root),4);
  if(!std::isfinite(numeric)||numeric>s.tolerance/64)throw std::runtime_error("Bevel numeric conditioning exceeds its profile budget");
  const double budget=s.tolerance/32;
  std::function<void(double,double,int)> split=[&](double a,double b,int depth){
    bool accepted=false;
    try{const auto q=project(s,p.curve(std::min(a,b),std::max(a,b))),qa=project(s,p.curve(a,a)),qb=project(s,p.curve(b,b));const double h=std::abs(b-a);std::array<double,2> uncertainty{};
      for(int i=0;i<2;++i)uncertainty[i]=(qa[i].c[0].hi-qa[i].c[0].lo+qb[i].c[0].hi-qb[i].c[0].lo)/2+h*(qa[i].c[1].hi-qa[i].c[1].lo+qb[i].c[1].hi-qb[i].c[1].lo)/6;
      const double u=magnitude(q[0].c[4])*24*std::pow(h,4)/384+uncertainty[0],v=magnitude(q[1].c[4])*24*std::pow(h,4)/384+uncertainty[1];accepted=std::isfinite(u)&&std::isfinite(v)&&(radius+v)*u+v+numeric<=budget;}catch(const std::runtime_error&){}
    if(accepted){spans.push_back({a,b});if(spans.size()>max_spans)throw std::runtime_error("Spherical bevel curve exceeds the 256-pole budget");return;}
    if(depth>=16)throw std::runtime_error("Spherical bevel derivative enclosure did not converge");
    const double mid=a+(b-a)/2;split(a,mid,depth+1);split(mid,b,depth+1);
  };
  split(p.a,p.b,0);
  // Knots are increasing even when a circle is traversed backwards.
  const double sign=p.b>=p.a?1:-1;
  TColgp_Array1OfPnt2d poles(1,2*spans.size()+2);TColStd_Array1OfReal knots(1,spans.size()+1);TColStd_Array1OfInteger mult(1,spans.size()+1);
  auto uv=[&](double t){auto v=project(s,p.curve(t,t));return gp_Pnt2d(phase+(mirror?-1:1)*middle(v[0].c[0]),middle(v[1].c[0]));};
  auto slope=[&](double t){auto v=project(s,p.curve(t,t));return gp_Vec2d((mirror?-1:1)*middle(v[0].c[1])*sign,middle(v[1].c[1])*sign);};
  poles.SetValue(1,uv(p.a));
  for(size_t i=0;i<spans.size();++i){const auto span=spans[i];const double h=std::abs(span.b-span.a)/3;const auto a=uv(span.a),b=uv(span.b);poles.SetValue(2*i+2,a.Translated(slope(span.a)*h));poles.SetValue(2*i+3,b.Translated(slope(span.b)*(-h)));}
  poles.SetValue(2*spans.size()+2,uv(p.b));
  for(size_t i=0;i<=spans.size();++i){knots.SetValue(i+1,sign*(i==spans.size()?p.b:spans[i].a));mult.SetValue(i+1,i==0||i==spans.size()?4:2);}
  return new Geom2d_BSplineCurve(poles,knots,mult,3);
}
Handle(Geom_ConicalSurface) backcone(const BevelData&s,double distance){return new Geom_ConicalSurface(gp_Ax3(gp_Pnt(0,0,distance/std::cos(s.pitch)),gp_Dir(0,0,1)),s.pitch-pi/2,0);}
Handle(Geom2d_Curve) line(double u0,double v0,double u1,double v1){const gp_Pnt2d a(u0,v0),b(u1,v1);return new Geom2d_TrimmedCurve(new Geom2d_Line(a,gp_Dir2d(gp_Vec2d(a,b))),0,a.Distance(b));}
TopoDS_Edge edge(const Handle(Geom2d_Curve)&curve,const Handle(Geom_Surface)&surface,double tolerance){BRepBuilderAPI_MakeEdge made(curve,surface,curve->FirstParameter(),curve->LastParameter());if(!made.IsDone())throw std::runtime_error("Bevel curve-on-cone edge failed");auto e=made.Edge();if(!BRepLib::BuildCurve3d(e,tolerance,GeomAbs_C1,14,128))throw std::runtime_error("Bevel 3D boundary reconstruction failed");return e;}
struct NamedFace{TopoDS_Face face;uint32_t source;uint8_t role;};
void append(std::vector<NamedFace>&faces,const TopoDS_Face&face,uint32_t source,uint8_t role){faces.push_back({face,source,role});}
struct Edges{TopoDS_Edge heel,toe;uint32_t source;};
std::unique_ptr<Shape> construct(const BevelData&s){
  for(double v:{s.pitch,s.base,s.tip,s.root,s.distance,s.width,s.bore,s.half_base,s.fillet,s.tolerance,s.minimum})if(!std::isfinite(v))throw std::runtime_error("Nonfinite spherical bevel input");
  if(s.teeth<4||s.teeth>64||s.root<=0||s.base<=0||s.base>=s.pitch||s.root>=s.pitch||s.tip<=s.pitch||s.tip>=pi/2||s.distance<=0||s.width<=0||s.width>=s.distance||s.bore<0||s.fillet<=0||s.fillet>=s.distance||s.tolerance<=0||s.minimum<=0||s.half_base<=0||s.half_base>=pi/2)throw std::runtime_error("Spherical bevel exceeds its validated geometric domain");
  const Root blend=root(s);const double ratio=(s.distance-s.width)/s.distance;
  const double inner_gamma=s.root/2,inner_v=-s.distance*std::tan(inner_gamma)/(std::cos(s.pitch)*(std::tan(inner_gamma)*std::sin(s.pitch)+std::cos(s.pitch)));
  const double root_radius=(s.distance-s.width)*std::sin(s.root)/std::cos(s.root-s.pitch);
  if(root_radius-s.bore<s.minimum)throw std::runtime_error("Bevel bore consumes its toe root wall");
  const auto heel_surface=backcone(s,s.distance),toe_surface=backcone(s,s.distance-s.width);
  const double tip_t=std::acos(std::cos(s.tip)/std::cos(s.base))/std::sin(s.base);
  const auto tip=spherical(s,tip_t);const double half_tip=std::atan2(tip.y,tip.x);
  // One positive-azimuth flank, root-to-tip; mirror/reverse below builds the
  // counterclockwise outer boundary without a spur or flat-plane substitute.
  std::vector<Piece> flank;
  flank.push_back({0,1,0,blend.arc,0,0,0,[&](double a,double b){return circle_jet(blend,a,b);}});
  if(blend.meridian&&s.base-blend.touch_gamma>s.minimum/s.distance)flank.push_back({1,1,blend.touch_gamma,s.base,0,0,0,[&](double a,double b){return meridian_jet(s.half_base,a,b);}});
  flank.push_back({2,1,blend.touch_t,tip_t,0,0,0,[&](double a,double b){return sphere_jet(s,a,b);}});
  std::vector<NamedFace> source_faces;
  BRepBuilderAPI_Sewing sewing(s.tolerance,true,true,false,false);sewing.SetMaxTolerance(s.tolerance);
  auto add_face=[&](TopoDS_Face face,uint32_t source,uint8_t role){sewing.Add(face);append(source_faces,face,source,role);};
  for(uint32_t tooth=0;tooth<s.teeth;++tooth){
    const double phase=2*pi*tooth/s.teeth,first=phase-pi/s.teeth,last=phase+pi/s.teeth;
    std::vector<std::pair<Handle(Geom2d_Curve),uint32_t>> curves;
    auto iso=[&](double gamma,double a,double b,uint32_t source){const double v=-s.distance*std::sin(gamma)/(std::cos(s.pitch)*std::cos(gamma-s.pitch));curves.emplace_back(line(a,v,b,v),source);};
    auto boundary=[&](const Piece&p,bool mirror)->Handle(Geom2d_Curve){
      if(p.source==1){const double u=phase+(mirror?-s.half_base:s.half_base);auto v=[&](double gamma){return -s.distance*std::sin(gamma)/(std::cos(s.pitch)*std::cos(gamma-s.pitch));};return line(u,v(p.a),u,v(p.b));}
      return approximation(s,p,phase,mirror);
    };
    iso(s.root,first,phase-blend.phi,tooth*8+7);
    for(const auto&p:flank)curves.emplace_back(boundary(p,true),tooth*8+p.source);
    iso(s.tip,phase-half_tip,phase+half_tip,tooth*8+3);
    for(auto it=flank.rbegin();it!=flank.rend();++it){Piece reversed=*it;std::swap(reversed.a,reversed.b);curves.emplace_back(boundary(reversed,false),tooth*8+(6-it->source));}
    iso(s.root,phase+blend.phi,last,tooth*8+7);
    std::vector<Edges> outer;BRepBuilderAPI_MakeWire heel_wire,toe_wire;
    for(const auto&c:curves){
      auto h=edge(c.first,heel_surface,s.tolerance/32);
      // Geom2d curves do not support arbitrary affine scaling; the 2D curve is
      // reconstructed from identical native BSpline poles/knots below.
      Handle(Geom2d_Curve) uv_toe;
      if(auto spline=Handle(Geom2d_BSplineCurve)::DownCast(c.first);!spline.IsNull()){
        auto copy=Handle(Geom2d_BSplineCurve)::DownCast(spline->Copy());for(int i=1;i<=copy->NbPoles();++i){auto p=copy->Pole(i);p.SetY(p.Y()*ratio);copy->SetPole(i,p);}uv_toe=copy;
      }else{double a=c.first->FirstParameter(),b=c.first->LastParameter();const auto p=c.first->Value(a),q=c.first->Value(b);uv_toe=line(p.X(),p.Y()*ratio,q.X(),q.Y()*ratio);}
      BRepBuilderAPI_MakeEdge toe_made(uv_toe,toe_surface,uv_toe->FirstParameter(),uv_toe->LastParameter());if(!toe_made.IsDone())throw std::runtime_error("Bevel toe boundary failed");auto t=toe_made.Edge();
      double a,b;auto original=BRep_Tool::Curve(h,a,b);gp_Trsf shrink;shrink.SetScale(gp_Pnt(0,0,0),ratio);auto scaled=Handle(Geom_Curve)::DownCast(original->Transformed(shrink));
      // Lines have arc-length parameters. Use the native surface's own exact
      // 3D line/circle reconstruction; spline flanks retain exact homothety.
      if(!Handle(Geom2d_BSplineCurve)::DownCast(c.first).IsNull()){BRep_Builder builder;builder.UpdateEdge(t,scaled,s.tolerance/16);builder.Range(t,a,b);}else if(!BRepLib::BuildCurve3d(t,s.tolerance/32,GeomAbs_C1,14,128))throw std::runtime_error("Bevel toe ISO curve failed");
      heel_wire.Add(h);toe_wire.Add(t);if(!heel_wire.IsDone()||!toe_wire.IsDone())throw std::runtime_error("Bevel cap wire disconnected");outer.push_back({heel_wire.Edge(),toe_wire.Edge(),c.second});
    }
    auto close_cap=[&](BRepBuilderAPI_MakeWire&wire,const Handle(Geom_ConicalSurface)&surface,double k){
      const auto start=curves.front().first->Value(curves.front().first->FirstParameter()),end=curves.back().first->Value(curves.back().first->LastParameter());
      for(const auto&c:std::array<Handle(Geom2d_Curve),3>{line(last,end.Y()*k,last,inner_v*k),line(last,inner_v*k,first,inner_v*k),line(first,inner_v*k,first,start.Y()*k)})wire.Add(edge(c,surface,s.tolerance/32));
      if(!wire.IsDone())throw std::runtime_error("Bevel cone cap did not close");BRepBuilderAPI_MakeFace cap(surface,wire.Wire(),true);if(!cap.IsDone())throw std::runtime_error("Bevel conical cap construction failed");return cap.Face();
    };
    add_face(close_cap(heel_wire,heel_surface,1),tooth*2,0);add_face(close_cap(toe_wire,toe_surface,ratio),tooth*2,1);
    for(const auto&part:outer){
      const auto kind=part.source%8;
      if(kind==7||kind==3){
        const double gamma=kind==7?s.root:s.tip;const auto cone=Handle(Geom_ConicalSurface)(new Geom_ConicalSurface(gp_Ax3(gp_Pnt(0,0,0),gp_Dir(0,0,1)),gamma,0));
        double a,b;auto c=BRep_Tool::CurveOnSurface(part.heel,heel_surface,TopLoc_Location(),a,b);const double u0=c->Value(a).X(),u1=c->Value(b).X();
        const double distance=s.distance/std::cos(gamma-s.pitch);
        BRepBuilderAPI_MakeFace side(cone,std::min(u0,u1),std::max(u0,u1),distance*ratio,distance,s.tolerance);if(!side.IsDone())throw std::runtime_error("Bevel analytic root/tip cone failed");add_face(side.Face(),part.source,6);
      }else if(kind==1||kind==5){
        // Below the base cone the declared meridian is an actual plane through
        // the pitch apex. Preserve that exact elementary surface instead of a
        // high-degree ruled spline with a numerically near-zero volume term.
        TopoDS_Vertex ha,hb,ta,tb;TopExp::Vertices(part.heel,ha,hb,true);TopExp::Vertices(part.toe,ta,tb,true);
        const auto a=BRep_Tool::Pnt(ha),b=BRep_Tool::Pnt(hb),c=BRep_Tool::Pnt(ta),d=BRep_Tool::Pnt(tb);
        BRepBuilderAPI_MakeWire wire;wire.Add(part.heel);wire.Add(BRepBuilderAPI_MakeEdge(b,d).Edge());wire.Add(TopoDS::Edge(part.toe.Reversed()));wire.Add(BRepBuilderAPI_MakeEdge(c,a).Edge());
        if(!wire.IsDone())throw std::runtime_error("Bevel meridian plane wire failed");
        const double phi=std::atan2(a.Y(),a.X());BRepBuilderAPI_MakeFace face(gp_Pln(gp_Pnt(0,0,0),gp_Dir(-std::sin(phi),std::cos(phi),0)),wire.Wire(),true);
        if(!face.IsDone())throw std::runtime_error("Bevel exact meridian face failed");add_face(face.Face(),part.source,6);
      }else add_face(BRepFill::Face(part.heel,part.toe),part.source,6);
    }
    for(int end=0;end<2;++end){const auto surface=end?toe_surface:heel_surface;const double k=end?ratio:1;BRepBuilderAPI_MakeFace core(surface,first,last,inner_v*k,0,s.tolerance);if(!core.IsDone())throw std::runtime_error("Bevel cone apex sector failed");add_face(core.Face(),tooth*2+1,end);}
  }
  sewing.Perform();const auto sewn=sewing.SewedShape();
  if(sewn.IsNull()||sewn.ShapeType()!=TopAbs_SHELL||sewing.NbFreeEdges()!=0||sewing.NbMultipleEdges()!=0||sewing.NbDeletedFaces()!=0)throw std::runtime_error("Spherical bevel needs one watertight shell (free="+std::to_string(sewing.NbFreeEdges())+", multiple="+std::to_string(sewing.NbMultipleEdges())+")");
  BRepBuilderAPI_MakeSolid made(TopoDS::Shell(sewn));if(!made.IsDone())throw std::runtime_error("Spherical bevel solid construction failed");auto shape=made.Solid();
  if(!BRepLib::OrientClosedSolid(shape)||!BRepCheck_Analyzer(shape).IsValid())throw std::runtime_error("Spherical bevel sewn solid is invalid");
  std::vector<NamedFace> mapped;
  for(const auto&source:source_faces){const auto actual=sewing.IsModified(source.face)?sewing.Modified(source.face):source.face;for(TopExp_Explorer faces(actual,TopAbs_FACE);faces.More();faces.Next())mapped.push_back({TopoDS::Face(faces.Current()),source.source,source.role});if(actual.ShapeType()==TopAbs_FACE)mapped.push_back({TopoDS::Face(actual),source.source,source.role});}
  TopoDS_Shape final_shape=shape;
  if(s.bore>0){
    const double maximum=s.distance/std::cos(s.pitch);const auto tool=BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0,0,-1),gp_Dir(0,0,1)),s.bore,maximum+2).Shape();
    BRepAlgoAPI_Cut cut;cut.SetNonDestructive(true);cut.SetArguments(TopTools_ListOfShape());
    TopTools_ListOfShape args,tools;args.Append(shape);tools.Append(tool);cut.SetArguments(args);cut.SetTools(tools);cut.Build();if(!cut.IsDone())throw std::runtime_error("Bevel bore subtraction failed");final_shape=cut.Shape();
    std::vector<NamedFace> next;for(const auto&source:mapped){if(!cut.IsDeleted(source.face))next.push_back(source);for(const auto&f:cut.Modified(source.face))if(f.ShapeType()==TopAbs_FACE)next.push_back({TopoDS::Face(f),source.source,source.role});}mapped=std::move(next);
    for(TopExp_Explorer f(tool,TopAbs_FACE);f.More();f.Next())for(const auto&actual:cut.Modified(f.Current()))if(actual.ShapeType()==TopAbs_FACE&&BRepAdaptor_Surface(TopoDS::Face(actual)).GetType()==GeomAbs_Cylinder)mapped.push_back({TopoDS::Face(actual),0,2});
  }
  if(!BRepCheck_Analyzer(final_shape).IsValid())throw std::runtime_error("Finished spherical bevel B-rep is invalid");int count=0;for(TopExp_Explorer solids(final_shape,TopAbs_SOLID);solids.More();solids.Next())++count;if(count!=1)throw std::runtime_error("Spherical bevel must retain one connected solid");
  BRepClass3d_SolidClassifier classifier(final_shape);
  for(double fraction:{.2,.5,.8})for(uint32_t tooth=0;tooth<s.teeth;++tooth){const double R=s.distance-s.width*fraction;for(int gap=0;gap<2;++gap){const V q=polar(s.pitch,2*pi*tooth/s.teeth+(gap?pi/s.teeth:0));classifier.Perform(point(q*R),s.tolerance*4);if((classifier.State()==TopAbs_IN)!=(!gap))throw std::runtime_error("Spherical bevel lost pitch-cone tooth material or space");}}
  auto result=std::make_unique<Shape>(Shape{final_shape,{},{}});TopTools_IndexedMapOfShape faces;TopExp::MapShapes(final_shape,TopAbs_FACE,faces);
  for(int i=1;i<=faces.Extent();++i){bool found=false;for(const auto&source:mapped)if(source.face.IsSame(faces(i))){NativeSeed seed{static_cast<uint32_t>(i-1),source.source,source.role};bool duplicate=false;for(const auto&known:result->seeds)if(known.face==seed.face&&known.source_index==seed.source_index&&known.role==seed.role)duplicate=true;if(!duplicate)result->seeds.push_back(seed);found=true;}if(!found)throw std::runtime_error("Spherical bevel face has no exact native ancestry");}
  return result;
}
}
std::unique_ptr<Shape> spherical_bevel_gear(const BevelData&s){try{return construct(s);}catch(const Standard_Failure&e){throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native spherical bevel operation failed");}}
}
