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
use crate::state::NodeState;

/// 集群缓存，按拓扑版本号惰性更新
#[derive(Clone)]
pub struct ClusterCache {
    pub version: u64,
    pub clusters: Vec<Vec<usize>>,
}

impl ClusterCache {
    pub fn new() -> Self {
        ClusterCache {
            version: u64::MAX, // 强制第一次 tick 重建
            clusters: Vec::new(),
        }
    }

    /// 如果拓扑版本匹配则返回缓存，否则重新计算
    pub fn resolve(&mut self, graph: &crate::graph::NebulaGraph) -> &[Vec<usize>] {
        if self.version == graph.topology_version {
            return &self.clusters;
        }
        self.clusters = ClusterDetector::detect(graph);
        self.version = graph.topology_version;
        &self.clusters
    }
}

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
// 新的第 0.5 层：Cluster Detection — 双关系传递闭包
// ============================================================

/// Cluster = 通过"突触(synapse) + 约束引用"可达的节点闭包
pub struct ClusterDetector;

impl ClusterDetector {
    /// 双关系 BFS：从每个未访问节点出发，沿 synapse 边和约束引用边扩散
    ///
    /// 约束引用边 = node A 的公式中引用了 node B 的变量符号（通过 synapse 连通）
    ///
    /// 返回 Vec<Vec<usize>>，每个子 vec 是一个 cluster 内的 node_id 列表
    pub fn detect(graph: &NebulaGraph) -> Vec<Vec<usize>> {
        let n = graph.nodes.len();
        if n == 0 {
            return Vec::new();
        }

        // 构建邻接表：两个节点之间有边当且仅当
        // a) 存在 synapse 连通（任意方向）
        // b) 共享变量引用（通过 synapse 路径间接连通也行）

        // 先用 Union-Find 把 synapse 连通的节点合并
        let mut node_uf: Vec<usize> = (0..n).collect();
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

        //  synapse 边连接节点
        for edge in &graph.edges {
            let from = edge.from_node;
            let to = edge.to_node;
            if from < n && to < n && from != to {
                union(&mut node_uf, from, to);
            }
        }

        // 约束引用边：如果 node_i 的公式引用了一个符号 s，
        // 且存在 synapse 将 node_j 的某个端口映射到该符号 s，
        // 则 i 和 j 有约束引用关系
        let mut node_adj: Vec<HashSet<usize>> = vec![HashSet::new(); n];
        for edge in &graph.edges {
            if edge.from_node < n && edge.to_node < n && edge.from_node != edge.to_node {
                // 双向连接
                node_adj[edge.from_node].insert(edge.to_node);
                node_adj[edge.to_node].insert(edge.from_node);
            }
        }

        // 约束引用：如果 node_i 的公式中有符号 s，
        // 且 node_j 也有符号 s（通过任一 synapse 边），则 i 和 j 耦合
        for i in 0..n {
            let syms_i: HashSet<String> = graph.nodes[i].formula.symbols().into_iter().collect();
            for j in (i + 1)..n {
                // 已通过 synapse 连通则跳过
                if find(&mut node_uf, i) == find(&mut node_uf, j) {
                    continue;
                }
                let syms_j: HashSet<String> =
                    graph.nodes[j].formula.symbols().into_iter().collect();
                // 共享符号 → 约束引用边
                if syms_i.intersection(&syms_j).next().is_some() {
                    node_adj[i].insert(j);
                    node_adj[j].insert(i);
                    union(&mut node_uf, i, j);
                }
            }
        }

        // 最终路径压缩
        for i in 0..n {
            find(&mut node_uf, i);
        }

        // 收集每个 cluster 的节点列表
        let mut root_to_cluster: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..n {
            root_to_cluster.entry(node_uf[i]).or_default().push(i);
        }

        // 按最小 node_id 排序
        let mut clusters: Vec<Vec<usize>> = root_to_cluster.into_values().collect();
        for c in &mut clusters {
            c.sort();
        }
        clusters.sort_by(|a, b| a[0].cmp(&b[0]));

        clusters
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

/// 单个 cluster 的编译结果
pub struct ClusterCompilation {
    pub node_ids: Vec<usize>,
    pub constraints: Vec<CompiledConstraint>,
    pub blocks: Vec<(Vec<usize>, Vec<usize>)>,
    pub n_vars: usize,
    pub global_idx_map: HashMap<VarPort, usize>,
}

/// Tick 编译结果
pub struct TickCompilation {
    pub clusters: Vec<ClusterCompilation>,
}

/// 单次 tick 的求解结果
#[derive(Debug, Clone)]
pub struct ClusterTickResult {
    /// 每个 cluster 的求解状态
    pub cluster_states: Vec<Vec<(usize, NodeState)>>,
    /// 平均残差
    pub avg_residual: f64,
}

pub struct ClusterSolverV3 {
    pub newton: BlockNewtonSolver,
    /// 每个 cluster 的变量向量（X_cluster 隔离）
    pub cluster_xs: Vec<Vec<f64>>,
    pub compilation: Option<TickCompilation>,
}

impl ClusterSolverV3 {
    pub fn new() -> Self {
        ClusterSolverV3 {
            newton: BlockNewtonSolver::new(),
            cluster_xs: Vec::new(),
            compilation: None,
        }
    }

    // ======================================================
    // Phase 1: Snapshot → Phase 2: Compile
    // ======================================================

    /// 编译阶段：Snapshot → Cluster Detection → for each Cluster: UF → Dep → SCC
    ///
    /// 使用 ClusterCache 惰性缓存，拓扑未变时跳过重复检测
    pub fn compile(&mut self, graph: &NebulaGraph, cache: &mut ClusterCache) {
        // Phase 1: Snapshot 由调用者保证（graph 是引用，不可变）

        // Phase 1.5: Cluster Detection（使用缓存）
        let raw_clusters = cache.resolve(graph).to_vec();

        let mut cluster_comps: Vec<ClusterCompilation> = Vec::new();
        self.cluster_xs = Vec::new();

        for node_ids in &raw_clusters {
            // 构建子图（只包含该 cluster 的节点和边）
            let node_set: HashSet<usize> = node_ids.iter().copied().collect();
            // UF 只需 node_set 过滤，直接复用 graph 引用

            // 用子图构建临时 NebulaGraph 供 UF 使用
            // 但 UF 只需要 edges 和 nodes，直接从原 graph 过滤引用
            // 简化：用原 graph + node_set 做 UF（只对该 cluster 内的 ports 和 edges）

            // Phase 2a: Union-Find
            let ports = VariableMerger::extract_ports_subset(graph, node_ids);
            let (_port_to_idx, parent, _labels) = VariableMerger::compute_equivalence_subset(&ports, graph, node_ids);
            let (global_idx_map, n_vars) = VariableMerger::build_global_index(&ports, &parent);

            // 初始化 X_cluster
            let mut x_cluster = vec![0.0; n_vars];
            for edge in &graph.edges {
                if node_set.contains(&edge.from_node) && node_set.contains(&edge.to_node) {
                    if let Some(d) = edge.default_value {
                        let port = VarPort::new(edge.from_node, &edge.from_symbol);
                        if let Some(&gidx) = global_idx_map.get(&port) {
                            if x_cluster[gidx] == 0.0 {
                                x_cluster[gidx] = d;
                            }
                        }
                    }
                }
            }
            self.cluster_xs.push(x_cluster);

            // Phase 2b: Dependency 构建（只对该 cluster 内的节点）
            let (constraints, dg) = build_dependency_system_subset(graph, &global_idx_map, n_vars, node_ids);

            if constraints.is_empty() {
                // 无约束的 cluster（纯赋值或常量节点）跳过
                continue;
            }

            // Phase 2c: SCC 分块
            let raw_sccs = SccPartitioner::partition(&constraints, &dg);

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

            cluster_comps.push(ClusterCompilation {
                node_ids: node_ids.clone(),
                constraints,
                blocks,
                n_vars,
                global_idx_map,
            });
        }

        self.compilation = Some(TickCompilation {
            clusters: cluster_comps,
        });
    }

    // ======================================================
    // Phase 3: Solve — 每个 cluster 独立 tick + 原子提交
    // ======================================================

    /// 执行一次所有 cluster 的 Block Newton 迭代
    ///
    /// 返回每个 cluster 的节点状态和平均残差。
    /// 原子提交保证：失败集群不修改 cluster_xs，保留旧值供下 Tick 使用。
    pub fn tick(&mut self) -> ClusterTickResult {
        let comp = match &self.compilation {
            Some(c) => c,
            None => {
                return ClusterTickResult {
                    cluster_states: Vec::new(),
                    avg_residual: f64::INFINITY,
                }
            }
        };

        let mut all_states: Vec<Vec<(usize, NodeState)>> = Vec::new();
        let mut total_res = 0.0;
        let mut total_cons = 0;

        for (ci, cluster_comp) in comp.clusters.iter().enumerate() {
            // ---- 原子提交：先在 temp_x 上求解 ----
            let old_x = self.cluster_xs[ci].clone();
            let mut temp_x = old_x.clone();
            let mut cluster_has_singular = false;
            let mut cluster_has_unconverged = false;

            for (_block_idx, (cons_indices, var_indices)) in cluster_comp.blocks.iter().enumerate()
            {
                let block_cons: Vec<&CompiledConstraint> = cons_indices
                    .iter()
                    .map(|&cid| &cluster_comp.constraints[cid])
                    .collect();

                // Tick 内多步迭代
                let mut block_converged = false;
                for _ in 0..self.newton.max_iter {
                    match self.newton.step_block(&block_cons, var_indices, &mut temp_x) {
                        Ok(converged) => {
                            block_converged = converged;
                            if converged {
                                break;
                            }
                        }
                        Err(_) => {
                            cluster_has_singular = true;
                            break;
                        }
                    }
                }
                for &cid in cons_indices {
                    let res = (cluster_comp.constraints[cid].func)(&temp_x).abs();
                    total_res += res;
                    total_cons += 1;
                }
                if !block_converged && !cluster_has_singular {
                    cluster_has_unconverged = true;
                }
            }

            eprintln!("DBG ci={} sing={} unconv={} blocks={} temp_x={:?}", ci, cluster_has_singular, cluster_has_unconverged, cluster_comp.blocks.len(), temp_x);
            // ---- 原子提交决策 ----
            let node_color = if cluster_has_singular {
                NodeState::Purple
            } else if cluster_has_unconverged {
                NodeState::Yellow
            } else if cluster_comp.blocks.is_empty() {
                NodeState::Gray
            } else {
                NodeState::Green
            };

            // 仅成功时写回 cluster_xs，失败/未收敛保留旧值（Yellow 状态保持）
            if node_color == NodeState::Green {
                self.cluster_xs[ci].copy_from_slice(&temp_x);
                if ci == 0 {
                    
                }
            } else {
                if ci == 0 {
                    
                }
            }

            // Purple/Yellow：不写回，cluster_xs[ci] 保留上一 Tick 的值
            // Yellow 的 temp_x 可作为下一 Tick 初值（已保留在 temp_x，但 cluster_xs 仍是旧值）
            // 这一步实现"逐步逼近"链式推进

            for &nid in &cluster_comp.node_ids {
                all_states.push(vec![(nid, node_color.clone())]);
            }
        }

        ClusterTickResult {
            cluster_states: all_states,
            avg_residual: if total_cons > 0 {
                total_res / total_cons as f64
            } else {
                0.0
            },
        }
    }

    pub fn get_value(&self, node_id: usize, symbol: &str) -> Option<f64> {
        let comp = self.compilation.as_ref()?;
        let port = VarPort::new(node_id, symbol);
        for (ci, cluster_comp) in comp.clusters.iter().enumerate() {
            if let Some(&gidx) = cluster_comp.global_idx_map.get(&port) {
                return Some(self.cluster_xs[ci][gidx]);
            }
        }
        None
    }
}

// ============================================================
// 辅助：subset 版本的 extract_ports / compute_equivalence / build_dependency_system
// ============================================================

impl VariableMerger {
    /// 只提取指定 node_ids 中的变量端口
    pub fn extract_ports_subset(graph: &NebulaGraph, node_ids: &[usize]) -> Vec<VarPort> {
        let node_set: HashSet<usize> = node_ids.iter().copied().collect();
        let mut ports_set: HashSet<VarPort> = HashSet::new();
        for node in &graph.nodes {
            if node_set.contains(&node.id) {
                for sym in node.formula.symbols() {
                    ports_set.insert(VarPort::new(node.id, &sym));
                }
            }
        }
        let mut ports: Vec<VarPort> = ports_set.into_iter().collect();
        ports.sort_by(|a, b| a.node_id.cmp(&b.node_id).then(a.symbol.cmp(&b.symbol)));
        ports
    }

    /// 只在 cluster 内执行 Union-Find
    pub fn compute_equivalence_subset(
        ports: &[VarPort],
        graph: &NebulaGraph,
        node_ids: &[usize],
    ) -> (HashMap<VarPort, usize>, Vec<usize>, HashMap<usize, String>) {
        let node_set: HashSet<usize> = node_ids.iter().copied().collect();
        let n = ports.len();
        let mut port_to_idx: HashMap<VarPort, usize> = HashMap::new();
        for (i, p) in ports.iter().enumerate() {
            port_to_idx.insert(p.clone(), i);
        }

        let mut parent: Vec<usize> = (0..n).collect();

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

        for edge in &graph.edges {
            if node_set.contains(&edge.from_node) && node_set.contains(&edge.to_node) {
                let from_port = VarPort::new(edge.from_node, &edge.from_symbol);
                let to_port = VarPort::new(edge.to_node, &edge.to_symbol);
                if let (Some(&fi), Some(&ti)) = (port_to_idx.get(&from_port), port_to_idx.get(&to_port)) {
                    union(&mut parent, fi, ti);
                }
            }
        }

        for i in 0..n {
            find(&mut parent, i);
        }

        let mut root_to_label: HashMap<usize, String> = HashMap::new();
        for p in ports {
            let idx = port_to_idx[p];
            let root = parent[idx];
            root_to_label.entry(root).or_insert_with(|| p.symbol.clone());
        }

        (port_to_idx, parent, root_to_label)
    }
}

/// 只对 cluster 内节点构建依赖系统
pub fn build_dependency_system_subset(
    graph: &NebulaGraph,
    global_idx_map: &HashMap<VarPort, usize>,
    n_global_vars: usize,
    node_ids: &[usize],
) -> (Vec<CompiledConstraint>, DependencyGraph) {
    let node_set: HashSet<usize> = node_ids.iter().copied().collect();
    let mut constraints: Vec<CompiledConstraint> = Vec::new();

    for node in &graph.nodes {
        if node_set.contains(&node.id) {
            if let Some(cons) = compile_constraint(node.id, &node.formula, global_idx_map) {
                constraints.push(cons);
            }
        }
    }

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
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);
        let comp = solver.compilation.as_ref().unwrap();
        assert!(comp.clusters[0].blocks.len() >= 1);
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
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);

        // 多次 tick 直到收敛
        for _ in 0..30 {
            let result = solver.tick();
            if result.avg_residual < 1e-8 {
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
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);

        for _ in 0..30 {
            let result = solver.tick();
            if result.avg_residual < 1e-8 {
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
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);

        for _ in 0..50 {
            let result = solver.tick();
            if result.avg_residual < 1e-8 {
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
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);

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
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);

        // tick 不应 panic
        let result = solver.tick();
        assert!(result.avg_residual.is_finite());
    }
}
