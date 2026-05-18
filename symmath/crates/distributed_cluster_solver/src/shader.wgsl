// Phase 5 — Distributed Cluster Solver GPU Kernel
//
// 每个 workgroup(1 线程) 处理一个 cluster 的整个 DAG:
//   0: κ (Input)       → Newton 变量
//   1: 1.0 (Const)     → 常量
//   2: 1−κ (Sub)       = 1.0 − κ
//   3: d (Const)       = 40·AU
//   4: d_eff (Mul)     = d × (1−κ)
//   5: c (Const)       = 光速 299792.458
//   6: t (Div)         = d_eff / c
//
// 每个 thread 顺序执行，无需 barrier（单线程内天然有序）。

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

    // 索引
    let k  = base + 0u;   // κ
    let one = base + 1u;  // 1.0
    let sub = base + 2u;  // 1−κ
    let d   = base + 3u;  // 40·AU
    let de  = base + 4u;  // d_eff
    let c   = base + 5u;  // 光速
    let t   = base + 6u;  // t

    // 1) 1−κ = 1.0 − κ
    nodes[sub].value = nodes[one].value - nodes[k].value;

    // 2) d_eff = d × (1−κ)
    nodes[de].value = nodes[d].value * nodes[sub].value;

    // 3) t = d_eff / c
    nodes[t].value = nodes[de].value / nodes[c].value;

    // 4) Newton step: t − TARGET = 0
    //    residual = t − TARGET, Jacobian = ∂t/∂κ = −d/c
    //    Δκ = −residual / J = (t − TARGET) / (−d/c)
    let residual = nodes[t].value - TARGET;
    let jacobian = -nodes[d].value / nodes[c].value;
    let delta = -residual / jacobian;
    nodes[k].value += delta;
}
