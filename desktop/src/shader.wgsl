// SymNebula Desktop Shader — 点渲染 + 线渲染
// ============================================================

struct Camera {
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

// ── 节点 (PointList) ──────────────────────────────────────────

struct NodeInput {
    @location(0) position: vec3<f32>,
    @location(1) energy: f32,
    @location(2) status: u32,
    @location(3) highlighted: u32,
};

struct NodeOutput {
    @builtin(position) clip_position: vec4<f32>,
    @builtin(point_size) point_size: f32,
    @location(0) energy: f32,
    @location(1) status: u32,
    @location(2) highlighted: u32,
};

@vertex
fn vs_node(input: NodeInput) -> NodeOutput {
    var out: NodeOutput;
    out.clip_position = camera.proj * camera.view * vec4(input.position, 1.0);
    out.point_size = 2.0 + input.energy * 6.0;
    out.energy = input.energy;
    out.status = input.status;
    out.highlighted = input.highlighted;
    return out;
}

@fragment
fn fs_node(input: NodeOutput) -> @location(0) vec4<f32> {
    var color: vec3<f32>;
    // Green / Yellow / Purple
    switch input.status {
        case 0u: { color = vec3(0.1, 1.0, 0.6); }
        case 1u: { color = vec3(1.0, 0.8, 0.1); }
        default: { color = vec3(0.8, 0.2, 1.0); }
    }
    // Highlight blend
    if input.highlighted == 1u {
        color = mix(color, vec3(1.0, 1.0, 1.0), 0.4);
    }
    let brightness = 0.15 + input.energy * 0.85;
    return vec4(color * brightness, 1.0);
}

// ── 突触 (LineList) ──────────────────────────────────────────

struct SynapseInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct SynapseOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_synapse(input: SynapseInput) -> SynapseOutput {
    var out: SynapseOutput;
    out.clip_position = camera.proj * camera.view * vec4(input.position, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_synapse(input: SynapseOutput) -> @location(0) vec4<f32> {
    return vec4(input.color, 0.5);
}
