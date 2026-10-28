#include "distance.hpp"
#include "multicad-kernel/src/lib.rs.h"
#include <BRepExtrema_DistShapeShape.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <array>
#include <algorithm>
#include <cmath>
#include <stdexcept>
namespace multicad::kernel {
static TopoDS_Shape selected(const Shape &shape,uint8_t kind,uint32_t ordinal) {
  if(kind==0) {if(ordinal!=0)throw std::runtime_error("Whole-solid selection has no topology ordinal");return shape.value;}
  if(kind>3)throw std::runtime_error("Unknown B-rep measurement selection");
  static const std::array<TopAbs_ShapeEnum,3> kinds={TopAbs_FACE,TopAbs_EDGE,TopAbs_VERTEX};
  TopTools_IndexedMapOfShape map;TopExp::MapShapes(shape.value,kinds[kind-1],map);
  if(map.Extent()>100000)throw std::runtime_error("Measurement topology budget exceeded");
  if(ordinal>=static_cast<uint32_t>(map.Extent()))throw std::runtime_error("Selected B-rep topology is missing");
  return map(static_cast<Standard_Integer>(ordinal+1));
}
DistanceData minimum_distance(const Shape &a,const Shape &b,const DistanceOptionsData &options) {
 const auto ak=options.a_kind,bk=options.b_kind;const auto ai=options.a_index,bi=options.b_index,maximum=options.maximum;const double tolerance=options.tolerance;
 try {
  if(!std::isfinite(tolerance)||tolerance<=0.||maximum==0||maximum>256)throw std::runtime_error("Invalid native measurement limits");
  const auto left=selected(a,ak,ai),right=selected(b,bk,bi);
  BRepExtrema_DistShapeShape operation;operation.LoadS1(left);operation.LoadS2(right);operation.SetDeflection(tolerance);
  operation.SetMultiThread(false);
  if(!operation.Perform()||!operation.IsDone()||operation.NbSolution()<1)throw std::runtime_error("Native minimum distance did not converge");
  const double value=operation.Value();if(!std::isfinite(value)||value<0.)throw std::runtime_error("Native distance is not a finite nonnegative length");
  DistanceData result{};result.distance=value;result.total_solutions=static_cast<uint32_t>(operation.NbSolution());result.inner=operation.InnerSolution();result.complete=static_cast<uint32_t>(operation.NbSolution())<=maximum;
  const auto count=std::min(static_cast<uint32_t>(operation.NbSolution()),maximum);
  for(uint32_t i=0;i<count;++i){const auto p=operation.PointOnShape1(i+1),q=operation.PointOnShape2(i+1);result.witnesses.push_back(DistanceWitnessData{Point3{p.X(),p.Y(),p.Z()},Point3{q.X(),q.Y(),q.Z()}});}
  return result;
 } catch(const Standard_Failure &error){throw std::runtime_error(error.GetMessageString()?error.GetMessageString():"Native minimum-distance failure");}
}
}
