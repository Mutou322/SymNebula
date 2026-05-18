//! Phase 5 HPC — 分布式 Cluster 求解器共享类型与工具
//!
//! 所有模块公用的类型定义: Node, SparseBlock, ConvergenceMonitor 等。

pub mod jacobian;
pub mod convergence;
pub mod communicator;
pub mod cluster;
pub mod scheduler;

use bytemuck::{Pod, Zeroable};

// ─── 常量 ──────────────────────────────────────────────────

pub const AU: f32 = 149_597_870.7;
pub const C_LIGHT: f32 = 299_792.458;
pub const TARGET: f32 = 59.88;
pub const NO_INPUT: u32 = u32::MAX;

/// 节点种类
pub const KIND_INPUT: u32 = 0;
pub const KIND_CONST: u32 = 1;
pub const KIND_SUB: u32 = 2;
pub const KIND_MUL: u32 = 3;
pub const KIND_DIV: u32 = 4;
pub const KIND_EQ: u32 = 5;
pub const KIND_ADD: u32 = 6;

// ─── GPU Node ──────────────────────────────────────────────

/// GPU 友好的节点 SoA 布局
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Node {
    pub value: f32,
    pub kind: u32,
    pub input0: u32,
    pub input1: u32,
}

impl Node {
    pub fn input(k: f32) -> Self {
        Node { value: k, kind: KIND_INPUT, input0: NO_INPUT, input1: NO_INPUT }
    }
    pub fn constant(v: f32) -> Self {
        Node { value: v, kind: KIND_CONST, input0: NO_INPUT, input1: NO_INPUT }
    }
    pub fn sub(in0: u32, in1: u32) -> Self {
        Node { value: 0.0, kind: KIND_SUB, input0: in0, input1: in1 }
    }
    pub fn mul(in0: u32, in1: u32) -> Self {
        Node { value: 0.0, kind: KIND_MUL, input0: in0, input1: in1 }
    }
    pub fn div(in0: u32, in1: u32) -> Self {
        Node { value: 0.0, kind: KIND_DIV, input0: in0, input1: in1 }
    }
    pub fn eq(lhs: u32) -> Self {
        Node { value: 0.0, kind: KIND_EQ, input0: lhs, input1: NO_INPUT }
    }
    pub fn add(in0: u32, in1: u32) -> Self {
        Node { value: 0.0, kind: KIND_ADD, input0: in0, input1: in1 }
    }
}

// ─── SoA DAG ───────────────────────────────────────────────

/// DAG 拓扑 + 状态 (SoA 布局)
#[derive(Debug, Clone)]
pub struct DagTopology {
    pub kinds: Vec<u32>,
    pub input0: Vec<u32>,
    pub input1: Vec<u32>,
    pub values: Vec<f32>,
    pub dirty: Vec<bool>,
    pub variable_nodes: Vec<u32>,
    pub constraint_nodes: Vec<u32>,
    pub constraint_rhs: Vec<f32>,
}

impl DagTopology {
    pub fn n_vars(&self) -> usize { self.variable_nodes.len() }
    pub fn n_cons(&self) -> usize { self.constraint_nodes.len() }
    pub fn n_nodes(&self) -> usize { self.kinds.len() }
}

/// 构建曲率跳跃 DAG (7 节点)
///   0: κ(Input)  1: 1.0(Const)  2: 1-κ(Sub)  3: d(Const)
///   4: d_eff(Mul)  5: c(Const)  6: t(Div)
pub fn build_curvature_dag(kappa: f32) -> DagTopology {
    let d = 40.0 * AU;
    let mut dg = DagTopology {
        kinds:          vec![KIND_INPUT, KIND_CONST, KIND_SUB, KIND_CONST, KIND_MUL, KIND_CONST, KIND_DIV],
        input0:         vec![NO_INPUT; 7],
        input1:         vec![NO_INPUT; 7],
        values:         vec![kappa, 1.0, 0.0, d, 0.0, C_LIGHT, 0.0],
        dirty:          vec![true, false, true, false, true, false, true],
        variable_nodes: vec![0],
        constraint_nodes: vec![6],
        constraint_rhs: vec![TARGET],
    };
    dg.input0[2] = 1; dg.input1[2] = 0;  // 1-κ
    dg.input0[4] = 3; dg.input1[4] = 2;  // d × (1-κ)
    dg.input0[6] = 4; dg.input1[6] = 5;  // d_eff / c
    dg
}

/// 将 DAG 转换为平坦 Node 数组 (GPU 上传用)
pub fn dag_to_nodes(dag: &DagTopology) -> Vec<Node> {
    (0..dag.n_nodes()).map(|i| Node {
        value: dag.values[i],
        kind: dag.kinds[i],
        input0: dag.input0[i],
        input1: dag.input1[i],
    }).collect()
}

// ─── Sparse Block Jacobian ─────────────────────────────────

/// Block-CSR 稀疏矩阵
#[derive(Debug, Clone)]
pub struct SparseBlock {
    pub n_rows: usize,
    pub n_cols: usize,
    pub row_offsets: Vec<u32>,
    pub col_indices: Vec<u32>,
    pub values: Vec<f32>,
}

impl SparseBlock {
    pub fn new(n_rows: usize, n_cols: usize) -> Self {
        Self { n_rows, n_cols, row_offsets: vec![0; n_rows + 1], col_indices: Vec::new(), values: Vec::new() }
    }
    pub fn nnz(&self) -> usize { self.values.len() }
    pub fn to_dense(&self) -> Vec<Vec<f32>> {
        let mut d = vec![vec![0.0f32; self.n_cols]; self.n_rows];
        for r in 0..self.n_rows {
            let s = self.row_offsets[r] as usize;
            let e = self.row_offsets[r + 1] as usize;
            for k in s..e { d[r][self.col_indices[k] as usize] = self.values[k]; }
        }
        d
    }
}

// ─── 收敛监视器 ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ConvergenceMonitor {
    pub local_residuals: Vec<Vec<f32>>,
    pub global_residuals: Vec<f32>,
    pub deltas: Vec<f32>,
    pub converged: bool,
    pub iteration: usize,
    pub tol: f32,
}

impl ConvergenceMonitor {
    pub fn new(tol: f32) -> Self {
        Self { local_residuals: Vec::new(), global_residuals: Vec::new(), deltas: Vec::new(), converged: false, iteration: 0, tol }
    }
    pub fn record(&mut self, _n_clusters: usize) {
        self.iteration = self.global_residuals.len();
    }
    pub fn global_norm(&self) -> f32 {
        self.global_residuals.last().copied().unwrap_or(0.0)
    }
}

// ─── 工具函数 ──────────────────────────────────────────────

/// 高斯消元(列选主元) 解方阵 J·x = b
pub fn gauss_solve(j: &[Vec<f32>], b: &[f32]) -> Option<Vec<f32>> {
    let n = b.len();
    if n == 0 { return Some(vec![]); }
    let mut a = j.to_vec();
    let mut rhs = b.to_vec();
    for col in 0..n {
        let mut max_row = col;
        let mut max_val = a[col][col].abs();
        for row in (col + 1)..n { if a[row][col].abs() > max_val { max_val = a[row][col].abs(); max_row = row; } }
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

// ─── 测试 ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dag_build() { assert_eq!(build_curvature_dag(0.5).n_nodes(), 7); }
    #[test]
    fn gauss_2x2() {
        let j = vec![vec![2.0, 1.0], vec![1.0, -1.0]];
        let b = vec![10.0, 2.0];
        let x = gauss_solve(&j, &b).unwrap();
        assert!((x[0] - 4.0).abs() < 1e-6);
        assert!((x[1] - 2.0).abs() < 1e-6);
    }
}
