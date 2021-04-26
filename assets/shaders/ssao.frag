#version 450
#include "render_params.glsl"

layout(location=0) in vec2 in_uv;

layout(location=0) out vec4 out_ssao;

layout(set = 0, binding = 0) uniform texture2DMS t_depth;
layout(set = 0, binding = 1) uniform sampler s_depth;

layout(set = 1, binding = 0) uniform Uni {RenderParams params;};

float PHI = 1.618034;

float fastnoise(vec2 xy, float seed){
    return fract(tan(distance(xy*PHI, xy)*seed)*xy.x);
}

float sample_depth(vec2 coords) {
    return texelFetch(sampler2DMS(t_depth, s_depth), ivec2(coords * params.viewport), 0).r;
}

vec2 derivative_from_depth(float depth, vec2 texcoords) {
    ivec2 c = ivec2(texcoords * params.viewport);

    float depthx = texelFetch(sampler2DMS(t_depth, s_depth), c + ivec2(1, 0), 0).r;
    float depthy = texelFetch(sampler2DMS(t_depth, s_depth), c + ivec2(0, 1), 0).r;

    return params.viewport * vec2(depthx - depth, depthy - depth);
}

void main() {
    const float total_strength = 1.0;
    const float base = 0.1;

    const float area = 0.5;
    const float falloff = 0.0015;

    const float radius = 0.02;

    const int samples = 16;
    vec3 sample_sphere[samples] = {
    vec3( 0.5381, 0.1856,-0.4319), vec3( 0.1379, 0.2486, 0.4430),
    vec3( 0.3371, 0.5679,-0.0057), vec3(-0.6999,-0.0451,-0.0019),
    vec3( 0.0689,-0.1598,-0.8547), vec3( 0.0560, 0.0069,-0.1843),
    vec3(-0.0146, 0.1402, 0.0762), vec3( 0.0100,-0.1924,-0.0344),
    vec3(-0.3577,-0.5301,-0.4358), vec3(-0.3169, 0.1063, 0.0158),
    vec3( 0.0103,-0.5869, 0.0046), vec3(-0.0897,-0.4940, 0.3287),
    vec3( 0.7119,-0.0154,-0.0918), vec3(-0.0533, 0.0596,-0.5411),
    vec3( 0.0352,-0.0631, 0.5460), vec3(-0.4776, 0.2847,-0.0271)
    };

    float xr = fastnoise(in_uv * 1000.0, 1.0); // three primes
    float yr = fastnoise(in_uv * 1000.0, 2.0);
    float zr = fastnoise(in_uv * 1000.0, 3.0);
    vec3 random = normalize( vec3(xr, yr, zr) );

    float depth = sample_depth(in_uv);
    //out_ssao = depth;

    vec2 derivative = derivative_from_depth(depth, in_uv);
    //out_ssao = vec4(abs(derivative), 1.0 - depth, 1);

    float radius_depth = radius / depth;
    float occlusion = 0.0;
    for(int i=0; i < samples; i++) {
        vec3 ray = radius_depth * reflect(sample_sphere[i], random);
        //float dott = dot(ray,normal);
        vec2 off = ray.xy;
        //off = radius_depth * vec2( 0.0100,-0.1924);

        float occ_depth = sample_depth(clamp(in_uv + off, 0.0, 1.0));
        float difference = (depth - occ_depth + dot(off, derivative)) / clamp(length(derivative), 0.01, 1.0);

        occlusion += step(falloff, difference) * (1.0-smoothstep(falloff, area, difference));
    }

    float ao = 1.0 - total_strength * occlusion * (1.0 / samples);
    float v = clamp(ao + base, 0.0, 1.0);
    out_ssao = vec4(v, v, v, 1);
}