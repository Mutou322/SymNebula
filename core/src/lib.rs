/// NebulaMind 核心库
///
/// 纯粹、确定性的数学公式推演与模型动态运行工具。
/// 零外部依赖，仅使用 Rust 标准库。
///
/// 四个核心模块：
///   - ast:     表达式语法树与极简解析器
///   - state:   节点状态机（灰/黄/绿/紫）
///   - graph:   星云图数据结构（神经元-突触）
///   - solver:  约束求解器与双向坍缩
///   - engine:  逻辑时钟调度器

pub mod ast;
pub mod state;
pub mod graph;
pub mod solver;
pub mod engine;
pub mod solver_trait;
pub mod integrators;
pub mod guard;
pub mod solvers;
