# SymNebula Tick Flow — ClusterSolver v4

> ClusterSolver v4 的完整 Tick 流水线设计说明书。
> 包含流程图、状态演化波形、关键设计规则。

---

## 1. 完整流水线

```
Legend:
  G = Green ✅ (收敛成功)
  Y = Yellow ⚠️ (部分收敛 / 欠定)
  P = Purple ❌ (失败 / 回滚)
  [C] = Commit 原子提交

                           ┌─────────────────────────────┐
                           │ Phase 0: Snapshot           │
                           │ frozen_graph = graph        │
                           │ topology_version = graph.topology_version │
                           └─────────────┬───────────────┘
                                         │
                                         ▼
                           ┌─────────────────────────────┐
                           │ Phase 1: Cluster Detection  │
                           │  Check ClusterCache.version │
                           │      ┌───────────────┐      │
                           │      │Version same?  │──Yes─▶ Use cached clusters
                           │      └──────┬────────┘      │
                           │             ▼               │
                           │      No ──▶ Compute clusters│
                           │             │               │
                           │             ▼               │
                           │      Update ClusterCache    │
                           └─────────────┬───────────────┘
                                         │
                                         ▼
          ┌───────────────────────────────────────────────────────────┐
          │ Phase 2: Single Node Solver (isolated nodes)              │
          │  For each node not in any cluster:                        │
          │     Solve → node.next_buffer + Green/Yellow/Purple       │
          └─────────────┬─────────────────────────────────────────────┘
                        │
                        ▼
          ┌───────────────────────────────────────────────────────────┐
          │ Phase 3: ClusterSolver (per cluster)                       │
          │                                                             │
          │  ① Compile: Union-Find → Rewrite → SCC                     │
          │  ② Solve: Block Newton on temp_x (Tick内多步迭代)          │
          │  ③ Judge: 全块收敛→Green, 奇异→Purple, 未收敛→Yellow     │
          │  ④ Commit: Green→write cluster_xs, Y/P→保留旧值            │
          └─────────────┬─────────────────────────────────────────────┘
                        │
                        ▼
                           ┌─────────────────────────────┐
                           │ Phase 4: Buffer Swap        │
                           │ all_nodes values + status   │
                           │ become active for next Tick │
                           └─────────────────────────────┘
```

---

## 2. 连续 Tick 状态演化波形

```
Ticks →       t0        t1        t2        t3        t4
-------------------------------------------------------------------
Cluster A:
  X_cluster  X░░░ →   X░░░ →   X███ →   X███ →   X███
  Status     ▓▓▓ →   ▓▓▓ →   ███ →   ███ →   ███
  Commit              [C]      [C]      [C]      [C]

Cluster B:
  X_cluster  X░░░ →   X░░░ →   X░░░ →   X███ →   X███
  Status     ▓▓▓ →   ▓▓▓ →   ▓▓▓ →   ███ →   ███
  Commit              [C]      [C]      [C]      [C]

Cluster C:
  X_cluster  X░░░ →   X░░░ →   X░░░ →   X░░░ →   X░░░
  Status     ░░░ →   ░░░ →   ░░░ →   ░░░ →   ░░░
  Commit              [C]      [C]      [C]      [C]  (rollback)

Cluster D:
  X_cluster  X░░░ →   X░░░ →   X░░░ →   X███ →   X███
  Status     ▓▓▓ →   ░░░ →   ▓▓▓ →   ███ →   ███
  Commit              [C]      [C]      [C]      [C]

Node1 (Single):
  X_single   X░░ →   X░░ →   X██ →   X██ →   X██
  Status     ▓▓ →   ▓▓ →   ██ →   ██ →   ██
  Commit              [C]     [C]     [C]     [C]

Node3 (Single):
  X_single   X░░ →   X░░ →   X░░ →   X░░ →   X░░
  Status     ░░ →   ░░ →   ░░ →   ░░ →   ░░
  Commit              [C]     [C]     [C]     [C]  (rollback)

Legend:
  █ = Green ✅ (收敛成功)
  ▓ = Yellow ⚠️ (部分收敛 / 欠定)
  ░ = Purple ❌ (失败 / 回滚)
  X░░/X██ = X_cluster/X_single 临时迭代值变化
  [C] = Commit 原子写入 next_buffer
  → = Tick 内迭代推进
  | = Tick 时间推进
```

---

## 3. 连续 Tick 收敛说明

### 3.1 逐步逼近
- Yellow ⚠️ 节点残差逐步降低，多 Tick 后 → Green ✅
- 每 Tick 只推进一步，符合"每 Tick 推进一步"设计哲学
- Yellow 保留旧值供下游，同时 X_cluster 保留迭代值供下 Tick 初值

### 3.2 隔离回滚
- Purple ❌ 节点完全不写入 next_buffer，保持数学自洽
- 下游看到上一 Tick 的合法值（滞后一步）
- 一个集群 Purple 不阻塞其他集群 Green/Yellow 迭代

### 3.3 独立集群
- 集群间变量互斥，求解过程完全隔离
- 原子提交保证全局一致性
- 多集群可并行求解

### 3.4 增量稳定
- topology_version 不变时 ClusterCache 复用，零成本
- 拓扑固定时集群划分完全确定性
- 数值/状态变化不影响集群划分

---

## 4. 关键设计规则

| 规则 | 说明 |
|------|------|
| Cluster ≠ SCC | Cluster = 物理耦合域, SCC = 数学求解分块 |
| UF 只在 cluster 内 | 不跨 cluster 合并变量 |
| X_cluster 隔离 | 求解期间唯一真值源，不改 graph/node state |
| 原子提交 | temp_x 克隆，成功后 copy_from_slice，失败保留旧值 |
| 拓扑版本号 | add_node/add_edge 递增，状态变化不递增 |
| Tick 内多步迭代 | 每个 block 反复 Newton 直至收敛或 max_iter |
| 紫色隔离 | 上游 Purple → 下游 NoOp(Gray)，不传播数值 |

---

## 5. 连续 Tick 状态传播规则

### 传播规则表

| 上游状态 | 下游行为 |
|----------|----------|
| Green ✅ | 正常计算，使用上游输出值 |
| Yellow ⚠️ | 下游仍收到上一 Tick 的旧值，暂缓推进 |
| Purple ❌ | 下游返回 NoOp(Gray)，切断传播 |

### Yellow 状态持续性

- 当前 Tick: X_cluster 在 temp_x 上求解，未收敛 → Yellow
- cluster_xs 保留旧值（不写 temp_x）
- temp_x 的当前值**被丢弃**
- 下 Tick 的 X_cluster 从 cluster_xs（旧值）重新出发
- 注意：当前实现中 Yellow 不保留 temp_x 作为初值，每次从旧值出发

---

## 6. 工程安全

- ensure_finite 守卫所有数值输出
- 奇异 Jacobian → Err 不 panic
- m < n 欠定跳过不报错
- Purple/Yellow 原子不污染全局状态

---

## 7. 已知限制

- 仅支持 Expr::Eq 约束，非 Eq 公式不参与
- 单节点 Solver 分流未实现（Phase 2 应走 SolverManager）
- Tick 集成到 engine.rs 未完成
- 符号求导未实现（Expr::derive → 精确 Jacobian）
- Yellow 状态的 X_cluster 迭代值未跨 Tick 保持（每次从旧值出发）
