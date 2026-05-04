# SymNebula 核心架构

## 总览

```
┌───────────────────────────────────────────────┐
│                Python UI 层                  │
│  Render / Input / Observer Window            │
│  - 用户输入公式                              │
│  - 可视化节点状态（灰/黄/绿/紫）              │
│  - 投影高维数据到 2D/3D 渲染                 │
│                                               │
│  ⚠️ 绝不做计算或逻辑判断                      │
└─────────────────────────────┬─────────────────┘
                              │ IPC (JSON-line)
                              ▼
┌─────────────────────────────┐
│        Rust 核心调度层      │
│        Core Engine          │
│                             │
│  Graph / Node 管理          │
│  - 节点符号作用域隔离       │
│  - 星云拓扑 & 依赖分析      │
│                             │
│  Tick 流 & 状态机           │
│  - 每个 Tick:               │
│      1. 构建 LocalContext 快照 │
│      2. ClusterSolver 分组求解│
│      3. Integrator 数值步进  │
│      4. safe_outputs 检查 & 输出│
│      5. commit 到节点状态     │
│                             │
│  宏护栏 / proc-macro         │
│  - safe_solver / safe_integrator │
│  - 自动捕获 panic & 异常      │
│  - 数值合法性检查             │
│                             │
│  Solver & Integrator trait   │
│  - 用户自定义公式节点驱动     │
│  - 内核仅调用接口执行        │
└─────────────┬─────────────────┘
              │
              ▼
┌─────────────────────────────┐
│      C / Math 库（纯计算）   │
│  - 矩阵运算、微积分          │
│  - 零决策、零状态机          │
│  - 性能极致优化              │
└─────────────────────────────┘
```

## 核心数据流

用户公式输入 → Python UI → Rust 内核

每个 Node 由用户公式定义，输入变量生成突触接口（自动接线柱）。

```
for cluster in cluster_solver {
    let local_ctx = LocalContext::from_node_and_graph(cluster, graph);
    let outputs = cluster.solve_cluster(&local_ctx);
    node.write_next(safe_outputs(outputs));
}
commit_all_nodes();
```

## Tick 流：Compute → Integrator → Commit

```
┌───────────────────────────────┐
│       Rust 内核 Core Engine   │
│                               │
│  ┌─────────────────────────┐ │
│  │ 1️⃣ Compute 阶段         │ │
│  │ - 构建 LocalContext 快照│ │
│  │ - ClusterSolver 分组求解  │ │
│  │   ┌─────────────────┐   │ │
│  │   │ 用户自定义 Solver│   │ │
│  │   │ solve_impl()    │   │ │
│  │   │ (宏护栏包裹)     │   │ │
│  │   │  safe_solver    │   │ │
│  │   └─────────────────┘   │ │
│  │                         │ │
│  │    SolveResult (绿/黄/紫) │ │
│  └───────────┬─────────────┘ │
│              │               │
│  ┌───────────┴─────────────┐ │
│  │ 2️⃣ Integrator 阶段      │ │
│  │ - 动态演化 (半隐式欧拉)  │ │
│  │ - 输出经过 validate_outputs│ │
│  │ - 上游 Purple 检测      │ │
│  └───────────┬─────────────┘ │
│              │               │
│  ┌───────────┴─────────────┐ │
│  │ 3️⃣ Commit 阶段          │ │
│  │ - 写入 next_buffer      │ │
│  │ - 更新节点状态           │ │
│  │ - Purple 清空出边        │ │
│  └─────────────────────────┘ │
└───────────────────────────────┘
```

## 异常 / 多解流

```
上游节点输出 ↓
    ┌─────────────┐
    │ Purple (紫) │───── 下游节点 Gray(NoOp) → 输出切断
    └─────────────┘

    ┌─────────────┐
    │ Yellow (黄) │───── 下游节点 Yellow(PropagatedUncertainty)
    └─────────────┘
         ↑
   多解 / 欠定 / 迭代中
```

## 节点状态颜色

| 状态  | 颜色 | 含义                     |
|-------|------|--------------------------|
| Grey  | 灰   | 待编辑 / 未激活           |
| Yellow| 黄   | 多解待定 / 迭代中          |
| Green | 绿   | 计算成功 / 输出有效        |
| Purple| 紫   | 数学异常 / 已安全切断      |

## 安全护栏（五道防线）

1. **两阶段 Tick** — Phase 1 约束求解 → Phase 2 时间推进
2. **Purple 隔离** — 上游 Purple → 下游 NoOp，出边 delay_buffer 清空
3. **Partial 传播** — 上游 Yellow → 下游 Partial(PropagatedUncertainty)
4. **数值合法性** — 所有输出过 `validate_outputs`，NaN/Inf 拒绝
5. **确定性排序** — Solver 按 `(priority, name)` 稳定排序
