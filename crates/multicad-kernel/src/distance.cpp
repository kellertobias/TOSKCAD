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
#include <limits>
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
static DistanceData measured(const TopoDS_Shape &left,const TopoDS_Shape &right,double tolerance,uint32_t maximum) {
  BRepExtrema_DistShapeShape operation;operation.LoadS1(left);operation.LoadS2(right);operation.SetDeflection(tolerance);
  operation.SetMultiThread(false);
  if(!operation.Perform()||!operation.IsDone()||operation.NbSolution()<1)throw std::runtime_error("Native minimum distance did not converge");
  const double value=operation.Value();if(!std::isfinite(value)||value<0.)throw std::runtime_error("Native distance is not a finite nonnegative length");
  DistanceData result{};result.distance=value;result.total_solutions=static_cast<uint32_t>(operation.NbSolution());result.inner=operation.InnerSolution();result.complete=static_cast<uint32_t>(operation.NbSolution())<=maximum;
  const auto count=std::min(static_cast<uint32_t>(operation.NbSolution()),maximum);
  for(uint32_t i=0;i<count;++i){const auto p=operation.PointOnShape1(i+1),q=operation.PointOnShape2(i+1);result.witnesses.push_back(DistanceWitnessData{Point3{p.X(),p.Y(),p.Z()},Point3{q.X(),q.Y(),q.Z()}});}
  return result;
}
DistanceData minimum_distance(const Shape &a,const Shape &b,const DistanceOptionsData &options) {
 try {
  const double tolerance=options.tolerance;const uint32_t maximum=options.maximum;
  if(!std::isfinite(tolerance)||tolerance<=0.||maximum==0||maximum>256)throw std::runtime_error("Invalid native measurement limits");
  const auto left=selected(a,options.a_kind,options.a_index),right=selected(b,options.b_kind,options.b_index);
  if(options.a_kind!=0||options.b_kind!=0)return measured(left,right,tolerance,maximum);
  // DistShapeShape's solid containment test does not treat a Compound as a
  // filled union. Query its actual solid pairs, retaining a bounded set of
  // native extrema witnesses; no bounding-box distance becomes a result.
  TopTools_IndexedMapOfShape solidsA,solidsB;
  TopExp::MapShapes(left,TopAbs_SOLID,solidsA);TopExp::MapShapes(right,TopAbs_SOLID,solidsB);
  if(solidsA.IsEmpty()||solidsB.IsEmpty()||solidsA.Extent()>128||solidsB.Extent()>128)throw std::runtime_error("Whole-solid clearance supports 1 to 128 solids per Part");
  struct Summary {double distance;uint32_t count;bool inner,complete;};
  std::vector<Summary> summaries;
  std::vector<std::pair<double,DistanceWitnessData>> witnesses;
  DistanceData result{};result.distance=std::numeric_limits<double>::infinity();
  for(int i=1;i<=solidsA.Extent();++i)for(int j=1;j<=solidsB.Extent();++j) {
    const auto candidate=measured(solidsA(i),solidsB(j),tolerance,maximum);
    summaries.push_back(Summary{candidate.distance,candidate.total_solutions,candidate.inner,candidate.complete});
    result.distance=std::min(result.distance,candidate.distance);
    witnesses.erase(std::remove_if(witnesses.begin(),witnesses.end(),[&](const auto &w){return w.first>result.distance+tolerance;}),witnesses.end());
    if(candidate.distance<=result.distance+tolerance)
      for(const auto &w:candidate.witnesses)if(witnesses.size()<maximum)witnesses.emplace_back(candidate.distance,w);
  }
  bool complete=true;
  for(const auto &candidate:summaries)if(candidate.distance<=result.distance+tolerance) {
    if(candidate.count>1000000-result.total_solutions)throw std::runtime_error("Clearance solution budget exceeded");
    result.total_solutions+=candidate.count;result.inner=result.inner||candidate.inner;complete=complete&&candidate.complete;
  }
  for(const auto &w:witnesses)result.witnesses.push_back(w.second);
  result.complete=complete&&result.total_solutions==result.witnesses.size();
  return result;
 } catch(const Standard_Failure &error){throw std::runtime_error(error.GetMessageString()?error.GetMessageString():"Native minimum-distance failure");}
}
}
