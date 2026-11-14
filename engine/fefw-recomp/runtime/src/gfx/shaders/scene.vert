#version 450
// Test-scene vertex shader: one quad from gl_VertexIndex (triangle strip), rotated and placed by push constants.
layout(push_constant) uniform PC {
    vec2 center;      // NDC
    vec2 half_size;   // in units of half the window height
    float angle;      // radians
    float aspect;     // window width / height
    float lod;        // texture level of detail the fragment shader samples
    float pad;
} pc;
layout(location = 0) out vec2 uv;
void main() {
    vec2 corner = vec2(float(gl_VertexIndex & 1) * 2.0 - 1.0, float(gl_VertexIndex >> 1) * 2.0 - 1.0);
    uv = corner * 0.5 + 0.5;
    float c = cos(pc.angle), s = sin(pc.angle);
    vec2 p = corner * pc.half_size;
    p = vec2(c * p.x - s * p.y, s * p.x + c * p.y);
    p.x /= pc.aspect;
    gl_Position = vec4(pc.center + p, 0.0, 1.0);
}
