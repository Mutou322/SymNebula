/// 求解器模块化接口 + 标准实现
///
/// Solver trait 将求解逻辑与 Tick/Graph/Engine 完全解耦。
/// 内核只负责调度，不负责"怎么解方程"。
///
/// 架构：
///   SolverManager → 遍历 Solver → supports() → solve() → SolveResult
///
/// 内置 Solver：
///   - EvalSolver:    纯表达式求值（无等号）
///   - NewtonSolver:  等式节点，代数求解 + Newton 降级

use std::cell::RefCell;
use std::collections::HashMap;

use crate::ast::Expr;
use crate::graph::Node;
use crate::solver::{solve_eq, solver_step, make_eq_function};
use crate::state::{NodeState, SolverState};

// ============================================================
// SolveResult — 统一求解输出
// ============================================================

/// 求解器的统一输出。
#[derive(Debug, Clone)]
pub enum SolveResult {
    /// 完全收敛，携带输出值 (symbol -> value)
    Converged(HashMap<String, f64>),
    /// 部分收敛或未完全收敛（标黄）
    Partial(HashMap<String, f64>),
    /// 求解失败（标紫）
    Failed(String),
    /// 无操作（标灰）
    NoOp,
}

impl SolveResult {
    /// 提取输出值
    pub fn values(&self) -> HashMap<String, f64> {
        match self {
            SolveResult::Converged(m) | SolveResult::Partial(m) => m.clone(),
            _ => HashMap::new(),
        }
    }

    /// 映射到节点状态
    pub fn node_state(&self) -> NodeState {
        match self {
            SolveResult::Converged(_) => NodeState::Green,
            SolveResult::Partial(_) => NodeState::Yellow,
            SolveResult::Failed(_) => NodeState::Purple,
            SolveResult::NoOp => NodeState::Gray,
        }
    }
}

// ============================================================
// Solver trait
// ============================================================

/// 抽象求解器接口。
///
/// 实现此 trait 的结构体可替换求解策略，不依赖 Tick/Graph/Engine。
pub trait Solver {
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
/// 持有一组 Solver，遍历匹配执行。
/// 内核只需调用 solve_node，不关心内部选用了哪个求解器。
pub struct SolverManager {
    solvers: Vec<Box<dyn Solver>>,
}

impl SolverManager {
    pub fn new(solvers: Vec<Box<dyn Solver>>) -> Self {
        SolverManager { solvers }
    }

    /// 遍历所有 Solver，找到第一个 supports 的并执行。
    pub fn solve_node(&self, node: &Node, ctx: &HashMap<String, f64>) -> SolveResult {
        for solver in &self.solvers {
            if solver.supports(node) {
                return solver.solve(node, ctx);
            }
        }
        SolveResult::NoOp
    }
}

// ============================================================
// EvalSolver — 纯表达式求值
// ============================================================

/// 纯表达式求解器。
///
/// 处理所有无等号的节点（Number、纯表达式）。
pub struct EvalSolver;

impl EvalSolver {
    pub fn new() -> Self {
        EvalSolver
    }
}

impl Solver for EvalSolver {
    fn supports(&self, node: &Node) -> bool {
        !matches!(node.formula, Expr::Eq(_, _))
    }

    fn solve(&self, node: &Node, ctx: &HashMap<String, f64>) -> SolveResult {
        match &node.formula {
            Expr::Number(n) => {
                let mut map = HashMap::new();
                map.insert("output".to_string(), *n);
                SolveResult::Converged(map)
            }
            expr => match expr.eval(ctx) {
                Ok(val) => {
                    let mut map = HashMap::new();
                    map.insert("output".to_string(), val);
                    SolveResult::Converged(map)
                }
                Err(e) => SolveResult::Failed(e),
            },
        }
    }
}

// ============================================================
// NewtonSolver — 等式求解器
// ============================================================

/// 等式求解器。
///
/// 优先代数求解（solve_eq），失败则降级到单变量 Newton。
/// 内部持有每个节点的 SolverState，独立维护迭代状态。
pub struct NewtonSolver {
    /// node_id -> SolverState
    states: RefCell<HashMap<usize, SolverState>>,
}

impl NewtonSolver {
    pub fn new() -> Self {
        NewtonSolver {
            states: RefCell::new(HashMap::new()),
        }
    }

    /// 获取或创建节点的 SolverState
    fn get_state(&self, node_id: usize, init: f64) -> SolverState {
        self.states
            .borrow_mut()
            .entry(node_id)
            .or_insert_with(|| SolverState::new(init))
            .clone()
    }

    /// 更新 SolverState
    fn set_state(&self, node_id: usize, state: SolverState) {
        self.states.borrow_mut().insert(node_id, state);
    }
}

impl Solver for NewtonSolver {
    fn supports(&self, node: &Node) -> bool {
        matches!(node.formula, Expr::Eq(_, _))
    }

    fn solve(&self, node: &Node, ctx: &HashMap<String, f64>) -> SolveResult {
        // 先代数求解
        match solve_eq(&node.formula, ctx) {
            Ok(result) if !result.symbol.is_empty() => {
                let mut map = HashMap::new();
                map.insert(result.symbol.clone(), result.value);
                map.insert("output".to_string(), result.value);
                SolveResult::Converged(map)
            }
            Ok(_) => {
                // 所有变量已知且验证通过
                if let Expr::Eq(l, r) = &node.formula {
                    if let (Ok(lv), Ok(rv)) = (l.eval(ctx), r.eval(ctx)) {
                        let mut map = HashMap::new();
                        map.insert("output".to_string(), (lv + rv) / 2.0);
                        return SolveResult::Converged(map);
                    }
                }
                SolveResult::Failed("等式验证失败".into())
            }
            Err(_) => {
                // 降级到 Newton
                let mut state = self.get_state(node.id, 0.0);
                let result = newton_solve_step(node, ctx, &mut state);
                self.set_state(node.id, state);
                result
            }
        }
    }
}

/// 实际执行 Newton 一步（可从外部调用）
pub fn newton_solve_step(
    node: &Node,
    ctx: &HashMap<String, f64>,
    state: &mut SolverState,
) -> SolveResult {
    let target = node.solve_target.as_deref().unwrap_or("");
    if target.is_empty() {
        return SolveResult::Failed("无求解目标".into());
    }

    let mut f = make_eq_function(&node.formula, target, ctx);
    let test_val = f(state.current);
    if !test_val.is_finite() {
        return SolveResult::Failed("奇异点（除零）".into());
    }

    solver_step(state, make_eq_function(&node.formula, target, ctx));

    let mut map = HashMap::new();
    map.insert(target.to_string(), state.current);
    map.insert("output".to_string(), state.current);

    if state.converged {
        SolveResult::Converged(map)
    } else {
        SolveResult::Partial(map)
    }
}

// ============================================================
// 便捷构造函数
// ============================================================

/// 创建默认求解器管理器：EvalSolver + NewtonSolver
pub fn default_solver_manager() -> (SolverManager, NewtonSolver) {
    let newton = NewtonSolver::new();
    let mgr = SolverManager::new(vec![
        Box::new(EvalSolver::new()),
        Box::new(NewtonSolver::new()),
    ]);
    (mgr, newton)
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse_simple_eq;

    #[test]
    fn test_eval_solver_number() {
        let node = Node {
            id: 0,
            formula: Expr::Number(42.0),
            state: NodeState::Gray,
            value: None,
            solve_target: None,
        };
        let solver = EvalSolver::new();
        assert!(solver.supports(&node));

        let ctx = HashMap::new();
        let result = solver.solve(&node, &ctx);
        match result {
            SolveResult::Converged(map) => {
                assert!((map.get("output").unwrap() - 42.0).abs() < 1e-9);
            }
            _ => panic!("期望 Converged"),
        }
    }

    #[test]
    fn test_eval_solver_expression() {
        let expr = crate::ast::parse_expression("a + b").unwrap();
        let node = Node {
            id: 0,
            formula: expr,
            state: NodeState::Gray,
            value: None,
            solve_target: None,
        };
        let solver = EvalSolver::new();
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), 3.0);
        ctx.insert("b".to_string(), 7.0);

        let result = solver.solve(&node, &ctx);
        match result {
            SolveResult::Converged(map) => {
                assert!((map.get("output").unwrap() - 10.0).abs() < 1e-9);
            }
            _ => panic!("期望 Converged"),
        }
    }

    #[test]
    fn test_newton_solver_solve_eq() {
        // a + 3 = 10 → 代数求解 a = 7
        let expr = parse_simple_eq("a + 3 = 10").unwrap();
        let node = Node {
            id: 0,
            formula: expr,
            state: NodeState::Gray,
            value: None,
            solve_target: Some("a".to_string()),
        };
        let solver = NewtonSolver::new();
        assert!(solver.supports(&node));

        let ctx = HashMap::new();
        let result = solver.solve(&node, &ctx);
        match result {
            SolveResult::Converged(map) => {
                let val = map.get("a").unwrap();
                assert!((val - 7.0).abs() < 1e-9, "期望 a=7, 得到 {}", val);
            }
            _ => panic!("代数求解应 Converged, 结果: {:?}", result),
        }
    }

    #[test]
    fn test_newton_solve_step_function() {
        // 直接测试 newton_solve_step
        let expr = parse_simple_eq("x * x = 4").unwrap();
        let node = Node {
            id: 0,
            formula: expr,
            state: NodeState::Gray,
            value: None,
            solve_target: Some("x".to_string()),
        };
        let ctx = HashMap::new();
        let mut state = SolverState::new(3.0);

        for _ in 0..20 {
            let result = newton_solve_step(&node, &ctx, &mut state);
            match result {
                SolveResult::Converged(map) => {
                    let val = map.get("x").unwrap();
                    assert!((val - 2.0).abs() < 1e-5, "期望 x≈2, 得到 {}", val);
                    return;
                }
                _ => {}
            }
        }
        panic!("Newton 未收敛");
    }
}
