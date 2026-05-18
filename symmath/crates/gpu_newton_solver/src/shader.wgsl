// Phase 4 — GPU Newton Batch Solver
// 曲率跳跃：t = 40·AU·(1-κ) / c
//
// DAG (5 nodes):
//   0: κ,       Input
//   1: 1 − κ,   Sub  (特殊, 非标准 Mul)
//   2: d=40·AU,  Const
//   3: d_eff,    Mul = d × (1−κ)
//   4: t,        Div = d_eff / c
//
// 每 dispatch = 1 次 Newton 迭代:
//   Layer 1a: 计算 1−κ             (thread 1)
//   Layer 1b: 计算 d_eff           (thread 3)
//   Layer 1c: 计算 t               (thread 4)
//   Layer 2:  残差 f = t − 59.88   (thread 4)
//   Layer 3:  Jacobian ∂f/∂κ       (thread 0)
//   Layer 4:  Δκ = −f/J, κ += Δκ   (thread 0)

struct GpuNode {
    value: f32,
    kind: u32,     // 0=Input, 1=Const, 2=Mul, 3=Div
    input0: u32,
    input1: u32,
};

const TARGET: f32 = 59.88;
const C_LIGHT: f32 = 299792.458;

@group(0) @binding(0) var<storage, read_write> nodes: array<GpuNode>;
@group(0) @binding(1) var<storage, read_write> residual: array<f32>;
@group(0) @binding(2) var<storage, read_write> jacobian: array<f32>;
@group(0) @binding(3) var<storage, read_write> delta: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= 5u) { return; }

    // ── Layer 1a: 1 − κ ──
    if (i == 1u) {
        nodes[1].value = 1.0 - nodes[0].value;
    }

    storageBarrier();

    // ── Layer 1b: d_eff = d × (1−κ) ──
    if (i == 3u) {
        nodes[3].value = nodes[2].value * nodes[1].value;
    }

    storageBarrier();

    // ── Layer 1c: t + Layer 2: residual ──
    if (i == 4u) {
        nodes[4].value = nodes[3].value / C_LIGHT;
        residual[0] = nodes[4].value - TARGET;
    }

    storageBarrier();

    // ── Layer 3+4: Jacobian + Newton step ──
    if (i == 0u) {
        // ∂(t − t_target)/∂κ = −d / c  (const, 线性系统)
        jacobian[0] = -nodes[2].value / C_LIGHT;
        // Δκ = −f / J
        delta[0] = -residual[0] / jacobian[0];
        nodes[0].value += delta[0];
    }
}
