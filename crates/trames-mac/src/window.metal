// SPDX-License-Identifier: GPL-3.0-or-later
#include <metal_stdlib>
using namespace metal;
struct SceneImage {
    float2 origin;
    float2 size;
    float2 viewport;
    uint offset;
    uint stride;
    uint width;
    uint height;
    uint alpha;
    uint transform;
};
struct Vertex { float4 position [[position]]; float2 uv; };
vertex Vertex waylandVertex(uint id [[vertex_id]], constant SceneImage &image [[buffer(0)]]) {
    constexpr float2 quad[] = {float2(0,0),float2(1,0),float2(0,1),float2(1,1)};
    float2 p = (image.origin + quad[id] * image.size) / image.viewport;
    return {float4(p.x * 2 - 1, 1 - p.y * 2, 0, 1), quad[id]};
}
static float2 bufferUV(float2 uv, uint transform) {
    if (transform & 4) uv.x=1-uv.x;
    switch (transform & 3) {
      case 1: return float2(uv.y,1-uv.x);
      case 2: return 1-uv;
      case 3: return float2(1-uv.y,uv.x);
      default: return uv;
    }
}
fragment float4 waylandShm(Vertex vtx [[stage_in]], const device uchar *bytes [[buffer(0)]],
                           constant SceneImage &image [[buffer(1)]]) {
    float2 uv=bufferUV(vtx.uv,image.transform);
    uint2 at = min(uint2(uv * float2(image.width, image.height)), uint2(image.width-1,image.height-1));
    uint p = image.offset + at.y * image.stride + at.x * 4;
    return float4(bytes[p+2],bytes[p+1],bytes[p],image.alpha ? bytes[p+3] : 255) / 255.0;
}
fragment float4 waylandTexture(Vertex vtx [[stage_in]], texture2d<float> image [[texture(0)]], constant SceneImage &info [[buffer(1)]]) {
    constexpr sampler linear(coord::normalized, filter::linear, address::clamp_to_edge);
    float4 color=image.sample(linear,bufferUV(vtx.uv,info.transform));
    if (!info.alpha) color.a=1;
    return color;
}
