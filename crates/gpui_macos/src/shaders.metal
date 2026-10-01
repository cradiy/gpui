#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct SubtreeVertexOutput {
  float4 position [[position]];
};

vertex SubtreeVertexOutput subtree_vertex(uint vertex_id [[vertex_id]]) {
  float2 uv = float2((vertex_id << 1) & 2, vertex_id & 2);
  return {float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0)};
}

fragment float4 subtree_fragment(SubtreeVertexOutput input [[stage_in]],
                                texture2d<float> source [[texture(0)]]) {
  return source.read(uint2(input.position.xy));
}
