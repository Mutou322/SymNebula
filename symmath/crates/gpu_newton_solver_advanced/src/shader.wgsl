// Phase 4 高级 — GPU Newton Solver Shaders
//
// Kernel 1 (ad_main): Forward-mode AD
//   - 每个 workgroup 处理一个变量
//   - workgroup_size = 64, dispatch = n_vars × 1 × 1
//   - 每个线程处理一个节点的导数: O(n_nodes) 串行 for 循环
//
// Kernel 2 (residual_main): Residual + Jacobian CSV
//   - 每个线程处理一个约束
//   - 从 AD 输出读取 ∂lhs/∂x_j → 写入 sparse Jacobian
//
// Kernel 3 (apply_main): Δx 应用 + dirty propagation
//   - 更新变量值, 广播脏标记

// ──────────────────────────────────────────────────
// 共享结构
// ──────────────────────────────────────────────────

struct NodeInfo {
    kind: u32,     // 0=Input,1=Const,2=Sub,3=Mul,4=Div,5=Eq,6=Add
    input0: u32,
    input1: u32,
    value: f32,
};

// ──────────────────────────────────────────────────
// Kernel 1: Forward-mode AD
// ──────────────────────────────────────────────────

struct AdParams {
    n_vars: u32,     // 变量数 (dispatch 维度)
    n_nodes: u32,    // DAG 节点数
    var_idx: u32,    // 当前变量索引 (由 dispatch 隐式传递)
};

@group(0) @binding(0) var<storage, read> topology: array<NodeInfo>;
@group(0) @binding(1) var<storage, read_write> derivatives: array<f32>;
@group(0) @binding(2) var<storage, read_write> params: AdParams;

@compute @workgroup_size(1)
fn ad_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let var_idx = id.x;            // 当前变量
    let n = i32(params.n_nodes);
    let var_node = 0u;             // 简化: 第一个 Input 节点是变量
                                   // 真实实现需从 variable_nodes[] 读取

    // Workgroup 内串行: 每个线程遍历所有节点 O(n)
    for (var i = 0i32; i < n; i++) {
        let node = i32(i);
        let kind = topology[node].kind;
        let a = i32(topology[node].input0);
        let b = i32(topology[node].input1);
        let da = derivatives[var_idx * u32(n) + u32(a)];
        let db = derivatives[var_idx * u32(n) + u32(b)];

        var d: f32 = 0.0;

        if (kind == 0u) {
            // Input: 是当前变量则 seed 1.0
            if (node == 0) { d = 1.0; }  // var_node 在简化 DAG 中索引 0
        } else if (kind == 1u) {
            d = 0.0;  // Const
        } else if (kind == 6u) {
            d = da + db;     // Add
        } else if (kind == 2u) {
            d = da - db;     // Sub
        } else if (kind == 3u) {
            // Mul: a·db + b·da
            let va = topology[a].value;
            let vb = topology[b].value;
            d = va * db + vb * da;
        } else if (kind == 4u) {
            // Div: (da·b − a·db) / b²
            let va = topology[a].value;
            let vb = topology[b].value;
            d = (da * vb - va * db) / (vb * vb);
        } else if (kind == 5u) {
            d = da - db;     // Eq
        }

        derivatives[var_idx * u32(n) + u32(i)] = d;
    }
}

// ──────────────────────────────────────────────────
// Kernel 2: Residual + Jacobian CSV 构造
// ──────────────────────────────────────────────────
// (示意: 实际 CSR 构建需 CPU 端做 prefix sum / atomic)
//
// 每个线程处理一个约束:
//   residual = value[lhs] - target
//   J[i][j] = ∂lhs/∂x_j (从 derivatives 读取)

struct ResParams {
    n_cons: u32,
    con_lhs_nodes: array<u32>,        // Flat array 的绑定需 WGSL 动态数组
    con_targets: array<f32>,
};

// ──────────────────────────────────────────────────
// Kernel 3: Apply Δx + dirty propagation
// ──────────────────────────────────────────────────
// (示意: 直接更新变量值)

// 三个 kernel 可用同一个 pipeline, 用 entry point 区分。
// WGSL 允许一个文件内多个 @compute entry points。
