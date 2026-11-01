#include "advanced.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepBuilderAPI_Copy.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRep_Builder.hxx>
#include <TopoDS_Compound.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Pln.hxx>
#include <set>
#include <array>
#include <cmath>
#include <stdexcept>
namespace multicad::kernel {
namespace {
template<class F> auto guarded(F function)->decltype(function()) {
  try { return function(); } catch(const Standard_Failure &error) {
    throw std::runtime_error(error.GetMessageString()?error.GetMessageString():"Native advanced solid operation failed");
  }
}
constexpr std::array<TopAbs_ShapeEnum,3> kinds={TopAbs_FACE,TopAbs_EDGE,TopAbs_VERTEX};
TopTools_IndexedMapOfShape map(const TopoDS_Shape &shape,uint8_t kind) {
  TopTools_IndexedMapOfShape result;TopExp::MapShapes(shape,kinds.at(kind),result);
  if(result.Extent()>100000)throw std::runtime_error("Advanced topology budget exceeded");
  return result;
}
std::unique_ptr<Shape> checked(const TopoDS_Shape &shape) {
  if(shape.IsNull()||!BRepCheck_Analyzer(shape).IsValid())throw std::runtime_error("Advanced operation produced invalid native geometry");
  return std::make_unique<Shape>(Shape{shape,{},{}});
}
template<class Operation> void history(Shape &result,const Shape &source,BRepBuilderAPI_Copy &copy,Operation &operation) {
  for(uint8_t kind=0;kind<3;++kind) {
    const auto input=map(source.value,kind),output=map(result.value,kind);
    for(int i=1;i<=input.Extent();++i) {
      const auto copied=copy.ModifiedShape(input(i));
      if(copied.IsNull())throw std::runtime_error("Advanced copy history is incomplete");
      std::set<int> descendants;
      auto append=[&](const TopoDS_Shape &shape){int index=output.FindIndex(shape);if(index>0)descendants.insert(index);};
      append(copied);
      for(const auto &shape:operation.Modified(copied))append(shape);
      for(const auto &shape:operation.Generated(copied))append(shape);
      for(int index:descendants) {
        if(result.ancestry.size()>=1000000)throw std::runtime_error("Advanced history budget exceeded");
        result.ancestry.push_back(NativeAncestry{kind,0,static_cast<uint32_t>(index-1),static_cast<uint32_t>(i-1)});
      }
    }
  }
}
std::vector<TopoDS_Face> selections(const Shape &source,BRepBuilderAPI_Copy &copy,rust::Slice<const uint32_t> indices,bool empty_allowed) {
  if((indices.empty()&&!empty_allowed)||indices.size()>128)throw std::runtime_error("Advanced face selection budget exceeded");
  const auto faces=map(source.value,0);std::set<uint32_t> unique;std::vector<TopoDS_Face> result;
  for(uint32_t i:indices) {
    if(i>=static_cast<uint32_t>(faces.Extent())||!unique.insert(i).second)throw std::runtime_error("Selected face is absent or duplicated");
    const auto copied=copy.ModifiedShape(faces(static_cast<int>(i+1)));
    if(copied.IsNull())throw std::runtime_error("Selected face lacks exact copy history");
    result.push_back(TopoDS::Face(copied));
  }
  return result;
}
template<class Operation> void seed_generated(Shape &result,Operation &operation,const TopoDS_Shape &source,uint32_t index,uint8_t role,bool modified) {
  const auto faces=map(result.value,0);std::set<int> found;
  auto append=[&](const TopoDS_Shape &shape){int face=faces.FindIndex(shape);if(face>0)found.insert(face);};
  for(const auto &shape:operation.Generated(source))append(shape);
  if(modified) {append(source);for(const auto &shape:operation.Modified(source))append(shape);}
  for(int face:found) {
    if(result.seeds.size()>=1000000)throw std::runtime_error("Advanced generated-face history budget exceeded");
    result.seeds.push_back(NativeSeed{static_cast<uint32_t>(face-1),index,role});
  }
}
}
std::unique_ptr<CompoundJob> compound_job() { return std::make_unique<CompoundJob>(); }
void compound_add(CompoundJob &job,const Shape &source) {
  guarded([&] {
    if(job.members.size()>=128)throw std::runtime_error("Compound member budget exceeded");
    std::size_t count=0;
    for(uint8_t kind=0;kind<3;++kind)count+=static_cast<std::size_t>(map(source.value,kind).Extent());
    if(job.topology_count+count>100000)throw std::runtime_error("Compound topology budget exceeded");
    BRepBuilderAPI_Copy copy(source.value,true,false);if(!copy.IsDone())throw std::runtime_error("Compound member copy failed");
    CompoundMember member{copy.Shape(),{}};
    for(uint8_t kind=0;kind<3;++kind) {
      const auto input=map(source.value,kind);
      for(int i=1;i<=input.Extent();++i) {
        const auto shape=copy.ModifiedShape(input(i));
        if(shape.IsNull())throw std::runtime_error("Compound member lacks exact copy history");
        member.links.push_back(CompoundLink{kind,static_cast<uint32_t>(i-1),shape});
      }
    }
    job.members.push_back(std::move(member));
    job.topology_count+=count;
  });
}
std::unique_ptr<Shape> compound_finish(CompoundJob &job) {
  return guarded([&] {
    if(job.members.empty())throw std::runtime_error("Compound needs at least one member");
    BRep_Builder builder;TopoDS_Compound compound;builder.MakeCompound(compound);
    for(const auto &member:job.members)builder.Add(compound,member.copied);
    auto result=checked(compound);
    for(uint8_t kind=0;kind<3;++kind) {
      const auto output=map(compound,kind);
      for(size_t i=0;i<job.members.size();++i)for(const auto &link:job.members[i].links)if(link.kind==kind) {
        const int index=output.FindIndex(link.copied);
        if(index<=0)throw std::runtime_error("Compound lost a member topology element");
        if(result->ancestry.size()>=1000000)throw std::runtime_error("Compound history budget exceeded");
        result->ancestry.push_back(NativeAncestry{kind,static_cast<uint8_t>(i),static_cast<uint32_t>(index-1),link.input});
      }
    }
    return result;
  });
}
std::unique_ptr<Shape> solid_component_at(const Shape &source,uint32_t index) {
  return guarded([&] {
    if(index>=1024)throw std::runtime_error("Solid component index exceeds the component budget");
    TopoDS_Shape selected;uint32_t ordinal=0;
    // Match solid_count's traversal; these ordinals are evaluation-local only.
    for(TopExp_Explorer e(source.value,TopAbs_SOLID);e.More();e.Next(),++ordinal) {
      if(ordinal==index) {selected=e.Current();break;}
    }
    if(selected.IsNull())throw std::runtime_error("Solid component index is unavailable");
    BRepBuilderAPI_Copy copy(selected,true,false);
    if(!copy.IsDone())throw std::runtime_error("Solid component copy failed");
    auto result=checked(copy.Shape());
    for(uint8_t kind=0;kind<3;++kind) {
      const auto parent=map(source.value,kind),input=map(selected,kind),output=map(result->value,kind);
      for(int i=1;i<=input.Extent();++i) {
        const auto copied=copy.ModifiedShape(input(i));
        const int ancestor=parent.FindIndex(input(i)),descendant=output.FindIndex(copied);
        if(copied.IsNull()||ancestor<=0||descendant<=0)throw std::runtime_error("Solid component lacks exact copy ancestry");
        if(result->ancestry.size()>=1000000)throw std::runtime_error("Solid component history budget exceeded");
        result->ancestry.push_back(NativeAncestry{kind,0,static_cast<uint32_t>(descendant-1),static_cast<uint32_t>(ancestor-1)});
      }
    }
    return result;
  });
}
std::unique_ptr<Shape> shell_solid(const Shape &source,rust::Slice<const uint32_t> indices,double thickness,double tolerance) {
  return guarded([&] {
    if(!std::isfinite(thickness)||thickness==0.||std::abs(thickness)>1e7||!std::isfinite(tolerance)||tolerance<1e-7||tolerance>1e-3)throw std::runtime_error("Invalid shell thickness or tolerance");
    BRepBuilderAPI_Copy copy(source.value,true,false);if(!copy.IsDone())throw std::runtime_error("Shell input copy failed");
    const auto openings=selections(source,copy,indices,true);TopTools_ListOfShape closing;
    for(const auto &face:openings)closing.Append(face);
    BRepOffsetAPI_MakeThickSolid operation;
    operation.MakeThickSolidByJoin(copy.Shape(),closing,-thickness,tolerance,BRepOffset_Skin,false,false,GeomAbs_Intersection,false);
    if(!operation.IsDone())throw std::runtime_error("Shell offset failed for the selected faces and thickness");
    auto result=checked(operation.Shape());
    if(indices.empty()) {TopTools_IndexedMapOfShape shells;TopExp::MapShapes(result->value,TopAbs_SHELL,shells);if(shells.Extent()!=2)throw std::runtime_error("Closed hollow thickness is unsupported by this native result; select an opening face");}
    history(*result,source,copy,operation);
    const auto faces=map(source.value,0);
    for(int i=1;i<=faces.Extent();++i)seed_generated(*result,operation,copy.ModifiedShape(faces(i)),static_cast<uint32_t>(i-1),5,false);
    for(uint32_t i=0;i<openings.size();++i) {
      seed_generated(*result,operation,openings[i],i,3,true);
      for(TopExp_Explorer edge(openings[i],TopAbs_EDGE);edge.More();edge.Next())seed_generated(*result,operation,edge.Current(),i,3,false);
    }
    return result;
  });
}
std::unique_ptr<Shape> draft_solid(const Shape &source,rust::Slice<const uint32_t> indices,const Point3 &direction,const Frame3 &neutral,double angle) {
  return guarded([&] {
    if(!std::isfinite(angle)||std::abs(angle)<1e-8||std::abs(angle)>=std::acos(-1.)/2.)throw std::runtime_error("Invalid signed draft angle");
    BRepBuilderAPI_Copy copy(source.value,true,false);if(!copy.IsDone())throw std::runtime_error("Draft input copy failed");
    const auto selected=selections(source,copy,indices,false);
    gp_Vec u(neutral.u.x,neutral.u.y,neutral.u.z),v(neutral.v.x,neutral.v.y,neutral.v.z);
    const gp_Pln plane(gp_Pnt(neutral.origin.x,neutral.origin.y,neutral.origin.z),gp_Dir(u.Crossed(v)));
    const gp_Dir pull(direction.x,direction.y,direction.z);
    BRepOffsetAPI_DraftAngle operation(copy.Shape());
    for(const auto &face:selected) {
      const auto type=BRepAdaptor_Surface(face).GetType();
      if(type!=GeomAbs_Plane&&type!=GeomAbs_Cylinder&&type!=GeomAbs_Cone)throw std::runtime_error("Draft supports planar, cylindrical or conical faces only");
      operation.Add(face,pull,angle,plane);
      if(!operation.AddDone())throw std::runtime_error("Selected draft face cannot be modified with this direction and neutral plane");
    }
    operation.Build();if(!operation.IsDone())throw std::runtime_error("Draft failed; choose a compatible angle, neutral plane and faces");
    auto result=checked(operation.Shape());
    for(uint8_t kind=0;kind<3;++kind)if(map(source.value,kind).Extent()!=map(result->value,kind).Extent())throw std::runtime_error("Draft topology changes are unsupported");
    history(*result,source,copy,operation);
    for(uint32_t i=0;i<selected.size();++i)seed_generated(*result,operation,selected[i],i,3,true);
    return result;
  });
}
}
