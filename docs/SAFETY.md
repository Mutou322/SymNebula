# 安全护栏参考

## 五道工程安全防线

### 1. 两阶段 Tick（Phase 1 → Phase 2）

```
engine.rs: step()
├── Phase 1: 约束求解（非 dynamic 节点）
│   └── SolverManager.solve_node()
│         └── 上游 Purple? → NoOp
│         └── 上游 Yellow? → Partial(PropagatedUncertainty)
│         └── 正常 → 选 Solver 求解
│
├── Phase 2: 时间推进（dynamic 节点）
│   └── IntegratorManager.step_node()
│         └── 上游 Purple? → NoOp
│         └── 上游 Yellow? → Partial(PropagatedUncertainty)
│         └── 正常 → 积分步进
│
└── Commit
      ├── Green/Yellow → 写入 delay_buffer
      └── Purple → 清空出边 delay_buffer
```

### 2. Purple 隔离（硬切断）

```rust
// engine.rs
fn has_upstream_purple(&self, node_id: usize) -> bool {
    self.graph.edges.iter().any(|e| {
        e.to_node == node_id
            && self.graph.nodes.iter()
                .any(|n| n.id == e.from_node && n.state == NodeState::Purple)
    })
}
```

上游 Purple → 下游返回 `NoOp(Gray)`，整节点跳过。

### 3. Partial 传播（黄色传染）

```rust
if self.has_upstream_partial(node_id) {
    let known_vals = self.graph.get_inputs_for_node(node_id);
    let result = SolveResult::partial(
        known_vals,
        PartialReason::PropagatedUncertainty,
    );
    emit_result(node_id, &result, ...);
    continue;
}
```

上游 Yellow → 下游返回 `Partial(PropagatedUncertainty)`。

### 4. 数值合法性守卫

| 函数 | 作用 | 返回 |
|------|------|------|
| `ensure_finite(v)` | 检查 NaN/Inf | `Result<f64, &str>` |
| `ensure_nonzero(v)` | 检查除零 | `Result<f64, &str>` |
| `safe_div(a, b)` | 安全除法 | `Result<f64, &str>` |
| `safe_sqrt(x)` | 安全开方 | `Result<f64, &str>` |
| `validate_outputs(map)` | 批量验证 | `Result<(), &str>` |

### 5. 确定性 Solver 排序

```
SolverManager::solve_node():
  1. 收集所有 supports() 的 Solver → (solver, priority, name)
  2. 按 (priority, name) 字典序排序
  3. 取第一个执行
```

## 传播规则表

| 上游状态 | 下游行为 |
|----------|----------|
| Green    | 正常计算 |
| Yellow   | 返回 Partial(PropagatedUncertainty) |
| Purple   | 返回 NoOp(Gray) |
| Gray     | 跳过（不参与计算） |

## proc-macro 使用

### `#[safe_solver]`

```rust
use symnebula_macros::safe_solver;

#[safe_solver(priority = 100, name = "eval")]
impl Solver for MySolver {
    fn supports(&self, node: &Node) -> bool {
        // 匹配条件
    }

    fn solve(&self, node: &Node, ctx: &HashMap<String, f64>)
        -> Result<HashMap<String, f64>, &'static str>
    {
        // 纯数学逻辑，不操心安全
        let mut out = HashMap::new();
        out.insert("output".to_string(), 42.0);
        Ok(out)
    }
}
```

宏自动注入：
- `catch_unwind` 防止 panic
- `validate_outputs` 数值合法性
- `name()` 和 `priority()`

### `#[safe_integrator]`

```rust
use symnebula_macros::safe_integrator;

#[safe_integrator(name = "symplectic_euler")]
impl Integrator for MyIntegrator {
    fn step(&self, node: &Node, ctx: &HashMap<String, f64>, dt: f64)
        -> Result<HashMap<String, f64>, &'static str>
    {
        // 纯数学逻辑
        Ok(map)
    }
}
```

## JSON-line IPC 协议（Python ↔ Rust）

保留接口定义（未来实现时填充）：

```
> {"type": "add_node", "formula": "x + y = 10", "id": 0}
< {"type": "node_added", "id": 0, "state": "gray"}

> {"type": "tick", "n": 1}
< {"type": "tick_result", "nodes": [
      {"id": 0, "state": "green", "outputs": {"x": 3.0, "y": 7.0}}
  ]}

> {"type": "get_graph"}
< {"type": "graph", "nodes": [...], "edges": [...]}
```
