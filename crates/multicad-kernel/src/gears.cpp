#include "gears.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepLib.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <Standard_Failure.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <TColStd_Array1OfReal.hxx>
#include <TColgp_Array1OfPnt.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <algorithm>
#include <cmath>
#include <stdexcept>
#include <vector>

namespace multicad::kernel {
namespace {
constexpr double pi=3.14159265358979323846;
struct V {
  double x,y;
  V operator+(V b) const {return {x+b.x,y+b.y};}
  V operator-(V b) const {return {x-b.x,y-b.y};}
  V operator*(double s) const {return {x*s,y*s};}
};
V polar(double r,double a) {return {r*std::cos(a),r*std::sin(a)};}
V rotate(V p,double a) {return {p.x*std::cos(a)-p.y*std::sin(a),p.x*std::sin(a)+p.y*std::cos(a)};}
V mirror(V p) {return {p.x,-p.y};}
double radius(V p) {return std::hypot(p.x,p.y);}
gp_Pnt point(V p,double z) {return {p.x,p.y,z};}
struct FaceSource {TopoDS_Shape face;uint32_t source;uint8_t role;};
struct Built {TopoDS_Shape shape;std::vector<FaceSource> faces;};
struct Profile {TopoDS_Wire wire;std::vector<std::pair<TopoDS_Edge,uint32_t>> edges;};
struct Dims {double t0,ta;V root,foot,center;bool radial;};
V involute(const GearData&s,double t) {
  return rotate({s.base_radius*(std::cos(t)+t*std::sin(t)),
    s.base_radius*(std::sin(t)-t*std::cos(t))},-s.half_base);
}
V derivative(const GearData&s,double t) {
  return rotate({s.base_radius*t*std::cos(t),s.base_radius*t*std::sin(t)},-s.half_base);
}
Dims dimensions(const GearData&s) {
  Dims d;d.ta=std::sqrt(std::pow(s.tip_radius/s.base_radius,2)-1);
  const double foot=std::sqrt(s.root_radius*s.root_radius+2*s.root_radius*s.fillet_radius);
  d.radial=foot<s.base_radius;
  if(d.radial) {
    const double a=-s.half_base-std::asin(s.fillet_radius/(s.root_radius+s.fillet_radius));
    d.center=polar(s.root_radius+s.fillet_radius,a);d.root=polar(s.root_radius,a);
    d.foot=polar(foot,-s.half_base);d.t0=0;
  } else {
    d.t0=(std::sqrt(std::pow(s.root_radius+s.fillet_radius,2)-s.base_radius*s.base_radius)-s.fillet_radius)/s.base_radius;
    d.foot=involute(s,d.t0);
    d.center=d.foot-polar(s.fillet_radius,-s.half_base+d.t0+pi/2);
    d.root=d.center*(s.root_radius/radius(d.center));
  }
  const double rootHalf=-std::atan2(d.root.y,d.root.x);
  if(!std::isfinite(d.t0)||!std::isfinite(d.ta)||d.t0<0||d.t0>=d.ta||
      s.half_base-(d.ta-std::atan(d.ta))<=s.tolerance/s.tip_radius||
      rootHalf<=0||rootHalf>=pi/s.teeth-s.tolerance/s.root_radius)
    throw std::runtime_error("Gear fillet/involute leaves no separated root gap, flank or tip");
  return d;
}
Handle(Geom_BSplineCurve) flank(const GearData&s,const Dims&d,double phase,double z,bool mirrored) {
  // Cubic Hermite remainder: each component is bounded by max|f''''| h^4/384.
  // For the involute |f''''|=rb*sqrt(9+t*t); sqrt(2) also bounds vector error.
  const double bound=std::sqrt(2.)*s.base_radius*std::sqrt(9+d.ta*d.ta);
  const double needed=std::ceil((d.ta-d.t0)/std::pow(384*s.tolerance*.9/bound,.25));
  if(!std::isfinite(needed)||needed>127)throw std::runtime_error("Involute spline exceeds the bounded 256-pole budget");
  const int count=std::max(1,static_cast<int>(needed));
  TColgp_Array1OfPnt poles(1,2*count+2);
  TColStd_Array1OfReal knots(1,count+1);TColStd_Array1OfInteger multiplicities(1,count+1);
  const auto mapped=[&](V p) {return point(rotate(mirrored?mirror(p):p,phase),z);};
  poles.SetValue(1,mapped(involute(s,d.t0)));
  for(int i=0;i<count;++i) {
    const double a=d.t0+(d.ta-d.t0)*i/count,b=d.t0+(d.ta-d.t0)*(i+1)/count,h=(b-a)/3;
    const V p=involute(s,a),q=involute(s,b);
    // C1 composite Hermite form: the shared endpoint is the knot-insertion
    // blend of the neighboring derivative poles, so no refitting is needed.
    poles.SetValue(2*i+2,mapped(p+derivative(s,a)*h));
    poles.SetValue(2*i+3,mapped(q-derivative(s,b)*h));
  }
  poles.SetValue(2*count+2,mapped(involute(s,d.ta)));
  for(int i=0;i<=count;++i) {
    knots.SetValue(i+1,d.t0+(d.ta-d.t0)*i/count);
    multiplicities.SetValue(i+1,i==0||i==count?4:2);
  }
  Handle(Geom_BSplineCurve) curve=new Geom_BSplineCurve(poles,knots,multiplicities,3);
  // Independently evaluate the native spline between its Hermite endpoints.
  for(int i=0;i<count;++i)for(double f:{.25,.5,.75}) {
    const double t=d.t0+(d.ta-d.t0)*(i+f)/count;
    if(curve->Value(t).Distance(mapped(involute(s,t)))>s.tolerance)
      throw std::runtime_error("Native involute exceeds its declared profile error bound");
  }
  return curve;
}
TopoDS_Edge arc(V center,V a,V b,bool clockwise,double phase,double z) {
  const double first=std::atan2(a.y-center.y,a.x-center.x),last=std::atan2(b.y-center.y,b.x-center.x);
  double delta=std::fmod(last-first+2*pi,2*pi);if(clockwise)delta-=2*pi;
  const V mid=center+polar(radius(a-center),first+delta/2);
  return BRepBuilderAPI_MakeEdge(GC_MakeArcOfCircle(
    point(rotate(a,phase),z),point(rotate(mid,phase),z),point(rotate(b,phase),z)).Value()).Edge();
}
Profile profile(const GearData&s,double phase,double z) {
  const Dims d=dimensions(s);BRepBuilderAPI_MakeWire wire;Profile result;
  const auto add=[&](const TopoDS_Edge&e,uint32_t source) {
    wire.Add(e);if(!wire.IsDone())throw std::runtime_error("Native gear profile is disconnected");
    result.edges.push_back({wire.Edge(),source});
  };
  for(uint32_t i=0;i<s.teeth;++i) {
    const double a=phase+2*pi*i/s.teeth;const V start=involute(s,d.t0),tip=involute(s,d.ta);
    add(arc(d.center,d.root,d.foot,true,a,z),i*8);
    if(d.radial)add(BRepBuilderAPI_MakeEdge(point(rotate(d.foot,a),z),point(rotate(start,a),z)).Edge(),i*8+1);
    add(BRepBuilderAPI_MakeEdge(flank(s,d,a,z,false)).Edge(),i*8+2);
    add(arc({0,0},tip,mirror(tip),false,a,z),i*8+3);
    add(TopoDS::Edge(BRepBuilderAPI_MakeEdge(flank(s,d,a,z,true)).Edge().Reversed()),i*8+4);
    if(d.radial)add(BRepBuilderAPI_MakeEdge(point(rotate(mirror(start),a),z),point(rotate(mirror(d.foot),a),z)).Edge(),i*8+5);
    add(arc(mirror(d.center),mirror(d.foot),mirror(d.root),true,a,z),i*8+6);
    add(arc({0,0},mirror(d.root),rotate(d.root,2*pi/s.teeth),false,a,z),i*8+7);
  }
  result.wire=wire.Wire();return result;
}
template<class Builder>
void generated(Built&built,Builder&builder,const Profile&profile,uint8_t role) {
  for(const auto &edge:profile.edges)for(const auto &face:builder.Generated(edge.first))
    if(face.ShapeType()==TopAbs_FACE)built.faces.push_back({face,edge.second,role});
}
Built sweep(const GearData&s,double height,double twist,double phase,double z,uint8_t role) {
  const auto outline=profile(s,phase,z);Built result;
  if(std::abs(twist)<1e-12) {
    BRepPrimAPI_MakePrism prism(BRepBuilderAPI_MakeFace(outline.wire).Face(),gp_Vec(0,0,height));
    result.shape=prism.Shape();generated(result,prism,outline,role);
  } else {
    const auto main=BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(gp_Pnt(0,0,z),gp_Pnt(0,0,z+height)).Edge()).Wire();
    Handle(Geom_Surface) cylinder=new Geom_CylindricalSurface(gp_Ax3(),s.pitch_radius);
    Handle(Geom2d_Line) line=new Geom2d_Line(gp_Pnt2d(phase,z),gp_Dir2d(twist,height));
    auto edge=BRepBuilderAPI_MakeEdge(line,cylinder,0,std::hypot(twist,height)).Edge();
    if(!BRepLib::BuildCurve3d(edge,s.tolerance))throw std::runtime_error("Native auxiliary helix has no bounded 3D curve");
    const auto aux=BRepBuilderAPI_MakeWire(edge).Wire();
    BRepOffsetAPI_MakePipeShell pipe(main);pipe.SetMode(aux,true,BRepFill_NoContact);
    pipe.SetTolerance(s.tolerance,s.tolerance,1e-9);pipe.SetMaxDegree(11);pipe.SetMaxSegments(128);
    pipe.Add(outline.wire,false,false);pipe.Build();
    if(!pipe.IsDone()||!pipe.MakeSolid())throw std::runtime_error("Native involute helicoid sweep failed");
    result.shape=pipe.Shape();generated(result,pipe,outline,role);
  }
  if(!BRepCheck_Analyzer(result.shape).IsValid())throw std::runtime_error("Native gear sweep is invalid");
  return result;
}
template<class Operation>
Built boolean(Operation&operation,const Built&a,const Built&b) {
  TopTools_ListOfShape arguments,tools;arguments.Append(a.shape);tools.Append(b.shape);
  operation.SetArguments(arguments);operation.SetTools(tools);
  operation.SetNonDestructive(true);operation.SetRunParallel(false);operation.Build();
  if(!operation.IsDone()||operation.HasErrors()||operation.Shape().IsNull())throw std::runtime_error("Native gear Boolean failed");
  Built result{operation.Shape(),{}};TopTools_IndexedMapOfShape faces;TopExp::MapShapes(result.shape,TopAbs_FACE,faces);
  for(const auto*input:{&a,&b})for(const auto &source:input->faces) {
    const auto append=[&](const TopoDS_Shape&face) {
      if(faces.Contains(face))result.faces.push_back({face,source.source,source.role});
    };
    append(source.face);for(const auto &face:operation.Modified(source.face))append(face);
    if(result.faces.size()>16384)throw std::runtime_error("Native gear ancestry budget exceeded");
  }
  return result;
}
double phase_at(const GearData&s,double z) {
  if(s.style<2)return s.twist*z;
  const double band=(s.height-s.gap)/2;
  return s.twist*std::min({z,s.height-z,band});
}
// Internal material widens toward its outward root. The complementary void
// has involute flanks but its circular fillet belongs at the OUTER end,
// unlike an external gear's root fillet at the inner end.
std::unique_ptr<Shape> internal_ring(const GearData&s) {
  for(double v:{s.pitch_radius,s.base_radius,s.tip_radius,s.root_radius,s.fillet_radius,
      s.half_base,s.height,s.bore,s.outer,s.twist,s.gap,s.tolerance})
    if(!std::isfinite(v))throw std::runtime_error("Nonfinite internal gear input");
  if(s.teeth<4||s.teeth>128||s.base_radius<=0||s.tip_radius<=s.base_radius||
      s.tip_radius>=s.pitch_radius||s.root_radius<=s.pitch_radius||s.outer<=s.root_radius||
      s.fillet_radius<=0||s.root_radius-s.fillet_radius<=s.base_radius||s.height<=0||
      s.bore!=0||s.twist!=0||s.gap!=0||s.tolerance<=0)
    throw std::runtime_error("Internal spur exceeds validated radius, rim or sweep bounds");
  GearData cavity=s;
  cavity.half_base=pi/s.teeth-s.half_base;
  Dims d;
  d.t0=std::sqrt(std::pow(s.tip_radius/s.base_radius,2)-1);
  d.ta=(std::sqrt(std::pow(s.root_radius-s.fillet_radius,2)-s.base_radius*s.base_radius)+s.fillet_radius)/s.base_radius;
  d.radial=false;
  d.foot=involute(cavity,d.ta);
  d.center=d.foot+polar(s.fillet_radius,d.ta-cavity.half_base+pi/2);
  d.root=d.center*(s.root_radius/radius(d.center));
  const double rootHalf=-std::atan2(d.root.y,d.root.x);
  const double tipHalf=cavity.half_base-(d.t0-std::atan(d.t0));
  if(!std::isfinite(d.ta)||d.ta<=d.t0||radius(d.foot)>=s.root_radius||
      rootHalf<=s.tolerance/s.root_radius||rootHalf>=pi/s.teeth||
      tipHalf<=0||tipHalf>=pi/s.teeth-s.tolerance/s.tip_radius||
      std::abs(radius(d.center)-(s.root_radius-s.fillet_radius))>s.tolerance)
    throw std::runtime_error("Internal fillet leaves an unresolved root space, flank or tooth tip");
  Profile outline;BRepBuilderAPI_MakeWire wire;
  const auto add=[&](const TopoDS_Edge&e,uint32_t source) {
    wire.Add(e);if(!wire.IsDone())throw std::runtime_error("Internal cavity profile is disconnected");
    outline.edges.push_back({wire.Edge(),source});
  };
  const V start=involute(cavity,d.t0);
  for(uint32_t i=0;i<s.teeth;++i) {
    // Phase zero always denotes a MATERIAL tooth on +X, for either family.
    const double a=(2*pi*i+pi)/s.teeth;
    const uint32_t next=(i+1)%s.teeth;
    add(BRepBuilderAPI_MakeEdge(flank(cavity,d,a,0,false)).Edge(),i*8+4);
    add(arc(d.center,d.foot,d.root,false,a,0),i*8+6);
    add(arc({0,0},d.root,mirror(d.root),false,a,0),i*8+7);
    add(arc(mirror(d.center),mirror(d.root),mirror(d.foot),false,a,0),next*8);
    add(TopoDS::Edge(BRepBuilderAPI_MakeEdge(flank(cavity,d,a,0,true)).Edge().Reversed()),next*8+2);
    add(arc({0,0},mirror(start),rotate(start,2*pi/s.teeth),false,a,0),next*8+3);
  }
  outline.wire=wire.Wire();
  BRepPrimAPI_MakePrism prism(BRepBuilderAPI_MakeFace(outline.wire).Face(),gp_Vec(0,0,s.height));
  Built tool{prism.Shape(),{}};generated(tool,prism,outline,6);
  Built rim{BRepPrimAPI_MakeCylinder(s.outer,s.height).Shape(),{}};
  for(TopExp_Explorer it(rim.shape,TopAbs_FACE);it.More();it.Next())
    if(BRepAdaptor_Surface(TopoDS::Face(it.Current())).GetType()==GeomAbs_Cylinder)
      rim.faces.push_back({it.Current(),0,8});
  BRepAlgoAPI_Cut cut;Built built=boolean(cut,rim,tool);
  if(!BRepCheck_Analyzer(built.shape).IsValid())throw std::runtime_error("Internal ring is invalid");
  int count=0;for(TopExp_Explorer it(built.shape,TopAbs_SOLID);it.More();it.Next())++count;
  if(count!=1)throw std::runtime_error("Internal ring must retain one connected rim");
  BRepClass3d_SolidClassifier classifier;classifier.Load(built.shape);
  for(uint32_t i=0;i<s.teeth;++i)for(double fraction:{.2,.5,.8}) {
    const double a=2*pi*i/s.teeth,z=s.height*fraction;
    const auto inside=[&](double r,double angle) {
      classifier.Perform(gp_Pnt(r*std::cos(angle),r*std::sin(angle),z),s.tolerance*4);
      return classifier.State()==TopAbs_IN;
    };
    if(!inside(s.pitch_radius,a)||inside(s.pitch_radius,a+pi/s.teeth)||
        !inside((s.outer+s.root_radius)/2,a+pi/s.teeth)||inside(s.tip_radius*.99,a))
      throw std::runtime_error("Internal ring lost inward teeth, opening, spaces or rim");
  }
  auto result=std::make_unique<Shape>(Shape{built.shape,{},{}});
  TopTools_IndexedMapOfShape faces;TopExp::MapShapes(built.shape,TopAbs_FACE,faces);
  for(int i=1;i<=faces.Extent();++i) {
    const BRepAdaptor_Surface surface(TopoDS::Face(faces(i)));
    if(surface.GetType()==GeomAbs_Plane&&std::abs(surface.Plane().Axis().Direction().Z())>1-1e-10) {
      const double z=surface.Plane().Location().Z();
      if(std::abs(z)<s.tolerance*16||std::abs(z-s.height)<s.tolerance*16) {
        result->seeds.push_back({static_cast<uint32_t>(i-1),0,static_cast<uint8_t>(std::abs(z)<s.tolerance*16?0:1)});continue;
      }
    }
    bool found=false;for(const auto &source:built.faces)if(source.face.IsSame(faces(i))) {
      result->seeds.push_back({static_cast<uint32_t>(i-1),source.source,source.role});found=true;
    }
    if(!found)throw std::runtime_error("Internal ring face has no exact native ancestry");
  }
  return result;
}
}
std::unique_ptr<Shape> involute_gear(const GearData&s) {
  try {
    if(s.style==4)return internal_ring(s);
    for(double v:{s.pitch_radius,s.base_radius,s.tip_radius,s.root_radius,s.fillet_radius,
        s.half_base,s.height,s.bore,s.twist,s.gap,s.hub_radius,s.hub_start,s.hub_end,s.tolerance})
      if(!std::isfinite(v))throw std::runtime_error("Nonfinite native gear input");
    if(s.teeth<4||s.teeth>128||s.style>3||s.base_radius<=0||s.pitch_radius<s.base_radius||
        s.root_radius<=0||s.root_radius>=s.pitch_radius||s.tip_radius<=s.pitch_radius||
        s.fillet_radius<=0||s.bore<0||s.bore>=s.root_radius||s.height<=0||s.tolerance<=0||
        s.gap<0||s.gap>=s.height||std::abs(s.twist*s.height)>pi)
      throw std::runtime_error("Native gear exceeds its validated input bounds");
    if(s.hub_radius<0 || (s.hub_radius>0 &&
        (s.hub_radius>=s.root_radius || s.hub_radius<=s.bore ||
         s.hub_end<=s.hub_start || std::min(s.hub_end,s.height)-std::max(s.hub_start,0.)<=s.tolerance)))
      throw std::runtime_error("Native gear hub has no connected overlap, root clearance or bore wall");
    Built built;
    if(s.style<2)built=sweep(s,s.height,s.twist*s.height,0,0,6);
    else {
      built.shape=BRepPrimAPI_MakeCylinder(s.root_radius,s.height).Shape();
      for(TopExp_Explorer it(built.shape,TopAbs_FACE);it.More();it.Next())
        if(BRepAdaptor_Surface(TopoDS::Face(it.Current())).GetType()==GeomAbs_Cylinder)
          built.faces.push_back({it.Current(),0,3});
      const double band=(s.height-s.gap)/2;
      BRepAlgoAPI_Fuse lower;built=boolean(lower,built,sweep(s,band,s.twist*band,0,0,6));
      BRepAlgoAPI_Fuse upper;built=boolean(upper,built,sweep(s,band,-s.twist*band,s.twist*band,s.height-band,7));
    }
    if(s.hub_radius>0) {
      Built hub{BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0,0,s.hub_start),gp_Dir(0,0,1)),
          s.hub_radius,s.hub_end-s.hub_start).Shape(),{}};
      for(TopExp_Explorer it(hub.shape,TopAbs_FACE);it.More();it.Next()) {
        const BRepAdaptor_Surface surface(TopoDS::Face(it.Current()));
        const uint8_t role=surface.GetType()==GeomAbs_Cylinder?9:
            (std::abs(surface.Plane().Location().Z()-s.hub_start)<s.tolerance*16?10:11);
        hub.faces.push_back({it.Current(),0,role});
      }
      BRepAlgoAPI_Fuse fuse;built=boolean(fuse,built,hub);
    }
    if(s.bore>0) {
      const double low=s.hub_radius>0?std::min(0.,s.hub_start):0.;
      const double high=s.hub_radius>0?std::max(s.height,s.hub_end):s.height;
      Built tool{BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0,0,low-1),gp_Dir(0,0,1)),s.bore,high-low+2).Shape(),{}};
      BRepAlgoAPI_Cut cut;built=boolean(cut,built,tool);
    }
    if(!BRepCheck_Analyzer(built.shape).IsValid())throw std::runtime_error("Finished native gear is invalid");
    int count=0;for(TopExp_Explorer it(built.shape,TopAbs_SOLID);it.More();it.Next())++count;
    if(count!=1)throw std::runtime_error("Gear must remain one connected solid");
    BRepClass3d_SolidClassifier classifier;classifier.Load(built.shape);
    for(double fraction:{.13,.37,.5,.63,.87})for(uint32_t tooth=0;tooth<s.teeth;++tooth) {
      const double z=s.height*fraction,phase=phase_at(s,z)+2*pi*tooth/s.teeth;
      const bool inGap=s.style==3&&z>(s.height-s.gap)/2&&z<(s.height+s.gap)/2;
      const auto inside=[&](double angle) {
        classifier.Perform(gp_Pnt(s.pitch_radius*std::cos(angle),s.pitch_radius*std::sin(angle),z),s.tolerance*4);
        return classifier.State()==TopAbs_IN||classifier.State()==TopAbs_ON;
      };
      if(inside(phase)!=!inGap||inside(phase+pi/s.teeth))throw std::runtime_error("Native gear lost its requested teeth, spaces, twist or center gap");
    }
    auto result=std::make_unique<Shape>(Shape{built.shape,{},{}});
    TopTools_IndexedMapOfShape faces;TopExp::MapShapes(built.shape,TopAbs_FACE,faces);
    for(int i=1;i<=faces.Extent();++i) {
      const BRepAdaptor_Surface surface(TopoDS::Face(faces(i)));int cap=-1;
      if(surface.GetType()==GeomAbs_Plane&&std::abs(surface.Plane().Axis().Direction().Z())>1-1e-10) {
        const double z=surface.Plane().Location().Z();
        if(std::abs(z)<s.tolerance*16)cap=0;
        else if(std::abs(z-s.height)<s.tolerance*16)cap=1;
        else if(s.style==3&&std::abs(z-(s.height-s.gap)/2)<s.tolerance*16)cap=4;
        else if(s.style==3&&std::abs(z-(s.height+s.gap)/2)<s.tolerance*16)cap=5;
      }
      bool found=false;
      if(cap>=0) {result->seeds.push_back({static_cast<uint32_t>(i-1),0,static_cast<uint8_t>(cap)});found=true;}
      if(s.bore>0&&surface.GetType()==GeomAbs_Cylinder&&std::abs(surface.Cylinder().Radius()-s.bore)<s.tolerance*16) {
        result->seeds.push_back({static_cast<uint32_t>(i-1),0,2});continue;
      }
      for(const auto &source:built.faces)if(source.face.IsSame(faces(i))) {
        result->seeds.push_back({static_cast<uint32_t>(i-1),source.source,source.role});found=true;
      }
      if(!found)throw std::runtime_error("Gear face has no exact native ancestry");
    }
    return result;
  } catch(const Standard_Failure&e) {
    throw std::runtime_error(e.GetMessageString()?e.GetMessageString():"Native gear operation failed");
  }
}
}
