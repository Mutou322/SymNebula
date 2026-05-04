# SymNebula — 符号星云
# SymNebula — Symbol Nebula: Brain-like Mathematical Model Forge

> 一张活的、可自动重算的数学草稿纸。
> A living, auto-recomputing mathematical scratch paper.

**SymNebula**（符号星云）是一个纯粹、确定性的数学公式推演与模型动态运行工具。

它以人类大脑的神经元-突触网络为架构参考，以纯数学逻辑为唯一灵魂。用户通过创建"空神经元"、向其中输入自定义数学公式、并用"突触链接"将它们编织成星云图，即可让抽象的数学模型在计算机上演化、运行、被观察。

它不依赖任何人工智能、机器学习或智能辅助。它只是忠实执行数学逻辑的沙盒。

---

## 核心理念 | Core Philosophy

| 中文 | English |
|------|---------|
| **神经元 = 公式节点**，每个节点由用户自定义公式赋予功能 | **Neuron = Formula Node** — each node defined entirely by your formula |
| **突触 = 星云链接**，节点间的有向连接，传递数学量与约束 | **Synapse = Nebula Link** — directed connections carrying values and constraints |
| **星云 = 整个逻辑网络**，去中心化、可无限延伸 | **Nebula = The Logic Network** — decentralized, extensible web of references |
| **等式 = 数学平衡约束**，`=` 不是赋值，而是双向坍缩 | **Equation = Constraint** — `=` is a bidirectional equilibrium |
| **公式即源码**，数学公式本身就是驱动一切的源代码 | **Formula is Source Code** — the mathematics itself is the only source of truth |

---

## 技术架构 | Architecture

```
┌───────────────────────────────┐
│     Python UI 层              │  Render / Input / Observer
│     (纯渲染与输入)             │  Zero computation, display only
└─────────────┬─────────────────┘
              │  IPC (JSON-line)
┌─────────────▼─────────────────┐
│     Rust 核心调度层            │  Formula→AST, Graph, Tick, Guard
│     Core Engine                │
│                                │
│  solvers/                      │
│  ├── eval.rs   #[safe_solver] │
│  └── newton.rs  (代数+Bisect) │
│                                │
│  integrators/                  │
│  └── symplectic.rs #[safe_integrator] │
│                                │
│  guard/num.rs  数值合法性守卫  │  ensure_finite, safe_div, validate_outputs
│                                │
│  macros/ (proc-macro crate)    │  safe_solver, safe_integrator 属性宏
│  tests/crash_safety.rs         │  7个边界测试（除零/欠定/Purple隔离等）
└─────────────┬─────────────────┘
              │  FFI (future)
┌─────────────▼─────────────────┐
│     C / Math 库（纯计算层）    │  Matrix, calculus, PDE solvers
│     Pure Computation           │  Zero decisions, pure execution
└───────────────────────────────┘
```

### 工程安全五道防线

| 防线 | 机制 | 说明 |
|------|------|------|
| **两阶段 Tick** | Phase1 约束求解 → Phase2 时间推进 | dynamic 节点的约束与演化分离，互不干扰 |
| **Purple 隔离** | 上游 Purple → 下游 NoOp(Gray) | 异常不传播，出边 delay_buffer 清空 |
| **Partial 传播** | 上游 Yellow → 下游 Partial(PropagatedUncertainty) | 不确定性显式标记，不产生伪稳定 |
| **数值合法性** | `guard/num.rs` — ensure_finite, validate_outputs | 所有 Solver/Integrator 输出过检，NaN/Inf 拒绝 |
| **确定性排序** | `(priority, name)` 稳定排序 | 同优先级按名称字典序，消除不确定性 |

### 节点状态体系

| 颜色 | 状态 | 含义 |
|------|------|------|
| 灰 Gray | 静息 Idle | 待编辑或未激活 |
| 黄 Yellow | 待定 Pending | 多解待定 / 迭代中 / 上游不确定性传播 |
| 绿 Green | 有效 Stable | 计算成功，输出可传递 |
| 紫 Purple | 奇异 Singular | 数学异常（除零/NaN/奇异矩阵），已安全切断 |

### 传播规则

| 上游状态 | 下游行为 |
|----------|----------|
| Green | 正常计算 |
| Yellow | Partial(PropagatedUncertainty) |
| Purple | NoOp(Gray) — 整节点跳过 |
| Gray | 不参与计算 |

---

## 项目结构

```
sym-nebula/
├── core/                    # Rust 核心引擎
│   ├── src/
│   │   ├── engine.rs       # Tick 调度（两阶段）
│   │   ├── solver_trait.rs # Solver/Integrator trait + 管理器
│   │   ├── solvers/        # 求解器实现
│   │   │   ├── eval.rs     #   EvalSolver #[safe_solver(priority=100)]
│   │   │   └── newton.rs   #   NewtonSolver (代数+Newton+Bisection, p=200)
│   │   ├── integrators/    # 积分器实现
│   │   │   └── symplectic.rs  # SymplecticEuler #[safe_integrator]
│   │   ├── guard/           # 数值合法性守卫
│   │   └── ... (ast, graph, state, solver)
│   └── tests/
│       └── crash_safety.rs  # 崩溃边界测试（7个）
├── macros/                 # proc-macro crate（独立）
│   └── src/lib.rs          # safe_solver + safe_integrator 属性宏
├── docs/                   # 文档
│   ├── ARCHITECTURE.md     # 核心架构总览
│   ├── TICK_FLOW.md        # Tick 动态演化详解
│   └── SAFETY.md           # 安全护栏参考 & proc-macro 用法
└── README.md               # 本文件
```

---

## 核心特性 | Key Features

| 特性 | English | 说明 |
|------|---------|------|
| **完全自定义** | Fully Custom | 无内置公式库，从 1+1=2 到爱因斯坦场方程，全部由用户定义 |
| **双向约束求解** | Bidirectional Solving | 代数求解优先 → Newton 数值迭代 → Bisection fallback 三级降级 |
| **逻辑时钟驱动** | Tick-Based Evolution | 离散时间步，Compute → Integrator → Commit 三阶段 |
| **符号作用域隔离** | Isolated Namespaces | 每个节点独立命名空间，λ 可同时表示波长和特征值 |
| **奇异点安全隔离** | Singularity Isolation | 1/0、NaN、奇异矩阵自动标紫并切断输出，保护整体 |
| **proc-macro 安全护栏** | Macro Safety Guards | `#[safe_solver]` / `#[safe_integrator]` 自动注入 catch_unwind + validate_outputs |
| **时钟-UI 分离** | Decoupled Clock & UI | 后台全速计算，界面独立采样，永不卡帧 |

---

## 快速开始 | Quick Start

```bash
# 运行测试（50个，含7个崩溃安全测试）
cd ~/sym-nebula/core && cargo test

# 运行 mvp 演示
cargo run --bin mvp

# 运行轨道模拟
cargo run --bin orbit

# 运行压力测试
cargo run --bin stress
```

### 测试结果

| 测试 | 结果 |
|------|------|
| 单元测试 | 43/43 ✅ |
| 崩溃安全测试 | 7/7 ✅ |
| 总计 | 50/50 ✅ |
| 编译 warning | 1（未使用变量） |

---

## 开源协议 | License

本项目采用 **GNU General Public License v3.0** 协议开源。

This project is licensed under the **GNU General Public License v3.0**.

Copyright (c) 2026 NebulaMind
