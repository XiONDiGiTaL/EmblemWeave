#version 450
// Test-scene fragment shader: one texture, explicit level of detail.
layout(set = 0, binding = 0) uniform sampler2D tex;
layout(push_constant) uniform PC {
    vec2 center;
    vec2 half_size;
    float angle;
    float aspect;
    float lod;
    float pad;
} pc;
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 color;
void main() {
    color = vec4(textureLod(tex, uv, pc.lod).rgb, 1.0);
}
