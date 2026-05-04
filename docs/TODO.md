# SymNebula 待办 & 未实现问题

## 工程级别

### 1. 符号求导 (Symbolic Differentiation)
**位置**: `core/src/ast.rs`
**描述**: 给 `Expr` 实现 `fn derive(&self, var: &str) -> Expr`，从 AST 符号树直接生成导数表达式。
**优势**:
- Jacobian 计算精度无穷高（无数值截断误差）
- 比数值扰动快几十倍（n≥3 时明显）
- 可链式求高阶导
**工程路径**:
```
Expr::Number(_) → 0
Expr::Symbol(s) → if s == var { 1 } else { 0 }
Expr::Add(a, b) → derive(a) + derive(b)
Expr::Mul(a, b) → derive(a)*b + a*derive(b)
Expr::Pow(a, b) → ... (复合)
```
**优先级**: 低（当前数值 Jacobian 在 n≤10 时足够用）

### 2. ClusterSolver 接入 Tick 流
**位置**: `core/src/engine.rs`
**描述**: 在 Scheduler 中增加 `cluster: Option<ClusterSolver>` 字段，Tick 的 Phase 1 中检测节点集是否耦合，对耦合集调用 `cluster.solve()` 代替逐节点 `solver_mgr.solve_node()`。
**问题**:
- 需要自动检测哪些节点构成耦合集（依赖图分析）
- 需要把节点公式转换为 `Constraint` 闭包（自动从 AST 构建）
- 需要把 solve 结果写回各节点的 `next_buffer`
**优先级**: 中

### 3. ClusterSolver 自动从 Graph 提取约束
**位置**: `core/src/cluster.rs`
**描述**: 加 `ClusterSolver::from_graph(graph: &NebulaGraph, node_ids: &[usize])`，自动从指定节点的公式中提取约束、收集变量名、构建闭包。
**优先级**: 中（依赖 #2）

### 4. 阻尼自动衰减 (Adaptive Damping)
**位置**: `core/src/cluster.rs`
**描述**: 当前 damping 是固定值 0.1。改成：如果残差上升则减小阻尼，残差下降稳定后增大阻尼（类似 Armijo rule 或 Levenberg-Marquardt）。
**优先级**: 低

### 5. 残余 `unwrap()` 清理
**位置**: `core/src/engine.rs` 测试代码中、`core/src/solver.rs` 测试代码中
**描述**: 测试中仍有少量 `.unwrap()`（如 `get_value(...).unwrap()`）。这些不会在生产路径触发，但严格来说违反了零 panic 原则。
**优先级**: 极低（测试代码，不影响运行时安全）

---

## 文档级别

### 6. ClusterSolver 架构文档
**位置**: `docs/CLUSTER_SOLVER.md`
**描述**: 写 ClusterSolver 的设计文档，包含数学推导、Rust 接口、与 Tick 集成方式。
**优先级**: 中（等 #2 完成后再写）

### 7. 符号求导文档
**位置**: `docs/SYMBOLIC_DERIV.md`
**描述**: 符号求导的数学原理 + Expr AST 推导规则表。
**优先级**: 低

### 10. Cluster Builder
**描述**: 从 Graph 自动构建 ClusterSolver。扫描边拓扑，找出耦合节点集，提取变量和公式到 constraints + x。
**三步骤**:
1. 依赖图分析 — 找出共享变量符号的耦合节点群
2. 变量映射 — 符号名 → 全局向量 X 的索引
3. 约束构建 — 每个 Expr::Eq 构建闭包 |X| → f64
**优先级**: 高（下步核心工作）
**位置**: `core/src/cluster.rs` 中新增 `ClusterBuilder`

---

## 跨项目

### 8. Python UI 层
**描述**: JSON-line IPC 的 Python 消费端实现，节点渲染、公式输入、状态可视化。
**当前状态**: 未开始，无代码。
**优先级**: 低（等 Rust 核心稳定后再做）

### 9. C/Math 库 FFI 调用
**描述**: 矩阵运算、微积分、PDE 求解的 C 库接入。
**当前状态**: 纯 Rust 原生 `Matrix` 已够用，暂时不需要。
**优先级**: 极低
