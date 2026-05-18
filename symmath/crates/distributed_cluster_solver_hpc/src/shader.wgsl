// Phase 5 HPC — GPU Batch Newton Kernel
//
// 每个 workgroup(1 线程) 处理一个 cluster 的完整 DAG:
//   κ → 1-κ → d_eff → t → residual → Δκ
//
// nodes layout: [cluster0_nodes..., cluster1_nodes..., ...]
// 每个 cluster 7 个节点: κ, 1.0, 1-κ, d, d_eff, c, t

struct Node {
    value: f32,
    kind: u32,
    input0: u32,
    input1: u32,
};

const NODES_PER_CLUSTER: u32 = 7u;
const C_LIGHT: f32 = 299792.458;
const TARGET: f32 = 59.88;

@group(0) @binding(0) var<storage, read_write> nodes: array<Node>;

@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let cluster = id.x;
    let base = cluster * NODES_PER_CLUSTER;

    let k  = base + 0u;
    let one = base + 1u;
    let sub = base + 2u;
    let d   = base + 3u;
    let de  = base + 4u;
    let c   = base + 5u;
    let t   = base + 6u;

    // 1-κ
    nodes[sub].value = nodes[one].value - nodes[k].value;
    // d_eff = d × (1-κ)
    nodes[de].value = nodes[d].value * nodes[sub].value;
    // t = d_eff / c
    nodes[t].value = nodes[de].value / nodes[c].value;

    // Newton: Δκ = -(t - TARGET) / (-d/c)
    let residual = nodes[t].value - TARGET;
    let jacobian = -nodes[d].value / nodes[c].value;
    nodes[k].value += -residual / jacobian;
}
