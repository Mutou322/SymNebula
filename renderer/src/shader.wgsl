@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) energy: f32,
    @location(2) status: u32,
) -> VertexOutput {
    let scale = 1.0 / 200.0;
    var out: VertexOutput;
    out.clip_position = vec4(position * scale, 1.0);
    out.point_size = 1.5 + energy * 2.5;
    out.energy = energy;
    return out;
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @builtin(point_size) point_size: f32,
    @location(0) energy: f32,
};

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Soft warm white glow, brightness driven by energy
    let brightness = 0.15 + input.energy * 0.85;
    return vec4(vec3(brightness * 0.95, brightness * 0.92, brightness * 0.88), 1.0);
}
