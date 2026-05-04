/// ClusterSolver v2 — SCC 分块 Newton 求解器
///
/// 四层架构：
///   1. SccClusterer — Tarjan 算法划分强连通分量
///   2. ConstraintCompiler — 从 Node AST 提取约束闭包
///   3. BlockNewtonSolver — 逐块稀疏 Newton 求解
///   4. ClusterSolverV2 — 完整流程

use std::collections::{HashMap, HashSet};
use crate::ast::Expr;
use crate::graph::NebulaGraph;
use crate::guard::num::ensure_finite;
use crate::solver::Matrix;

// ============================================================
// 第 1 层：SCC 依赖图分析
// ============================================================

pub struct SccClusterer {
    n_nodes: usize,
    adj: Vec<Vec<usize>>,
}

impl SccClusterer {
    pub fn from_graph(graph: &NebulaGraph) -> Self {
        let n = graph.nodes.len();
        let mut adj = vec![Vec::new(); n];
        for edge in &graph.edges {
            let from = edge.from_node;
            let to = edge.to_node;
            if from < n && to < n && from != to && !adj[from].contains(&to) {
                adj[from].push(to);
            }
        }
        SccClusterer { n_nodes: n, adj }
    }

    pub fn compute_scc(&self) -> Vec<Vec<usize>> {
        let n = self.n_nodes;
        let mut index = 0usize;
        let mut indices = vec![0usize; n];
        let mut lowlink = vec![0usize; n];
        let mut on_stack = vec![false; n];
        let mut stack = Vec::new();
        let mut sccs = Vec::new();

        fn strongconnect(
            v: usize, index: &mut usize, indices: &mut [usize], lowlink: &mut [usize],
            on_stack: &mut [bool], stack: &mut Vec<usize>, sccs: &mut Vec<Vec<usize>>,
            adj: &[Vec<usize>],
        ) {
            *index += 1;
            indices[v] = *index; lowlink[v] = *index;
            stack.push(v); on_stack[v] = true;
            for &w in &adj[v] {
                if indices[w] == 0 {
                    strongconnect(w, index, indices, lowlink, on_stack, stack, sccs, adj);
                    lowlink[v] = lowlink[v].min(lowlink[w]);
                } else if on_stack[w] {
                    lowlink[v] = lowlink[v].min(indices[w]);
                }
            }
            if lowlink[v] == indices[v] {
                let mut scc = Vec::new();
                loop { let w = stack.pop().unwrap(); on_stack[w] = false; scc.push(w); if w == v { break; } }
                scc.sort();
                sccs.push(scc);
            }
        }
        for v in 0..n { if indices[v] == 0 { strongconnect(v, &mut index, &mut indices, &mut lowlink, &mut on_stack, &mut stack, &mut sccs, &self.adj); } }
        sccs.reverse();
        sccs
    }
}

// ============================================================
// 第 2 层：Constraint Compiler
// ============================================================

pub struct CompiledConstraint {
    pub node_id: usize,
    pub func: Box<dyn Fn(&[f64]) -> f64>,
    pub var_syms: Vec<(String, usize)>,
}

unsafe impl Send for CompiledConstraint {}
unsafe impl Sync for CompiledConstraint {}

pub struct ConstraintCompiler;

impl ConstraintCompiler {
    pub fn build_global_var_map(graph: &NebulaGraph) -> HashMap<String, usize> {
        let mut vm = HashMap::new();
        let mut i = 0;
        for node in &graph.nodes {
            for s in &node.formula.symbols().clone() {
                if !vm.contains_key(s.as_str()) { vm.insert(s.clone(), i); i += 1; }
            }
        }
        vm
    }

    pub fn compile_block(graph: &NebulaGraph, scc: &[usize], gvm: &HashMap<String, usize>) -> (Vec<usize>, Vec<CompiledConstraint>) {
        let mut bvs: HashSet<usize> = HashSet::new();
        let mut cons = Vec::new();
        let gvm_owned = gvm.clone();

        for &nid in scc {
            if let Some(node) = graph.nodes.iter().find(|n| n.id == nid) {
                if let Expr::Eq(lhs, rhs) = &node.formula {
                    let syms: Vec<String> = node.formula.symbols().clone();
                    let lv: Vec<(String, usize)> = syms.iter().filter_map(|s| gvm_owned.get(s.as_str()).map(|&idx| (s.clone(), idx))).collect();
                    for &(_, idx) in &lv { bvs.insert(idx); }
                    let lc = lhs.clone(); let rc = rhs.clone(); let lv2 = lv.clone();
                    cons.push(CompiledConstraint {
                        node_id: nid,
                        func: Box::new(move |x| {
                            let mut ctx = HashMap::new();
                            for (s, idx) in &lv2 { if *idx < x.len() { ctx.insert(s.clone(), x[*idx]); } }
                            match (lc.eval(&ctx), rc.eval(&ctx)) { (Ok(a), Ok(b)) => a - b, _ => f64::NAN }
                        }),
                        var_syms: lv,
                    });
                }
            }
        }
        let mut bvi: Vec<usize> = bvs.into_iter().collect();
        bvi.sort();
        (bvi, cons)
    }
}

// ============================================================
// 第 3 层：Block Newton Solver
// ============================================================

pub struct BlockNewtonSolver {
    pub eps: f64, pub tol: f64, pub max_iter: usize, pub damping: f64,
}

impl BlockNewtonSolver {
    pub fn new() -> Self { BlockNewtonSolver { eps: 1e-6, tol: 1e-9, max_iter: 100, damping: 1.0 } }

    pub fn step_block(&self, block_vars: &[usize], constraints: &[CompiledConstraint], global_x: &mut [f64]) -> Result<bool, &'static str> {
        let n = block_vars.len();
        let m = constraints.len();
        if n == 0 || m == 0 { return Ok(true); }
        if m < n { return Ok(false); }

        let f0: Vec<f64> = constraints.iter().map(|c| (c.func)(global_x)).collect();
        for &v in &f0 { ensure_finite(v)?; }

        let mut j = Matrix::new(m, n);
        for (k, c) in constraints.iter().enumerate() {
            for &(_, var_idx) in &c.var_syms {
                if let Some(i) = block_vars.iter().position(|&v| v == var_idx) {
                    let orig = global_x[var_idx];
                    global_x[var_idx] += self.eps;
                    let f1 = (c.func)(global_x);
                    global_x[var_idx] = orig;
                    j.set(k, i, ensure_finite((f1 - f0[k]) / self.eps)?);
                }
            }
        }

        let rhs: Vec<f64> = f0.iter().map(|v| -v).collect();

        match j.solve(rhs) {
            Ok(dx) => {
                for i in 0..n { let g = block_vars[i]; global_x[g] += dx[i] * self.damping; ensure_finite(global_x[g])?; }
                let nf: Vec<f64> = constraints.iter().map(|c| (c.func)(global_x)).collect();
                Ok(nf.iter().map(|v| v.abs()).sum::<f64>() / (m as f64) < self.tol)
            }
            Err(_) => Err("singular Jacobian"),
        }
    }
}

// ============================================================
// 第 4 层：ClusterSolverV2
// ============================================================

pub struct ClusterSolverV2 {
    pub x: Vec<f64>,
    pub var_map: HashMap<String, usize>,
    pub blocks: Vec<(Vec<usize>, Vec<CompiledConstraint>)>,
    pub newton: BlockNewtonSolver,
}

impl ClusterSolverV2 {
    pub fn from_graph(graph: &NebulaGraph) -> Self {
        let vm = ConstraintCompiler::build_global_var_map(graph);
        let sccs = SccClusterer::from_graph(graph).compute_scc();
        let mut x = vec![0.0; vm.len()];
        for edge in &graph.edges { if let Some(d) = edge.default_value { if let Some(&idx) = vm.get(&edge.from_symbol) { if x[idx] == 0.0 { x[idx] = d; } } } }
        let mut blocks = Vec::new();
        for scc in &sccs { let (bv, cons) = ConstraintCompiler::compile_block(graph, scc, &vm); if !cons.is_empty() { blocks.push((bv, cons)); } }
        ClusterSolverV2 { x, var_map: vm, blocks, newton: BlockNewtonSolver::new() }
    }

    pub fn tick(&mut self) -> f64 {
        let mut tr = 0.0; let mut nc = 0;
        for (bv, cons) in &self.blocks {
            match self.newton.step_block(bv, cons, &mut self.x) {
                Ok(_) => { let f: Vec<f64> = cons.iter().map(|c| (c.func)(&self.x)).collect(); tr += f.iter().map(|v| v.abs()).sum::<f64>(); nc += cons.len(); }
                Err(_) => { let f: Vec<f64> = cons.iter().map(|c| (c.func)(&self.x)).collect(); tr += f.iter().map(|v| v.abs()).sum::<f64>(); nc += cons.len(); }
            }
        }
        if nc > 0 { tr / nc as f64 } else { 0.0 }
    }

    pub fn get_value(&self, symbol: &str) -> Option<f64> { self.var_map.get(symbol).map(|&idx| self.x[idx]) }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse_simple_eq;
    use crate::graph::NebulaGraph;

    #[test]
    fn test_scc_self_loop() {
        let mut g = NebulaGraph::new();
        let n1 = g.add_node(parse_simple_eq("next_a = a + 1").unwrap());
        g.add_edge_with_default(n1, "next_a", n1, "a", 0.0);
        assert_eq!(SccClusterer::from_graph(&g).compute_scc().len(), 1);
    }

    #[test]
    fn test_var_map_unique() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("a + b = 10").unwrap());
        g.add_node(parse_simple_eq("a - b = 2").unwrap());
        let vm = ConstraintCompiler::build_global_var_map(&g);
        assert_eq!(vm.len(), 2);
    }

    #[test]
    fn test_linear_3x3_nonsingular() {
        // x + y = 7, x - y = 1, 2x + z = 10  → x=4, y=3, z=2
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 7").unwrap());
        g.add_node(parse_simple_eq("x - y = 1").unwrap());
        g.add_node(parse_simple_eq("2*x + z = 10").unwrap());
        let vm = ConstraintCompiler::build_global_var_map(&g);
        let all: Vec<usize> = (0..g.nodes.len()).collect();
        let (bv, cons) = ConstraintCompiler::compile_block(&g, &all, &vm);
        let mut x = vec![0.0, 0.0, 0.0];
        let newton = BlockNewtonSolver { eps: 1e-6, tol: 1e-9, max_iter: 50, damping: 1.0 };
        for _ in 0..20 { let _ = newton.step_block(&bv, &cons, &mut x); }
        let get = |s: &str| vm.get(s).map(|&i| x[i]).unwrap_or(0.0);
        println!("linear 3x3: x={:.4} y={:.4} z={:.4}", get("x"), get("y"), get("z"));
        assert!((get("x") - 4.0).abs() < 1.0);
        assert!((get("y") - 3.0).abs() < 1.0);
        assert!((get("z") - 2.0).abs() < 1.0);
    }
}
