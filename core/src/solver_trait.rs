/// 求解器模块化接口 + 管理器
///
/// Solver trait 将求解逻辑与 Tick/Graph/Engine 完全解耦。
/// 内核只负责调度，不负责"怎么解方程"。
///
/// 架构：
///   SolverManager → 按优先级排序 → Supports() → solve() → SolveResult
///
/// 具体 Solver 实现在 solvers/ 子模块中：
///   - solvers/eval.rs:   EvalSolver（纯表达式，优先级 100）
///   - solvers/newton.rs: NewtonSolver（等式/代数+Newton，优先级 200）

use std::collections::HashMap;

use crate::graph::Node;
use crate::state::NodeState;

// ============================================================
// PartialReason — 未完全收敛的原因
// ============================================================

/// Partial 状态的细化原因
#[derive(Debug, Clone, PartialEq)]
pub enum PartialReason {
    /// 迭代未收敛（Newton 还在跑）
    NotConverged,
    /// 多解待定（需要用户提供额外约束）
    MultipleSolutions,
    /// 欠定（方程数 < 变量数）
    Underdetermined,
    /// 过定（方程数 > 变量数）
    Overdetermined,
    /// 来自上游 Yellow 节点的传播不确定性
    PropagatedUncertainty,
}

// ============================================================
// SolveResult — 统一求解输出
// ============================================================

/// 求解器的统一输出。
///
/// 每个变体严格映射到一种节点状态（绿/黄/紫/灰）：
///   Converged → Green
///   Partial   → Yellow
///   Failed    → Purple
///   NoOp      → Gray
#[derive(Debug, Clone)]
pub enum SolveResult {
    /// 完全收敛，携带输出值 (symbol -> value)
    Converged(HashMap<String, f64>),
    /// 部分收敛或未完全收敛（标黄）
    Partial {
        values: HashMap<String, f64>,
        reason: PartialReason,
    },
    /// 求解失败（标紫）
    Failed(String),
    /// 无操作（标灰）
    NoOp,
}

impl SolveResult {
    /// 提取输出值
    pub fn values(&self) -> HashMap<String, f64> {
        match self {
            SolveResult::Converged(m) => m.clone(),
            SolveResult::Partial { values, .. } => values.clone(),
            _ => HashMap::new(),
        }
    }

    /// 映射到节点状态
    pub fn node_state(&self) -> NodeState {
        match self {
            SolveResult::Converged(_) => NodeState::Green,
            SolveResult::Partial { .. } => NodeState::Yellow,
            SolveResult::Failed(_) => NodeState::Purple,
            SolveResult::NoOp => NodeState::Gray,
        }
    }

    /// 快捷构造 Partial
    pub fn partial(values: HashMap<String, f64>, reason: PartialReason) -> Self {
        SolveResult::Partial { values, reason }
    }
}

// ============================================================
// Solver trait
// ============================================================

/// 抽象求解器接口。
///
/// 实现此 trait 的结构体可替换求解策略，不依赖 Tick/Graph/Engine。
///
/// priority() 控制多个 Solver 同时 supports 时的执行顺序。
/// 数值越小优先级越高。建议区间：
///   0-50   保留给特殊/紧急求解器
///   51-100 动态/物理求解器
///   101-200 通用数值求解器
///   201+   通用代数求解器
pub trait Solver {
    /// 求解器名称，用于调试和确定性排序
    fn name(&self) -> &'static str;

    /// 优先级，数值越小越优先。默认 128。
    fn priority(&self) -> u8 {
        128
    }

    /// 是否支持该节点
    fn supports(&self, node: &Node) -> bool;

    /// 执行一次求解。
    /// ctx: 节点输入端口的值（来自 delay_buffer）
    fn solve(&self, node: &Node, ctx: &HashMap<String, f64>) -> SolveResult;
}

// ============================================================
// SolverManager
// ============================================================

/// 求解器管理器。
///
/// 持有一组 Solver，按优先级排序后遍历匹配执行。
/// 内核只需调用 solve_node，不关心内部选用了哪个求解器。
pub struct SolverManager {
    solvers: Vec<Box<dyn Solver>>,
}

impl SolverManager {
    pub fn new(solvers: Vec<Box<dyn Solver>>) -> Self {
        SolverManager { solvers }
    }

    /// 按优先级（+名称确定性排序）找到第一个 supports 的并执行。
    pub fn solve_node(&self, node: &Node, ctx: &HashMap<String, f64>) -> SolveResult {
        // 找出所有匹配的求解器并按 (priority, name) 稳定排序
        let mut candidates: Vec<(&Box<dyn Solver>, u8, &'static str)> = Vec::new();
        for solver in &self.solvers {
            if solver.supports(node) {
                candidates.push((solver, solver.priority(), solver.name()));
            }
        }

        // 按 priority 升序，同优先级按 name 字典序
        candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(b.2)));

        match candidates.into_iter().next() {
            Some((solver, _, _)) => solver.solve(node, ctx),
            None => SolveResult::NoOp,
        }
    }
}

// ============================================================
// Integrator trait — 时间推进（非求解）
// ============================================================

/// 时间推进器接口。
///
/// 与 Solver 的区别：
///   Solver      解 F(x) = 0，输出稳态值
///   Integrator  做 state(t + dt)，输出下一时刻值
///
/// Dynamic 节点（位置、速度等）应使用 Integrator，不是 Solver。
pub trait Integrator {
    /// 执行一步时间推进。
    fn step(&self, node: &Node, ctx: &HashMap<String, f64>, dt: f64) -> SolveResult;
}

// ============================================================
// IntegratorManager
// ============================================================

/// 积分器管理器。
pub struct IntegratorManager {
    integrators: Vec<Box<dyn Integrator>>,
}

impl IntegratorManager {
    pub fn new(integrators: Vec<Box<dyn Integrator>>) -> Self {
        IntegratorManager { integrators }
    }

    pub fn step_node(&self, node: &Node, ctx: &HashMap<String, f64>, dt: f64) -> SolveResult {
        for integrator in &self.integrators {
            let result = integrator.step(node, ctx, dt);
            return result;
        }
        SolveResult::NoOp
    }
}

// ============================================================
// 默认管理器构造
// ============================================================

/// 创建默认求解器管理器（EvalSolver + NewtonSolver）
pub fn default_solver_manager() -> SolverManager {
    SolverManager::new(vec![
        Box::new(crate::solvers::EvalSolver::new()),
        Box::new(crate::solvers::NewtonSolver::new()),
    ])
}

/// 创建默认积分器管理器（SymplecticEuler）
pub fn default_integrator_manager() -> IntegratorManager {
    IntegratorManager::new(vec![Box::new(
        crate::integrators::symplectic::SymplecticEulerIntegrator::new(),
    )])
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Expr;
    use crate::graph::Node;

    // --- SolveResult ---

    #[test]
    fn test_solve_result_node_state_mapping() {
        let m = HashMap::new();
        assert_eq!(
            SolveResult::Converged(m.clone()).node_state(),
            NodeState::Green
        );
        assert_eq!(
            SolveResult::partial(m.clone(), PartialReason::NotConverged).node_state(),
            NodeState::Yellow
        );
        assert_eq!(
            SolveResult::Failed("err".into()).node_state(),
            NodeState::Purple
        );
        assert_eq!(SolveResult::NoOp.node_state(), NodeState::Gray);
    }

    // --- SolverManager 优先级 ---

    struct PriorityTestSolver {
        p: u8,
        name: &'static str,
    }
    impl Solver for PriorityTestSolver {
        fn name(&self) -> &'static str {
            self.name
        }
        fn priority(&self) -> u8 {
            self.p
        }
        fn supports(&self, _node: &Node) -> bool {
            true
        }
        fn solve(&self, _node: &Node, _ctx: &HashMap<String, f64>) -> SolveResult {
            let mut map = HashMap::new();
            map.insert(self.name.to_string(), self.p as f64);
            SolveResult::Converged(map)
        }
    }

    #[test]
    fn test_solver_priority() {
        let mgr = SolverManager::new(vec![
            Box::new(PriorityTestSolver {
                p: 200,
                name: "low",
            }),
            Box::new(PriorityTestSolver {
                p: 50,
                name: "high",
            }),
        ]);

        let node = Node {
            id: 0,
            formula: Expr::Number(0.0),
            state: NodeState::Gray,
            value: None,
            solve_target: None,
            is_dynamic: false,
        };
        let ctx = HashMap::new();
        let result = mgr.solve_node(&node, &ctx);
        match result {
            SolveResult::Converged(map) => {
                // 优先级 50 的 high 应该优先生效
                assert!(map.contains_key("high"), "高优先级应优先");
                assert!(!map.contains_key("low"), "低优先级不应执行");
            }
            _ => panic!("期望 Converged"),
        }
    }

    #[test]
    fn test_solver_priority_deterministic_tie_break() {
        // 同优先级按名称字典序：alpha < beta
        let mgr = SolverManager::new(vec![
            Box::new(PriorityTestSolver {
                p: 100,
                name: "beta",
            }),
            Box::new(PriorityTestSolver {
                p: 100,
                name: "alpha",
            }),
        ]);

        let node = Node {
            id: 0,
            formula: Expr::Number(0.0),
            state: NodeState::Gray,
            value: None,
            solve_target: None,
            is_dynamic: false,
        };
        let ctx = HashMap::new();
        let result = mgr.solve_node(&node, &ctx);
        match result {
            SolveResult::Converged(map) => {
                // "alpha" 字典序小于 "beta"，应优先
                assert!(map.contains_key("alpha"), "alpha 应在 beta 之前");
            }
            _ => panic!("期望 Converged"),
        }
    }
}
