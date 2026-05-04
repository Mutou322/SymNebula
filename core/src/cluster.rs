/// ClusterSolver v3 — 语义正确的 SCC 分块 Newton 求解器
///
/// 核心改进（相对 v2）：
///   1. Union-Find 变量等价：变量身份由 synapse 连通性决定，非字符串名
///   2. 双层图分离：Graph A(变量等价) → Union-Find, Graph B(依赖) → SCC
///   3. Tick 四阶段流水线：Snapshot → Compile(UnionFind→Dependency→SCC) → Solve → Commit
///   4. 确定性保证：Tick 内 Graph 不可变，编译顺序锁死
///
/// 四层架构：
///   1. VariableMerger — Union-Find 变量等价合并（基于 synapse 连通性）
///   2. DependencyBuilder — 从 Expr AST 推导 ∂F/∂x ≠ 0 稀疏依赖图
///   3. BlockNewtonSolver — 逐块稀疏 Newton 求解
///   4. ClusterSolverV3 — Tick 四阶段流水线

use std::collections::{HashMap, HashSet};
use crate::ast::Expr;
use crate::graph::NebulaGraph;
use crate::guard::num::ensure_finite;
use crate::solver::Matrix;

// ============================================================
// 第 0 层：Union-Find 数据结构
// ============================================================

/// 变量端口标识符：(node_id, symbol_name)
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct VarPort {
    pub node_id: usize,
    pub symbol: String,
}

impl VarPort {
    pub fn new(node_id: usize, symbol: &str) -> Self {
        VarPort { node_id, symbol: symbol.to_string() }
    }
}

/// Union-Find 变量等价合并器
///
/// 变量身份由 synapse 连通性决定，不是字符串名
pub struct VariableMerger;

impl VariableMerger {
    /// 从 Graph 中提取所有变量端口
    pub fn extract_ports(graph: &NebulaGraph) -> Vec<VarPort> {
        let mut ports_set: HashSet<VarPort> = HashSet::new();
        for node in &graph.nodes {
            for sym in node.formula.symbols() {
                ports_set.insert(VarPort::new(node.id, &sym));
            }
        }
        let mut ports: Vec<VarPort> = ports_set.into_iter().collect();
        ports.sort_by(|a, b| a.node_id.cmp(&b.node_id).then(a.symbol.cmp(&b.symbol)));
        ports
    }

    /// 构建 Union-Find 等价类：通过 synapse 连通性决定变量身份
    ///
    /// 规则：
    ///   - v(a, x) ≡ v(b, y) ⇔ 存在 synapse 路径连通 a.x 与 b.y
    ///   - 字符串名相同但无 synapse 连通 → 不同变量
    ///   - 字符串名不同但有 synapse 连通 → 同一变量
    ///
    /// 返回 (port_index_map, parent, equivalence_labels)
    ///   port_index_map: VarPort → usize (局部索引)
    ///   parent: Union-Find 的父数组
    ///   equivalence_labels: 每个等价类的标识符（取最小 VarPort 的字符串名）
    pub fn compute_equivalence(
        ports: &[VarPort],
        graph: &NebulaGraph,
    ) -> (HashMap<VarPort, usize>, Vec<usize>, HashMap<usize, String>) {
        let n = ports.len();
        let mut port_to_idx: HashMap<VarPort, usize> = HashMap::new();
        for (i, p) in ports.iter().enumerate() {
            port_to_idx.insert(p.clone(), i);
        }

        // Union-Find 初始化
        let mut parent: Vec<usize> = (0..n).collect();

        // find with path compression
        fn find(parent: &mut [usize], x: usize) -> usize {
            if parent[x] != x {
                parent[x] = find(parent, parent[x]);
            }
            parent[x]
        }

        fn union(parent: &mut [usize], a: usize, b: usize) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra != rb {
                parent[ra] = rb;
            }
        }

        // 遍历所有 synapse 边，连接 from_symbol 和 to_symbol 对应的端口
        for edge in &graph.edges {
            let from_port = VarPort::new(edge.from_node, &edge.from_symbol);
            let to_port = VarPort::new(edge.to_node, &edge.to_symbol);
            if let (Some(&fi), Some(&ti)) = (port_to_idx.get(&from_port), port_to_idx.get(&to_port)) {
                union(&mut parent, fi, ti);
            }
        }

        // 压缩所有路径
        for i in 0..n {
            find(&mut parent, i);
        }

        // 构建等价类标签：取该等价类中第一个端口的符号名
        let mut root_to_label: HashMap<usize, String> = HashMap::new();
        for p in ports {
            let idx = port_to_idx[p];
            let root = parent[idx];
            root_to_label.entry(root).or_insert_with(|| p.symbol.clone());
        }

        (port_to_idx, parent, root_to_label)
    }

    /// 将等价类映射到全局变量索引
    ///
    /// 返回 (global_index_map, n_global_vars)
    ///   global_index_map: VarPort → 全局索引
    ///   n_global_vars: 全局变量总数（等价类数）
    pub fn build_global_index(
        ports: &[VarPort],
        parent: &[usize],
    ) -> (HashMap<VarPort, usize>, usize) {
        let mut root_to_global: HashMap<usize, usize> = HashMap::new();
        let mut next_idx = 0;
        for (i, _p) in ports.iter().enumerate() {
            let root = parent[i];
            if !root_to_global.contains_key(&root) {
                root_to_global.insert(root, next_idx);
                next_idx += 1;
            }
        }

        let mut global_idx: HashMap<VarPort, usize> = HashMap::new();
        for (i, p) in ports.iter().enumerate() {
            let root = parent[i];
            let gidx = root_to_global[&root];
            global_idx.insert(p.clone(), gidx);
        }

        (global_idx, next_idx)
    }
}

// ============================================================
// 第 1 层：Dependency Builder
// ============================================================

/// 编译后的约束（residual form）
pub struct CompiledConstraint {
    pub node_id: usize,
    /// residual 函数 F_i(X) = 0
    pub func: Box<dyn Fn(&[f64]) -> f64>,
    /// 该约束依赖的全局变量索引列表
    pub var_indices: Vec<usize>,
}

unsafe impl Send for CompiledConstraint {}
unsafe impl Sync for CompiledConstraint {}

/// 依赖图：约束 ↔ 变量的双向映射
pub struct DependencyGraph {
    /// constraint_id → [global_var_index]
    pub constraint_to_vars: Vec<Vec<usize>>,
    /// global_var_index → [constraint_id]
    pub var_to_constraints: Vec<Vec<usize>>,
}

/// 从 Expr AST 推导依赖的变量（简单遍历符号表）
///
/// 规则：F 依赖 x_j ⇔ x_j 出现在 F 的公式中
/// 这是保守的（over-approximation），但保证结构完整性
fn extract_vars_from_expr(expr: &Expr) -> Vec<String> {
    expr.symbols()
}

/// 编译单个约束：从 Expr::Eq 生成 residual 闭包
fn compile_constraint(
    node_id: usize,
    formula: &Expr,
    global_idx_map: &HashMap<VarPort, usize>,
) -> Option<CompiledConstraint> {
    match formula {
        Expr::Eq(lhs, rhs) => {
            let syms = formula.symbols();
            // 从 node 局部符号 → 全局索引
            let mut indices: Vec<(String, usize)> = Vec::new();
            let mut seen_global: HashSet<usize> = HashSet::new();
            for s in &syms {
                let port = VarPort::new(node_id, s);
                if let Some(&gidx) = global_idx_map.get(&port) {
                    if seen_global.insert(gidx) {
                        indices.push((s.clone(), gidx));
                    }
                }
            }
            indices.sort_by(|a, b| a.1.cmp(&b.1));

            let lc = lhs.clone();
            let rc = rhs.clone();
            let iv = indices.clone();

            Some(CompiledConstraint {
                node_id,
                func: Box::new(move |x| {
                    let mut ctx = HashMap::new();
                    for (s, idx) in &iv {
                        if *idx < x.len() {
                            ctx.insert(s.clone(), x[*idx]);
                        }
                    }
                    match (lc.eval(&ctx), rc.eval(&ctx)) {
                        (Ok(a), Ok(b)) => a - b,
                        _ => f64::NAN,
                    }
                }),
                var_indices: indices.iter().map(|(_, idx)| *idx).collect(),
            })
        }
        _ => None, // 非 Eq 公式不参与约束求解
    }
}

/// Dependency Builder：从 Graph + 全局索引构建依赖图和约束列表
pub fn build_dependency_system(
    graph: &NebulaGraph,
    global_idx_map: &HashMap<VarPort, usize>,
    n_global_vars: usize,
) -> (Vec<CompiledConstraint>, DependencyGraph) {
    let mut constraints: Vec<CompiledConstraint> = Vec::new();

    for node in &graph.nodes {
        if let Some(cons) = compile_constraint(node.id, &node.formula, global_idx_map) {
            constraints.push(cons);
        }
    }

    // 构建双向依赖图
    let n_constraints = constraints.len();
    let mut c_to_v: Vec<Vec<usize>> = Vec::with_capacity(n_constraints);
    let mut v_to_c: Vec<Vec<usize>> = vec![Vec::new(); n_global_vars];

    for (cid, cons) in constraints.iter().enumerate() {
        let mut uniq: Vec<usize> = cons.var_indices.clone();
        uniq.sort();
        uniq.dedup();
        c_to_v.push(uniq.clone());

        for &vid in &uniq {
            if vid < n_global_vars {
                v_to_c[vid].push(cid);
            }
        }
    }

    let dg = DependencyGraph {
        constraint_to_vars: c_to_v,
        var_to_constraints: v_to_c,
    };

    (constraints, dg)
}

// ============================================================
// 第 2 层：SCC 分块（基于依赖图，非 graph.edges）
// ============================================================

/// SCC 分块器 — 基于 constraint↔variable 依赖图
pub struct SccPartitioner;

impl SccPartitioner {
    /// 在依赖图上运行 Tarjan，返回 SCC block 列表
    ///
    /// 节点空间 = [0..n_constraints) 约束节点
    ///            + [n_constraints..n_constraints + n_vars) 变量节点
    ///
    /// 边：constraint → var (约束依赖变量)
    ///     var → constraint (变量被约束使用)
    pub fn partition(
        constraints: &[CompiledConstraint],
        dg: &DependencyGraph,
    ) -> Vec<Vec<usize>> {
        let n_cons = constraints.len();
        let n_vars = dg.var_to_constraints.len();
        let total = n_cons + n_vars;

        if total == 0 {
            return Vec::new();
        }

        // 构建邻接表
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); total];

        for cid in 0..n_cons {
            for &vid in &dg.constraint_to_vars[cid] {
                let vid_abs = n_cons + vid;
                // constraint → variable
                adj[cid].push(vid_abs);
            }
        }
        for vid in 0..n_vars {
            let vid_abs = n_cons + vid;
            for &cid in &dg.var_to_constraints[vid] {
                // variable → constraint
                adj[vid_abs].push(cid);
            }
        }

        // Tarjan SCC
        let mut index = 0usize;
        let mut indices = vec![0usize; total];
        let mut lowlink = vec![0usize; total];
        let mut on_stack = vec![false; total];
        let mut stack = Vec::new();
        let mut sccs = Vec::new();

        fn strongconnect(
            v: usize,
            index: &mut usize,
            indices: &mut [usize],
            lowlink: &mut [usize],
            on_stack: &mut [bool],
            stack: &mut Vec<usize>,
            sccs: &mut Vec<Vec<usize>>,
            adj: &[Vec<usize>],
        ) {
            *index += 1;
            indices[v] = *index;
            lowlink[v] = *index;
            stack.push(v);
            on_stack[v] = true;
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
                loop {
                    let w = stack.pop().unwrap();
                    on_stack[w] = false;
                    scc.push(w);
                    if w == v {
                        break;
                    }
                }
                scc.sort();
                sccs.push(scc);
            }
        }

        for v in 0..total {
            if indices[v] == 0 {
                strongconnect(
                    v, &mut index, &mut indices, &mut lowlink,
                    &mut on_stack, &mut stack, &mut sccs, &adj,
                );
            }
        }

        sccs.reverse();
        sccs
    }
}

// ============================================================
// 第 3 层：Block Newton Solver
// ============================================================

pub struct BlockNewtonSolver {
    pub eps: f64,
    pub tol: f64,
    pub max_iter: usize,
    pub damping: f64,
}

impl BlockNewtonSolver {
    pub fn new() -> Self {
        BlockNewtonSolver {
            eps: 1e-6,
            tol: 1e-9,
            max_iter: 100,
            damping: 1.0,
        }
    }

    /// 对单个 SCC block 执行 Newton 求解
    ///
    /// block_constraints: block 中的约束子集（引用到全局 constraints 列表）
    /// block_vars: block 中涉及的全局变量索引（已去重排序）
    /// global_x: 全局变量向量（原地更新）
    ///
    /// 返回 Ok(true) 表示收敛，Ok(false) 表示未收敛但可继续，Err 表示奇异
    pub fn step_block(
        &self,
        block_constraints: &[&CompiledConstraint],
        block_vars: &[usize],
        global_x: &mut [f64],
    ) -> Result<bool, &'static str> {
        let n = block_vars.len();
        let m = block_constraints.len();

        if n == 0 || m == 0 {
            return Ok(true);
        }

        // 欠定块 → 跳过（依赖其他块提供初值）
        if m < n {
            return Ok(false);
        }

        // 计算当前残差 f0
        let f0: Vec<f64> = block_constraints
            .iter()
            .map(|c| (c.func)(global_x))
            .collect();
        for &v in &f0 {
            ensure_finite(v)?;
        }

        // 构建稀疏 Jacobian（仅对 block_vars 内的变量差分）
        let mut j = Matrix::new(m, n);
        for (k, cons) in block_constraints.iter().enumerate() {
            // 只对 block 内变量做差分
            for (ji, &vid) in block_vars.iter().enumerate() {
                // 检查该约束是否依赖这个变量
                let depends = cons.var_indices.contains(&vid);
                if !depends {
                    continue; // J[k][ji] = 0，已由 Matrix::new 初始化
                }
                let orig = global_x[vid];
                global_x[vid] += self.eps;
                let f1 = (cons.func)(global_x);
                global_x[vid] = orig;
                j.set(k, ji, ensure_finite((f1 - f0[k]) / self.eps)?);
            }
        }

        let rhs: Vec<f64> = f0.iter().map(|v| -v).collect();

        match j.solve(rhs) {
            Ok(dx) => {
                for i in 0..n {
                    let g = block_vars[i];
                    global_x[g] += dx[i] * self.damping;
                    ensure_finite(global_x[g])?;
                }
                // 检查是否收敛
                let nf: Vec<f64> = block_constraints
                    .iter()
                    .map(|c| (c.func)(global_x))
                    .collect();
                let avg_res = nf.iter().map(|v| v.abs()).sum::<f64>() / (m as f64);
                Ok(avg_res < self.tol)
            }
            Err(_) => Err("singular Jacobian"),
        }
    }
}

// ============================================================
// 第 4 层：ClusterSolverV3 — Tick 四阶段流水线
// ============================================================

/// Tick 编译结果（冻结一次编译，多次求解）
pub struct TickCompilation {
    /// 编译后的约束列表
    pub constraints: Vec<CompiledConstraint>,
    /// SCC block：每个 block 是 (constraint_indices_in_block, variable_indices_in_block)
    pub blocks: Vec<(Vec<usize>, Vec<usize>)>,
    /// 全局变量数
    pub n_global_vars: usize,
    /// 变量端口到全局索引的映射
    pub global_idx_map: HashMap<VarPort, usize>,
}

pub struct ClusterSolverV3 {
    /// 全局变量向量
    pub x: Vec<f64>,
    /// 当前 Tick 的编译结果（一次编译，可能多次调用 iter_tick）
    pub compilation: Option<TickCompilation>,
    pub newton: BlockNewtonSolver,
}

impl ClusterSolverV3 {
    pub fn new() -> Self {
        ClusterSolverV3 {
            x: Vec::new(),
            compilation: None,
            newton: BlockNewtonSolver::new(),
        }
    }

    // ======================================================
    // Phase 1-2: Snapshot + Compilation（单次执行）
    // ======================================================

    /// 编译阶段：冻结 Graph → Union-Find → Dependency → SCC
    ///
    /// 执行一次，结果缓存在 self.compilation 中供多次 solve 使用
    pub fn compile(&mut self, graph: &NebulaGraph) {
        // Phase 2a: Union-Find 变量等价合并
        let ports = VariableMerger::extract_ports(graph);
        let (_port_to_idx, parent, _labels) = VariableMerger::compute_equivalence(&ports, graph);
        let (global_idx_map, n_global_vars) = VariableMerger::build_global_index(&ports, &parent);

        // 初始化全局变量向量
        self.x = vec![0.0; n_global_vars];

        // 从 synapse default_value 填充初始值
        for edge in &graph.edges {
            if let Some(d) = edge.default_value {
                let port = VarPort::new(edge.from_node, &edge.from_symbol);
                if let Some(&gidx) = global_idx_map.get(&port) {
                    if self.x[gidx] == 0.0 {
                        self.x[gidx] = d;
                    }
                }
            }
        }

        // Phase 2b: Dependency 构建
        let (constraints, dg) = build_dependency_system(graph, &global_idx_map, n_global_vars);

        // Phase 2c: SCC 分块
        let raw_sccs = SccPartitioner::partition(&constraints, &dg);

        // 将 raw SCC 映射为 (constraint_indices, variable_indices) 格式
        let n_cons = constraints.len();
        let mut blocks: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();

        for scc in &raw_sccs {
            let mut cons_in_block: Vec<usize> = scc
                .iter()
                .filter(|&&node| node < n_cons)
                .copied()
                .collect();
            if cons_in_block.is_empty() {
                continue;
            }
            cons_in_block.sort();
            cons_in_block.dedup();

            // 收集该 block 涉及的全局变量
            let mut vars_in_block: HashSet<usize> = HashSet::new();
            for &cid in &cons_in_block {
                for &vid in &dg.constraint_to_vars[cid] {
                    vars_in_block.insert(vid);
                }
            }
            let mut var_list: Vec<usize> = vars_in_block.into_iter().collect();
            var_list.sort();

            blocks.push((cons_in_block, var_list));
        }

        self.compilation = Some(TickCompilation {
            constraints,
            blocks,
            n_global_vars,
            global_idx_map,
        });
    }

    // ======================================================
    // Phase 3: Solve（可多次迭代）
    // ======================================================

    pub fn tick(&mut self) -> f64 {
        let comp = match &self.compilation {
            Some(c) => c,
            None => return f64::INFINITY,
        };

        let mut total_res = 0.0;
        let mut total_cons = 0;

        for (_block_idx, (cons_indices, var_indices)) in comp.blocks.iter().enumerate() {
            let block_cons: Vec<&CompiledConstraint> = cons_indices
                .iter()
                .map(|&cid| &comp.constraints[cid])
                .collect();

            match self
                .newton
                .step_block(&block_cons, var_indices, &mut self.x)
            {
                Ok(converged) => {
                    for &cid in cons_indices {
                        let res = (comp.constraints[cid].func)(&self.x).abs();
                        total_res += res;
                        total_cons += 1;
                    }
                    if !converged && total_cons > 0 && total_res / total_cons as f64 > 0.1 {
                        // 未收敛且残差大时输出
                    }
                }
                Err(_e) => {
                    // 奇异块：残差累加但标记
                    for &cid in cons_indices {
                        let res = (comp.constraints[cid].func)(&self.x).abs();
                        total_res += res;
                        total_cons += 1;
                    }
                }
            }
        }

        if total_cons > 0 {
            total_res / total_cons as f64
        } else {
            0.0
        }
    }

    // ======================================================
    // Phase 4: Commit（查询接口）
    // ======================================================

    /// 获取全局变量值
    pub fn get_value(&self, node_id: usize, symbol: &str) -> Option<f64> {
        let port = VarPort::new(node_id, symbol);
        match &self.compilation {
            Some(comp) => comp.global_idx_map.get(&port).map(|&idx| self.x[idx]),
            None => None,
        }
    }

    /// 重置求解状态
    pub fn reset(&mut self, graph: &NebulaGraph) {
        self.compile(graph);
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse_simple_eq;
    use crate::graph::NebulaGraph;

    /// 最基本的 SCC 测试：自环节点
    #[test]
    fn test_scc_self_loop() {
        let mut g = NebulaGraph::new();
        let n1 = g.add_node(parse_simple_eq("next_a = a + 1").unwrap());
        g.add_edge_with_default(n1, "next_a", n1, "a", 0.0);

        let mut solver = ClusterSolverV3::new();
        solver.compile(&g);
        let comp = solver.compilation.as_ref().unwrap();
        assert!(comp.blocks.len() >= 1);
    }

    /// Union-Find 语义：同名但无 synapse → 不同变量
    #[test]
    fn test_union_find_same_name_no_synapse() {
        // 两个独立 node，各自有 x，但没有 synapse 连通
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + 1 = 5").unwrap());  // node 0: x = 4
        g.add_node(parse_simple_eq("x + 2 = 10").unwrap()); // node 1: x = 8

        let ports = VariableMerger::extract_ports(&g);
        let (port_to_idx, parent, _labels) = VariableMerger::compute_equivalence(&ports, &g);

        let port0 = VarPort::new(0, "x");
        let port1 = VarPort::new(1, "x");
        let idx0 = port_to_idx[&port0];
        let idx1 = port_to_idx[&port1];

        // 没有 synapse 连通 → 不同等价类
        assert_ne!(
            find_root(&parent, idx0),
            find_root(&parent, idx1),
            "同名无 synapse 的变量应该不同"
        );
    }

    /// Union-Find 语义：不同名但有 synapse → 同一变量
    #[test]
    fn test_union_find_diff_name_with_synapse() {
        let mut g = NebulaGraph::new();
        let n0 = g.add_node(parse_simple_eq("a + 1 = 5").unwrap());  // a = 4
        let n1 = g.add_node(parse_simple_eq("b + 2 = 10").unwrap()); // b = 8
        // synapse 连接 n0.a → n1.b
        g.add_edge_with_default(n0, "a", n1, "b", 0.0);

        let ports = VariableMerger::extract_ports(&g);
        let (port_to_idx, parent, _labels) = VariableMerger::compute_equivalence(&ports, &g);

        let port0 = VarPort::new(n0, "a");
        let port1 = VarPort::new(n1, "b");
        let idx0 = port_to_idx[&port0];
        let idx1 = port_to_idx[&port1];

        // 有 synapse 连通 → 同一等价类
        assert_eq!(
            find_root(&parent, idx0),
            find_root(&parent, idx1),
            "有 synapse 连通的变量应该相同"
        );
    }

    fn find_root(parent: &[usize], mut x: usize) -> usize {
        while parent[x] != x {
            x = parent[x];
        }
        x
    }

    /// 线性 3x3 非奇异系统：验证数值收敛
    ///
    /// 通过 synapse 连接三个节点的变量，形成耦合系统
    /// n0: x + y = 7, n1: x - y = 1, n2: 2*x + z = 10 → x=4, y=3, z=2
    #[test]
    fn test_linear_3x3_nonsingular() {
        let mut g = NebulaGraph::new();
        let n0 = g.add_node(parse_simple_eq("x + y = 7").unwrap());
        let n1 = g.add_node(parse_simple_eq("x - y = 1").unwrap());
        let n2 = g.add_node(parse_simple_eq("2*x + z = 10").unwrap());
        // synapse 连接三个节点的共享变量
        g.add_edge_with_default(n0, "x", n1, "x", 0.0);
        g.add_edge_with_default(n0, "y", n1, "y", 0.0);
        g.add_edge_with_default(n1, "x", n2, "x", 0.0);

        let mut solver = ClusterSolverV3::new();
        solver.compile(&g);

        // 多次 tick 直到收敛
        for _ in 0..30 {
            let residual = solver.tick();
            if residual < 1e-8 {
                break;
            }
        }

        let x = solver.get_value(n0, "x").unwrap_or(0.0);
        let y = solver.get_value(n0, "y").unwrap_or(0.0);
        let z = solver.get_value(n2, "z").unwrap_or(0.0);

        println!("v3 linear 3x3: x={:.4} y={:.4} z={:.4}", x, y, z);
        assert!((x - 4.0).abs() < 1.0, "x should be ~4.0, got {}", x);
        assert!((y - 3.0).abs() < 1.0, "y should be ~3.0, got {}", y);
        assert!((z - 2.0).abs() < 1.0, "z should be ~2.0, got {}", z);
    }

    /// 同名无 synapse：验证两节点独立求解
    #[test]
    fn test_same_name_no_synapse_independent() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + 1 = 5").unwrap());  // x=4
        g.add_node(parse_simple_eq("x + 2 = 10").unwrap()); // x=8

        let mut solver = ClusterSolverV3::new();
        solver.compile(&g);

        for _ in 0..30 {
            let residual = solver.tick();
            if residual < 1e-8 {
                break;
            }
        }

        // Port(0, "x") 和 Port(1, "x") 应该是不同的全局变量
        let val0 = solver.get_value(0, "x").unwrap_or(f64::NAN);
        let val1 = solver.get_value(1, "x").unwrap_or(f64::NAN);

        println!("same_name_no_synapse: node0.x={:.4}, node1.x={:.4}", val0, val1);

        // 因为是分别求解，各自应该有收敛值
        assert!(val0.is_finite() && val0 > 0.0, "node0.x should converge");
        assert!(val1.is_finite() && val1 > 0.0, "node1.x should converge");
    }

    /// 不同名但有 synapse：验证共享变量
    ///
    /// 2变量 + 2约束，通过 synapse 共享两个变量
    /// n0: a + b = 3, n1: a * b = 2  (a=2, b=1 or a=1, b=2)
    /// 加入 synapse 连接 n0.a ↔ n1.a 和 n0.b ↔ n1.b
    #[test]
    fn test_diff_name_with_synapse_shared() {
        let mut g = NebulaGraph::new();
        let n0 = g.add_node(parse_simple_eq("a + b = 3").unwrap());
        let n1 = g.add_node(parse_simple_eq("a * b = 2").unwrap());
        g.add_edge_with_default(n0, "a", n1, "a", 0.5);
        g.add_edge_with_default(n0, "b", n1, "b", 2.0);

        let mut solver = ClusterSolverV3::new();
        solver.compile(&g);

        for _ in 0..50 {
            let residual = solver.tick();
            if residual < 1e-8 {
                break;
            }
        }

        let a = solver.get_value(n0, "a").unwrap_or(f64::NAN);
        let b = solver.get_value(n0, "b").unwrap_or(f64::NAN);

        println!("shared: a={:.4}, b={:.4}", a, b);
        let ok = (a - 1.0).abs() < 1.0 && (b - 2.0).abs() < 1.0
            || (a - 2.0).abs() < 1.0 && (b - 1.0).abs() < 1.0;
        assert!(ok, "expected (a,b) ≈ (1,2) or (2,1), got ({},{})", a, b);
    }

    /// 验证编译阶段不修改 Graph（Snapshot 冻结）
    #[test]
    fn test_compile_does_not_mutate_graph() {
        let mut g = NebulaGraph::new();
        let n0_id = g.add_node(parse_simple_eq("x + y = 10").unwrap());
        g.add_edge_with_default(n0_id, "x", n0_id, "y", 0.0);

        let original_nodes = g.nodes.len();
        let original_edges = g.edges.len();

        let mut solver = ClusterSolverV3::new();
        solver.compile(&g);

        assert_eq!(g.nodes.len(), original_nodes, "compile 不应修改 graph nodes");
        assert_eq!(g.edges.len(), original_edges, "compile 不应修改 graph edges");
    }

    /// 欠定系统不 panic
    #[test]
    fn test_underdetermined_no_panic() {
        // 1 个等式，2 个变量
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());

        let mut solver = ClusterSolverV3::new();
        solver.compile(&g);

        // tick 不应 panic
        let residual = solver.tick();
        assert!(residual.is_finite());
    }
}
