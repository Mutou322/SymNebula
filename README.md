# SymNebula — 符号星云
# SymNebula — Symbol Nebula: Brain-like Mathematical Model Forge

> 一张活的、可自动重算的数学草稿纸。
> A living, auto-recomputing mathematical scratch paper.

**SymNebula**（符号星云）是一个纯粹、确定性的数学公式推演与模型动态运行工具。

**SymNebula** is a purely deterministic mathematical formula deduction and dynamic model simulation tool.

它以人类大脑的神经元-突触网络为架构参考，以纯数学逻辑为唯一灵魂。用户通过创建"空神经元"、向其中输入自定义数学公式、并用"突触链接"将它们编织成星云图，即可让抽象的数学模型在计算机上演化、运行、被观察。

Inspired by the neuron-synapse network of the human brain, driven by pure mathematical logic — with zero AI, zero machine learning, zero smart assistance. Users create empty "neurons", fill them with custom mathematical formulas, and weave them into a nebula graph with "synapse links", allowing abstract mathematical models to evolve, run, and be observed.

它不依赖任何人工智能、机器学习或智能辅助。它只是忠实执行数学逻辑的沙盒。

It is not an AI assistant. It is a faithful sandbox for mathematical logic.

---

## 核心理念 | Core Philosophy

| 中文 | English |
|------|---------|
| **神经元 = 公式节点**，每个节点由用户自定义公式赋予功能，无预设 | **Neuron = Formula Node** — each node is an empty vessel, defined entirely by the formula you write into it |
| **突触 = 星云链接**，节点间的有向连接，传递数学量与约束 | **Synapse = Nebula Link** — directed connections that carry mathematical values and constraints between nodes |
| **星云 = 整个逻辑网络**，去中心化、可无限延伸的引用网 | **Nebula = The Logic Network** — decentralized, infinitely extensible web of mathematical references |
| **等式 = 数学平衡约束**，`=` 不是赋值，而是双向坍缩 | **Equation = Constraint** — `=` is not assignment, it is a bidirectional equilibrium |
| **公式即源码**，数学公式本身就是驱动一切的源代码 | **Formula is Source Code** — the mathematics itself is the only source of truth |

---

## 技术架构 | Architecture

```
┌─────────────────────────────┐
│   Python（纯 UI 层）        │  ← 渲染、输入、观察窗 | Render, input, observer
│   Pure UI Layer             │
└─────────────┬───────────────┘
              │  IPC (JSON-line)
┌─────────────▼───────────────┐
│   Rust（核心调度引擎）      │  ← 解析公式、管理图、调度计算
│   Core Engine               │  Parse formulas, manage graph, schedule
└─────────────┬───────────────┘
              │  FFI
┌─────────────▼───────────────┐
│   C / Math Library（计算层）│  ← 纯数值运算，无决策逻辑
│   Pure Computation           │  Matrix ops, calculus, solvers
└─────────────────────────────┘
```

| 层 | Layer | 职责 | Responsibility |
|----|-------|------|----------------|
| **Python** | Skin & Senses | 渲染星云图、处理键鼠、驱动观察窗。不参与任何逻辑或计算 | Render the node graph, handle input, drive the observer window. Zero logic or computation |
| **Rust** | Brain | 管理图结构、解析公式为 AST、控制逻辑时钟、调度并行计算、状态机、文件管理 | Graph management, formula → AST, logical clock, parallel scheduling, state machine, file I/O |
| **C/数学库** | Muscles | 矩阵运算、微积分、数值积分、微分方程求解。纯执行，零决策 | Matrix operations, calculus, numerical integration, PDE solving. Execute only, no decisions |

---

## 核心特性 | Key Features

| 特性 | English | 说明 |
|------|---------|------|
| **完全自定义** | Fully Custom | 无内置公式库，从 1+1=2 到爱因斯坦场方程，全部由用户定义 |
| **双向约束求解** | Bidirectional Solving | 等式自动双向坍缩，已知 x 则求 y，反之亦然 |
| **逻辑时钟驱动** | Tick-Based Evolution | 离散时间步支持反馈回路、振荡、波动等动态演化 |
| **惰性求值** | Lazy Evaluation | 仅激活被观察或相关支路，极大降低消耗 |
| **符号作用域隔离** | Isolated Namespaces | 每个节点独立命名空间，λ 可同时表示波长和特征值 |
| **奇异点安全隔离** | Singularity Isolation | 1/0、维度不匹配自动标紫并切断输出，保护整体 |
| **时钟-UI 分离** | Decoupled Clock & UI | 后台全速计算，界面独立采样，永不卡帧 |
| **跨平台便携** | USB Portable | 可装入 U 盘，无需安装，跨平台直接运行 |

---

## 工程文件格式 | File Format

| 项目 | Item | 内容 |
|------|------|------|
| 格式 | Format | XML + ZIP，后缀 `.brainstorm` |
| 内容 | Contents | 节点、公式 AST、链接拓扑、常量、状态快照 |
| 安全 | Safety | 加载时自动拓扑扫描，预判递归深度，防资源耗尽 |

---

## 节点状态体系 | Node States

| 颜色 | Color | 状态 | State | 含义 |
|------|-------|------|-------|------|
| 灰 | Grey | 静息 | Idle | 待编辑或未激活 |
| 黄 | Yellow | 待定 | Pending | 计算中或多解等待限定 |
| 绿 | Green | 有效 | Stable | 计算成功，输出可传递 |
| 紫 | Purple | 奇异 | Singular | 数学异常（除零/维度不匹配），已安全切断 |

---

## 开源协议 | License

本项目采用 **GNU General Public License v3.0** 协议开源。

This project is licensed under the **GNU General Public License v3.0**.

Copyright (c) 2026 NebulaMind

任何人使用、修改、分发本代码，必须同样以 GPL v3 协议开源。

Anyone who uses, modifies, or distributes this code must also open-source it under GPL v3.
