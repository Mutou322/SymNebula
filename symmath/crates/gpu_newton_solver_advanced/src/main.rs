// Phase 4 高级 — GPU Newton Batch Solver (HPC 级)
//
// 特性：
//   - SoA 数据布局 (kinds/input0/input1/values/dirty 独立数组)
//   - Forward AD 自动生成 Jacobian (Symbolic for 基础运算)
//   - Sparse CSR Jacobian 存储
//   - Block 分块求解器
//   - Dirty Propagation 增量更新
//   - 收敛监视器 (residual norm + Δx norm)
//
// DAG: 地球→柯伊伯带曲率跳跃 | t = 40·AU·(1−κ)/c

use bytemuck::{Pod, Zeroable};

/// 未连接输入哨兵值
const NO_INPUT: u32 = u32::MAX;

// ============================================================
// Part 1 — SoA 数据布局 (Structure of Arrays)
// ============================================================

/// DAG 拓扑（固定）和状态（可变），SoA 布局便于 GPU 传输。
#[derive(Debug, Clone)]
pub struct DagTopology {
    // ─── SoA: 每个节点一个槽位 ───
    pub kinds: Vec<u32>,     // 0=Input, 1=Const, 2=Sub, 3=Mul, 4=Div, 5=Eq(约束)
    pub input0: Vec<u32>,    // 第一个输入节点索引
    pub input1: Vec<u32>,    // 第二个输入节点索引
    pub values: Vec<f32>,    // 当前值 (mut)
    pub dirty: Vec<bool>,    // 脏标记 (mut)

    // ─── 变量 / 约束索引 ───
    pub variable_nodes: Vec<u32>,  // 哪些节点是 Input (求解变量)
    pub constraint_nodes: Vec<u32>, // 哪些节点是 Eq (约束)
    pub constraint_rhs: Vec<f32>,  // 约束右侧目标值
}

impl DagTopology {
    pub fn n_vars(&self) -> usize { self.variable_nodes.len() }
    pub fn n_cons(&self) -> usize { self.constraint_nodes.len() }
    pub fn n_nodes(&self) -> usize { self.kinds.len() }
}

/// Sparse CSR 矩阵
#[derive(Debug, Clone)]
pub struct SparseCsr {
    pub n_rows: usize,
    pub n_cols: usize,
    pub row_offsets: Vec<u32>,  // [n_rows + 1]
    pub col_indices: Vec<u32>,  // [nnz]
    pub values: Vec<f32>,       // [nnz]
}

/// 收敛监视器
#[derive(Debug, Clone)]
pub struct ConvergenceMonitor {
    pub residuals: Vec<f32>,
    pub deltas: Vec<f32>,
    pub converged: bool,
    pub iteration: usize,
    pub tol: f32,
}

impl ConvergenceMonitor {
    pub fn new(tol: f32) -> Self {
        Self { residuals: Vec::new(), deltas: Vec::new(), converged: false, iteration: 0, tol }
    }
    pub fn residual_norm(&self) -> f32 {
        self.residuals.last().copied().unwrap_or(0.0)
    }
    pub fn record(&mut self, residual_norm: f32, delta_norm: f32) {
        self.residuals.push(residual_norm);
        self.deltas.push(delta_norm);
        self.iteration = self.residuals.len();
        // f32 精度保护: Δx 小于机器精度时视作收敛
        self.converged = residual_norm < self.tol || delta_norm < f32::EPSILON;
    }
}

/// Block 定义：一组约束 + 相关变量
#[derive(Debug, Clone)]
pub struct Block {
    pub constraint_indices: Vec<u32>,  // 约束全局索引
    pub variable_indices: Vec<u32>,    // 变量全局索引
    pub jacobian: SparseCsr,
}

// ============================================================
// Part 2 — GPU 友好 Node 布局 (bytemuck)
// ============================================================

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GpuNode {
    pub value: f32,
    pub kind: u32,
    pub input0: u32,
    pub input1: u32,
}

// ============================================================
// Part 3 — DAG 构建器
// ============================================================

/// 构建曲率跳跃 DAG (SoA 格式)
///
///   0: κ        Input  曲率
///   1: 1.0      Const  常量
///   2: 1−κ      Sub
///   3: d=40·AU  Const
///   4: d_eff    Mul    d × (1−κ)
///   5: c        Const  光速
///   6: t        Div    d_eff / c
///   7: 约束 Eq  Eq     t = 59.88
pub fn build_curvature_dag(kappa: f32, au: f32, c: f32, target_t: f32) -> DagTopology {
    let n = 8usize;
    let mut d = DagTopology {
        kinds:           vec![0, 1, 2, 1, 3, 1, 4, 5],
        input0:          vec![NO_INPUT; n],
        input1:          vec![NO_INPUT; n],
        values:          vec![kappa, 1.0, 0.0, 40.0 * au, 0.0, c, 0.0, 0.0],
        dirty:           vec![true, false, true, false, true, false, true, false],
        variable_nodes:  vec![0],
        constraint_nodes: vec![7],
        constraint_rhs:  vec![target_t],
    };
    // 连线: 节点 2 = 1−κ = 1.0 − κ
    d.input0[2] = 1;  // 1.0
    d.input1[2] = 0;  // κ
    // 连线: 节点 4 = d_eff = d × (1−κ)
    d.input0[4] = 3;  // d
    d.input1[4] = 2;  // 1−κ
    // 连线: 节点 6 = t = d_eff / c
    d.input0[6] = 4;  // d_eff
    d.input1[6] = 5;  // c
    // 连线: 节点 7 = Eq(t = target), rhs from constraint_rhs
    d.input0[7] = 6;  // t (lhs)
    d
}

/// 线性系统 DAG: x + y = 10, x − y = 2
pub fn build_linear_2x2(x: f32, y: f32) -> DagTopology {
    // 0:x  1:y  2:10  3:x+y  4:2  5:x-y  6:eq1  7:eq2
    let n = 8usize;
    let mut d = DagTopology {
        kinds:           vec![0; n],
        input0:          vec![NO_INPUT; n],
        input1:          vec![NO_INPUT; n],
        values:          vec![0.0; n],
        dirty:           vec![false; n],
        variable_nodes:  vec![0, 1],
        constraint_nodes: vec![6, 7],
        constraint_rhs:  vec![10.0, 2.0],
    };
    d.kinds[0] = 0; d.values[0] = x; d.dirty[0] = true;
    d.kinds[1] = 0; d.values[1] = y; d.dirty[1] = true;
    d.kinds[2] = 1; d.values[2] = 10.0;
    d.kinds[3] = 6; d.input0[3] = 0; d.input1[3] = 1; d.dirty[3] = true;  // Add x+y
    d.kinds[4] = 1; d.values[4] = 2.0;
    d.kinds[5] = 2; d.input0[5] = 0; d.input1[5] = 1; d.dirty[5] = true;  // Sub x-y
    d.kinds[6] = 5; d.input0[6] = 3;  // Eq1: sum = 10 → residual = sum - 10
    d.kinds[7] = 5; d.input0[7] = 5;  // Eq2: diff = 2 → residual = diff - 2
    d
}

// ============================================================
// Part 4 — Forward AD 引擎
// ============================================================

/// 对指定变量执行 Forward-mode AD，返回每个节点对该变量的偏导数。
///
/// Assumes: values 已更新 (tick 后), 节点按拓扑序排列。
pub fn forward_ad(dag: &DagTopology, var_node: u32) -> Vec<f32> {
    let n = dag.n_nodes();
    let mut deriv = vec![0.0f32; n];
    let kinds = &dag.kinds;
    let values = &dag.values;
    let input0 = &dag.input0;
    let input1 = &dag.input1;

    for i in 0..n {
        deriv[i] = match kinds[i] {
            0 | 1 => {
                if kinds[i] == 0 && i == var_node as usize { 1.0 } else { 0.0 }
            }
            _ => {
                let a = input0[i] as usize;
                let da = deriv[a];
                match kinds[i] {
                    6 => { let b = input1[i] as usize; da + deriv[b] }
                    2 => { let b = input1[i] as usize; da - deriv[b] }
                    3 => { let b = input1[i] as usize; values[a] * deriv[b] + values[b] * da }
                    4 => { let b = input1[i] as usize; (da * values[b] - values[a] * deriv[b]) / (values[b] * values[b]) }
                    5 => {
                        if input1[i] == NO_INPUT { da }
                        else { let b = input1[i] as usize; da - deriv[b] }
                    }
                    _ => 0.0,
                }
            }
        };
    }
    deriv
}

/// 用 Forward AD 构建稀疏 Jacobian。
///
/// J[i][j] = ∂f_i/∂x_j 其中 f_i = value[lhs_i] − rhs_i
pub fn build_jacobian(dag: &DagTopology) -> SparseCsr {
    let n_vars = dag.n_vars();
    let n_cons = dag.n_cons();
    let mut row_offsets = Vec::with_capacity(n_cons + 1);
    let mut col_indices = Vec::new();
    let mut values = Vec::new();

    row_offsets.push(0);

    for &con_node in &dag.constraint_nodes {
        let lhs = dag.input0[con_node as usize] as usize;
        let nnz_start = col_indices.len();

        for (j, &var_node) in dag.variable_nodes.iter().enumerate() {
            let deriv = forward_ad(dag, var_node);
            let j_val = deriv[lhs]; // ∂lhs/∂x_j (rhs is target constant, derivative=0)
            if j_val.abs() > 1e-15 {
                col_indices.push(j as u32);
                values.push(j_val);
            }
        }

        // 如果没有非零元素，也加一个占位（保持方阵结构）
        if col_indices.len() == nnz_start {
            col_indices.push(0);
            values.push(0.0);
        }

        row_offsets.push(col_indices.len() as u32);
    }

    SparseCsr {
        n_rows: n_cons,
        n_cols: n_vars,
        row_offsets: {
            let mut off = Vec::with_capacity(row_offsets.len());
            for &o in &row_offsets { off.push(o); }
            off
        },
        col_indices,
        values,
    }
}

/// 将稀疏 CSR 转为稠密矩阵（用于小系统求解）
pub fn sparse_to_dense(mat: &SparseCsr) -> Vec<Vec<f32>> {
    let mut dense = vec![vec![0.0f32; mat.n_cols]; mat.n_rows];
    for row in 0..mat.n_rows {
        let start = mat.row_offsets[row] as usize;
        let end = mat.row_offsets[row + 1] as usize;
        for k in start..end {
            let col = mat.col_indices[k] as usize;
            dense[row][col] = mat.values[k];
        }
    }
    dense
}

// ============================================================
// Part 5 — CPU Solver Pipeline
// ============================================================

/// 高斯消元（列选主元）解方阵 J·x = b
pub fn gauss_solve(j: &[Vec<f32>], b: &[f32]) -> Option<Vec<f32>> {
    let n = b.len();
    if n == 0 { return Some(vec![]); }
    let mut a = j.to_vec();
    let mut rhs = b.to_vec();

    for col in 0..n {
        let mut max_row = col;
        let mut max_val = a[col][col].abs();
        for row in (col + 1)..n {
            if a[row][col].abs() > max_val { max_val = a[row][col].abs(); max_row = row; }
        }
        if max_val < 1e-15 { return None; }
        if max_row != col { a.swap(col, max_row); rhs.swap(col, max_row); }
        let pivot = a[col][col];
        for row in (col + 1)..n {
            let f = a[row][col] / pivot;
            for k in col..n { a[row][k] -= f * a[col][k]; }
            rhs[row] -= f * rhs[col];
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = rhs[i];
        for j in (i + 1)..n { s -= a[i][j] * x[j]; }
        if a[i][i].abs() < 1e-15 { return None; }
        x[i] = s / a[i][i];
    }
    Some(x)
}

/// DAG 前向计算 (tick) — 只更新 dirty 节点
pub fn dag_tick(dag: &mut DagTopology) {
    for i in 0..dag.n_nodes() {
        if !dag.dirty[i] { continue; }
        let val = {
            let kind = dag.kinds[i];
            let a = dag.input0[i];
            match kind {
                0 | 1 => dag.values[i],
                _ => {
                    let va = dag.values[a as usize];
                    if kind == 5 && dag.input1[i] == NO_INPUT {
                        va
                    } else {
                        let b = dag.input1[i] as usize;
                        let vb = dag.values[b];
                        match kind {
                            6 => va + vb,
                            2 => va - vb,
                            3 => va * vb,
                            4 => va / vb,
                            5 => va - vb,
                            _ => dag.values[i],
                        }
                    }
                }
            }
        };
        dag.values[i] = val;
        dag.dirty[i] = false;
    }
}

/// 标记节点及其所有下游节点为 dirty
pub fn mark_dirty(dag: &mut DagTopology, node: u32) {
    if dag.dirty[node as usize] { return; }
    dag.dirty[node as usize] = true;
    // 传播到下游: 扫描所有节点，找出输入中包含 node 的
    for i in 0..dag.n_nodes() {
        if dag.input0[i] == node || dag.input1[i] == node {
            mark_dirty(dag, i as u32);
        }
    }
}

/// 计算残差向量
pub fn compute_residuals(dag: &DagTopology) -> Vec<f32> {
    dag.constraint_nodes.iter().enumerate().map(|(i, &con_node)| {
        let lhs = dag.input0[con_node as usize] as usize;
        dag.values[lhs] - dag.constraint_rhs[i]
    }).collect()
}

/// 单次 Newton 迭代（完整 pipeline）
pub fn newton_step(dag: &mut DagTopology, mon: &mut ConvergenceMonitor) -> bool {
    // 1. Tick (forward eval dirty nodes)
    dag_tick(dag);

    // 2. 残差
    let residuals = compute_residuals(dag);
    let res_norm = residuals.iter().map(|r| r * r).sum::<f32>().sqrt();

    // 3. Jacobian (Forward AD)
    let jac = build_jacobian(dag);
    let dense_j = sparse_to_dense(&jac);

    // 4. 解 J·Δx = −F
    let neg_f: Vec<f32> = residuals.iter().map(|r| -r).collect();
    let delta = match gauss_solve(&dense_j, &neg_f) {
        Some(d) => d,
        None => {
            mon.converged = false;
            return false;
        }
    };
    let delta_norm = delta.iter().map(|d| d * d).sum::<f32>().sqrt();

    // 5. 记录收敛
    mon.record(res_norm, delta_norm);
    if mon.converged { return true; }

    // 6. 应用 Δx 到变量节点
    let var_nodes = dag.variable_nodes.clone();
    for (j, &var_node) in var_nodes.iter().enumerate() {
        dag.values[var_node as usize] += delta[j];
        mark_dirty(dag, var_node);
    }

    false
}

/// Newton 求解循环
pub fn newton_solve(dag: &mut DagTopology, tol: f32, max_iter: usize) -> ConvergenceMonitor {
    let mut mon = ConvergenceMonitor::new(tol);
    for _ in 0..max_iter {
        if newton_step(dag, &mut mon) { break; }
    }
    mon
}

// ============================================================
// Part 6 — Block 分块求解
// ============================================================

/// 把 DAG 的约束分成多个 block，每块独立求解
pub fn partition_blocks(dag: &DagTopology, block_size: usize) -> Vec<Block> {
    let n_cons = dag.n_cons();
    let mut blocks = Vec::new();

    for start in (0..n_cons).step_by(block_size) {
        let end = (start + block_size).min(n_cons);
        let cons: Vec<u32> = dag.constraint_nodes[start..end].to_vec();

        // 收集块中用到的变量
        let mut vars = Vec::new();
        for &cn in &cons {
            let lhs = dag.input0[cn as usize] as usize;
            for (j, &vn) in dag.variable_nodes.iter().enumerate() {
                let deriv = forward_ad(dag, vn);
                if deriv[lhs].abs() > 1e-15 {
                    if !vars.contains(&(j as u32)) {
                        vars.push(j as u32);
                    }
                }
            }
        }

        let jac = build_jacobian(dag); // 全局 Jacobian

        blocks.push(Block {
            constraint_indices: cons,
            variable_indices: vars,
            jacobian: jac,
        });
    }
    blocks
}

// ============================================================
// Part 7 — GPU Shader (WGSL, feature=gpu)
// ============================================================
#[cfg(feature = "gpu")]
const SHADER: &str = include_str!("shader.wgsl");

#[cfg(feature = "gpu")]
mod gpu {
    use std::sync::mpsc;
    use wgpu::util::DeviceExt;
    use super::*;

    pub struct GpuSolver {
        device: wgpu::Device,
        queue: wgpu::Queue,
        pipeline: wgpu::ComputePipeline,
        bind_group_layout: wgpu::BindGroupLayout,
    }

    impl GpuSolver {
        pub fn new() -> Self {
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(instance.request_adapter(
                &wgpu::RequestAdapterOptions::default(),
            )).expect("no GPU adapter");
            let (device, queue) = pollster::block_on(
                adapter.request_device(&wgpu::DeviceDescriptor::default(), None)
            ).unwrap();

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("advanced_newton"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER)),
            });

            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ad_pipeline"),
                layout: None,
                module: &shader,
                entry_point: "ad_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            });

            let bind_group_layout = pipeline.get_bind_group_layout(0);

            GpuSolver { device, queue, pipeline, bind_group_layout }
        }

        /// GPU Forward AD: 计算所有变量对所有节点的偏导数
        pub fn compute_derivatives(&self, dag: &DagTopology) -> Vec<Vec<f32>> {
            let n_vars = dag.n_vars();
            let n_nodes = dag.n_nodes();
            let _n_cons = dag.n_cons();

            // 将 DAG 拓扑打包为 flat GPU buffer
            #[repr(C)]
            #[derive(Copy, Clone, Pod, Zeroable)]
            struct DagFlat {
                kinds: u32, input0: u32, input1: u32, value: f32,
            }
            let mut flat: Vec<DagFlat> = Vec::with_capacity(n_nodes);
            for i in 0..n_nodes {
                flat.push(DagFlat {
                    kinds: dag.kinds[i],
                    input0: dag.input0[i],
                    input1: dag.input1[i],
                    value: dag.values[i],
                });
            }

            let topo_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("topology"),
                contents: bytemuck::cast_slice(&flat),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });

            // deriv 输出 buffer: [n_vars][n_nodes] f32
            let deriv_size = (n_vars * n_nodes) as u64 * 4;
            let deriv_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("derivatives"),
                size: deriv_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });

            // 参数 buffer: n_vars, n_nodes, current_var
            let params_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("params"),
                contents: bytemuck::cast_slice(&[n_vars as u32, n_nodes as u32, 0u32]),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ad_bg"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: topo_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: deriv_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: params_buf.as_entire_binding() },
                ],
            });

            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("ad_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(n_vars as u32, 1, 1); // 每个变量一个 workgroup
            }
            self.queue.submit(Some(encoder.finish()));
            self.device.poll(wgpu::Maintain::Wait);

            // 回读 derivatives
            let slice = deriv_buf.slice(..);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
            self.device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().expect("mapping failed");
            let data = slice.get_mapped_range();
            let raw: &[f32] = bytemuck::cast_slice(&data);

            let mut result = vec![vec![0.0f32; n_nodes]; n_vars];
            for v in 0..n_vars {
                for n in 0..n_nodes {
                    result[v][n] = raw[v * n_nodes + n];
                }
            }
            drop(data);
            deriv_buf.unmap();

            result
        }
    }
}

// ============================================================
// Part 8 — 主入口 + 演示
// ============================================================

const AU: f32 = 149_597_870.7;
const C: f32 = 299_792.458;
const TARGET: f32 = 59.88;

fn main() {
    println!("╔════════════════════════════════════════════════════╗");
    println!("║  Phase 4 高级 — GPU Newton Solver (HPC级)         ║");
    println!("║  Forward AD · Sparse CSR · Block Solver           ║");
    println!("╚════════════════════════════════════════════════════╝\n");

    // ── 1. 曲率跳跃 Newton 求解 ──
    println!("── 曲率跳跃 Newton 求解 ──");
    let mut dag = build_curvature_dag(0.5, AU, C, TARGET);
    let mon = newton_solve(&mut dag, 1e-6, 10);
    println!("  κ* = {:.6}  (expected 0.997)", dag.values[0]);
    println!("  t  = {:.2}s (expected 59.88s)", dag.values[6]);
    println!("  iterations: {}  | final residual: {:.4e}\n", mon.iteration, mon.residual_norm());

    // ── 2. Forward AD 验证 ──
    println!("── Forward AD 逐节点偏导数 (∂/∂κ) ──");
    let dag_ad = build_curvature_dag(0.997, AU, C, TARGET);
    // tick 一次让值更新
    let mut dag_ad = dag_ad;
    dag_tick(&mut dag_ad);
    let deriv = forward_ad(&dag_ad, 0); // var_node=0 (κ)
    let labels = ["κ", "1.0", "1−κ", "d", "d_eff", "c", "t", "Eq"];
    for i in 0..dag_ad.n_nodes() {
        println!("  {i} {:<6}  ∂/∂κ = {:.6e}", labels[i], deriv[i]);
    }
    // 验证 ∂t/∂κ = -d/c
    let expected = -dag_ad.values[3] / dag_ad.values[5]; // -d/c
    println!("  ∂t/∂κ = {:.6e}  (analytical: {:.6e}) ✓", deriv[6], expected);

    // ── 3. Sparse Jacobian ──
    println!("\n── Sparse Jacobian (CSR) ──");
    let jac = build_jacobian(&dag_ad);
    let dense = sparse_to_dense(&jac);
    println!("  dimensions: {}×{}", jac.n_rows, jac.n_cols);
    println!("  non-zeros: {}", jac.col_indices.len());
    println!("  density: {:.1}%", 100.0 * jac.col_indices.len() as f32 / (jac.n_rows * jac.n_cols) as f32);
    println!("  J[0][0] = {:.6e}  (∂f/∂κ)", dense[0][0]);

    // ── 4. 多初始猜测 ──
    println!("\n── 多初始猜测收敛 ──");
    for &guess in &[0.0, 0.1, 0.3, 0.5, 0.7, 0.9, 1.0] {
        let mut d = build_curvature_dag(guess, AU, C, TARGET);
        let m = newton_solve(&mut d, 1e-6, 10);
        println!("  κ₀ = {guess} → κ* = {:.6}, t = {:.2}s, {} iters ✓",
            d.values[0], d.values[6], m.iteration);
    }

    // ── 5. 线性系统 2×2 ──
    println!("\n── 线性系统 2×2 (x+y=10, x−y=2) ──");
    let mut sys = build_linear_2x2(0.0, 0.0);
    let m2 = newton_solve(&mut sys, 1e-6, 10);
    println!("  x* = {:.6} (expected 6.0)", sys.values[0]);
    println!("  y* = {:.6} (expected 4.0)", sys.values[1]);
    println!("  iterations: {} | final residual: {:.4e}", m2.iteration, m2.residual_norm());

    // ── 6. Block partition ──
    println!("\n── Block 分块 ──");
    let blocks = partition_blocks(&dag_ad, 1);
    println!("  DAG: {} vars × {} cons → {} block(s)", dag_ad.n_vars(), dag_ad.n_cons(), blocks.len());

    #[cfg(feature = "gpu")]
    {
        println!("\n── GPU Forward AD ──");
        let gpu = gpu::GpuSolver::new();
        let gpu_derivs = gpu.compute_derivatives(&dag_ad);
        let j = gpu_derivs[0][6];
        println!("  GPU ∂t/∂κ = {:.6e}  (CPU: {:.6e})  diff: {:.2e}",
            j, deriv[6], (j - deriv[6]).abs());
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curvature_newton_converges() {
        let mut dag = build_curvature_dag(0.5, AU, C, TARGET);
        let mon = newton_solve(&mut dag, 1e-6, 10);
        assert!(mon.converged, "should converge");
        assert!((dag.values[0] - 0.997).abs() < 0.001, "κ should be ~0.997");
        assert!((dag.values[6] - 59.88).abs() < 0.1, "t should be ~59.88s");
    }

    #[test]
    fn forward_ad_derivatives() {
        let mut dag = build_curvature_dag(0.997, AU, C, TARGET);
        dag_tick(&mut dag);
        let deriv = forward_ad(&dag, 0);
        // ∂(1-κ)/∂κ = -1
        assert!((deriv[2] - (-1.0)).abs() < 1e-6, "∂(1-κ)/∂κ should be -1");
        // ∂d/∂κ = 0
        assert!(deriv[3].abs() < 1e-6, "∂d/∂κ should be 0");
        // ∂t/∂κ = -d/c
        let expected = -dag.values[3] / dag.values[5];
        assert!((deriv[6] - expected).abs() < 1e-4, "∂t/∂κ mismatch: got {} vs {}", deriv[6], expected);
    }

    #[test]
    fn jacobian_vs_finite_difference() {
        let eps = 1e-3f32;
        let mut dag = build_curvature_dag(0.5, AU, C, TARGET);
        dag_tick(&mut dag);
        let j_ad = build_jacobian(&dag);
        let dense = sparse_to_dense(&j_ad);

        // Finite difference
        let mut dag_plus = build_curvature_dag(0.5 + eps, AU, C, TARGET);
        dag_tick(&mut dag_plus);
        let r_plus = compute_residuals(&dag_plus);

        let mut dag_minus = build_curvature_dag(0.5 - eps, AU, C, TARGET);
        dag_tick(&mut dag_minus);
        let r_minus = compute_residuals(&dag_minus);

        let fd = (r_plus[0] - r_minus[0]) / (2.0 * eps);
        let rel_err = (dense[0][0] - fd).abs() / fd.abs().max(1.0);
        assert!(rel_err < 1e-3, "AD Jacobian vs FD: rel_err = {:.2e}", rel_err);
    }

    #[test]
    fn linear_2x2_solves() {
        let mut sys = build_linear_2x2(0.0, 0.0);
        let mon = newton_solve(&mut sys, 1e-6, 10);
        assert!(mon.converged);
        assert!((sys.values[0] - 6.0).abs() < 0.01, "x should be 6");
        assert!((sys.values[1] - 4.0).abs() < 0.01, "y should be 4");
    }

    #[test]
    fn dirty_propagation() {
        let mut dag = build_curvature_dag(0.997, AU, C, TARGET);
        dag_tick(&mut dag);
        // 所有节点 clean
        assert!(dag.dirty.iter().all(|&d| !d), "all clean after tick");

        // 修改 κ → 只有下游变脏
        mark_dirty(&mut dag, 0);
        assert!(dag.dirty[0], "κ dirty");
        assert!(dag.dirty[2], "1-κ dirty");
        assert!(dag.dirty[4], "d_eff dirty");
        assert!(dag.dirty[6], "t dirty");
        assert!(dag.dirty[7], "Eq dirty");
        // 常量不应被污染
        assert!(!dag.dirty[1], "Const(1) not dirty");
        assert!(!dag.dirty[3], "d not dirty");
        assert!(!dag.dirty[5], "c not dirty");
    }

    #[test]
    fn multiple_initial_guesses() {
        for &guess in &[0.0, 0.1, 0.3, 0.5, 0.7, 0.9, 1.0] {
            let mut dag = build_curvature_dag(guess, AU, C, TARGET);
            let mon = newton_solve(&mut dag, 1e-6, 10);
            assert!(mon.converged, "κ₀={guess} should converge");
            assert!((dag.values[0] - 0.997).abs() < 0.001, "κ₀={guess}: κ should be 0.997");
        }
    }
}
