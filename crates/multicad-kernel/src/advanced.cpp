#include "advanced.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepBuilderAPI_Copy.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRep_Builder.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Iterator.hxx>
#include <gp_Pnt.hxx>
#include <algorithm>
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
  auto result=std::make_unique<Shape>(Shape{shape,{},{}});
  record_validated_shape(*result);return result;
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
    CompoundMember member{copy.Shape(),{},{},{}};
    member.exact_copy=member.copied.ShapeType()==source.value.ShapeType()&&member.copied.Orientation()==source.value.Orientation();
    try {
      for(TopExp_Explorer it(source.value,TopAbs_SOLID);it.More();it.Next())++member.solids;
      uint32_t copiedSolids=0;for(TopExp_Explorer it(member.copied,TopAbs_SOLID);it.More();it.Next())++copiedSolids;
      member.exact_copy=member.exact_copy&&member.solids==copiedSolids;
    } catch(const std::bad_alloc &) {member.exact_copy=false;}
    for(uint8_t kind=0;kind<3;++kind) {
      const auto input=map(source.value,kind);
      std::optional<TopTools_IndexedMapOfShape> output;
      std::set<int> covered;
      if(member.exact_copy)try {
        output=map(member.copied,kind);
        member.exact_copy=input.Extent()==output->Extent();
      } catch(const std::bad_alloc &) {member.exact_copy=false;}
      for(int i=1;i<=input.Extent();++i) {
        const auto shape=copy.ModifiedShape(input(i));
        if(shape.IsNull())throw std::runtime_error("Compound member lacks exact copy history");
        if(member.exact_copy)try {
          const int index=output->FindIndex(shape);
          member.exact_copy=index>0&&covered.insert(index).second&&
              (kind!=0||(*output)(index).Orientation()==input(i).Orientation());
        } catch(const std::bad_alloc &) {member.exact_copy=false;}
        // Keep mandatory native history unchanged. ModifiedShape's canonical
        // map wrapper need not carry the actual occurrence orientation.
        member.links.push_back(CompoundLink{kind,static_cast<uint32_t>(i-1),shape,input(i).Orientation()});
      }
    }
    // Pure native copy preserves geometry; capture only completed successful
    // certificates. Never hold a source lock while running native operations.
    if(member.exact_copy)try {
      {std::lock_guard<std::mutex> lock(source.face_cache->mutex);member.faces=source.face_cache->value;}
      {std::lock_guard<std::mutex> lock(source.volume_cache->mutex);member.volume=source.volume_cache->certificate;}
    } catch(const std::bad_alloc &) {member.faces.reset();member.volume.reset();}
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
    // Distinct deep copies and all existing ancestry remain unchanged. Only
    // complete, bijectively mapped accepted measurements can be retained.
    try {
      const auto output=map(compound,0);
      std::vector<CachedFace> completed(output.Extent());
      std::vector<bool> covered(output.Extent(),false);
      bool complete=true;
      for(const auto &member:job.members) {
        const auto copied=map(member.copied,0);
        if(!member.exact_copy||!member.faces||member.faces->size()!=static_cast<size_t>(copied.Extent())) {complete=false;break;}
        for(const auto &properties:*member.faces) {
          // Face links are written first, in exact source ordinal order.
          // Checked indexing keeps transport linear in the topology budget.
          if(properties.index>=member.links.size()) {complete=false;break;}
          const auto &link=member.links[properties.index];
          if(link.kind!=0||link.input!=properties.index) {complete=false;break;}
          const int index=output.FindIndex(link.copied);
          if(index<=0||covered[index-1]||output(index).Orientation()!=link.orientation) {complete=false;break;}
          completed[index-1]=describe_face(TopoDS::Face(output(index)),index-1,
              gp_Pnt(properties.centroid[0],properties.centroid[1],properties.centroid[2]),properties.area);
          covered[index-1]=true;
        }
        if(!complete)break;
      }
      if(complete&&std::find(covered.begin(),covered.end(),false)==covered.end())result->face_cache->value=std::move(completed);
    } catch(const Standard_Failure &) { /* Ordinary face measurement remains available. */ }
      catch(const std::bad_alloc &) { /* Retention is optional. */ }
    // The integral of a one-member compound equals that exact copied member.
    // Do not infer/sum certificates for multiple members, even coincident ones.
    try {if(job.members.size()==1) {
      const auto &member=job.members.front();
      uint32_t solids=0;for(TopExp_Explorer it(compound,TopAbs_SOLID);it.More();it.Next())++solids;
      TopoDS_Iterator child(compound);
      const bool onlyChild=child.More()&&child.Value().IsEqual(member.copied);
      if(child.More())child.Next();
      if(onlyChild&&!child.More()&&member.exact_copy&&solids==member.solids&&member.volume) {
        const auto &v=*member.volume;
        if(std::isfinite(v.mass)&&v.mass>0.&&std::isfinite(v.relative_error)&&v.relative_error>=0.&&v.relative_error<=1e-7)
          result->volume_cache->certificate=v;
      }
    }} catch(const Standard_Failure &) { /* Ordinary native volume remains available. */ }
       catch(const std::bad_alloc &) { /* Optional proof cannot invalidate geometry. */ }
    return result;
  });
}
bool compound_property_fallback_probe(const Shape &source) {
  for(int kind=0;kind<2;++kind) {
    auto job=compound_job();compound_add(*job,source);
    if(!job->members[0].faces||job->members[0].faces->empty())return false;
    if(kind==0)job->members[0].faces->pop_back();
    else job->members[0].faces->front().index=UINT32_MAX;
    auto result=compound_finish(*job);
    if(result->face_cache->value||!valid(*result))return false;
    const auto measured=faces(*result);
    if(measured.empty()||face_integration_count(*result)!=measured.size())return false;
  }
  return true;
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
