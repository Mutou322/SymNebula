/// 求解器模块化接口 + 标准实现
///
/// Solver trait 将求解逻辑与 Tick/Graph/Engine 完全解耦。
/// 内核只负责调度，不负责"怎么解方程"。
///
/// 架构：
///   SolverManager → 按优先级排序 → Supports() → solve() → SolveResult
///
/// 内置 Solver：
///   - EvalSolver:    纯表达式求值（无等号），优先级 100
///   - NewtonSolver:  等式节点，代数求解 + Newton 降级，优先级 200

use std::cell::RefCell;
use std::collections::HashMap;

use crate::ast::Expr;
use crate::graph::Node;
use crate::solver::{make_eq_function, solve_eq, solver_step};
use crate::state::{NodeState, SolverState};

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

    /// 按优先级排序后找到第一个 supports 的并执行。
    pub fn solve_node(&self, node: &Node, ctx: &HashMap<String, f64>) -> SolveResult {
        // 线性扫描找最高优先级的匹配求解器
        let mut best: Option<&Box<dyn Solver>> = None;
        let mut best_priority: u8 = 255;

        for solver in &self.solvers {
            if solver.supports(node) {
                let p = solver.priority();
                if p < best_priority {
                    best_priority = p;
                    best = Some(solver);
                    // 优先级 0 是最高的，碰到直接执行
                    if p == 0 {
                        return solver.solve(node, ctx);
                    }
                }
            }
        }

        match best {
            Some(solver) => solver.solve(node, ctx),
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

    pub fn step_node(
        &self,
        node: &Node,
        ctx: &HashMap<String, f64>,
        dt: f64,
    ) -> SolveResult {
        for integrator in &self.integrators {
            let result = integrator.step(node, ctx, dt);
            return result;
        }
        SolveResult::NoOp
    }
}

// ============================================================
// SymplecticEulerIntegrator — 半隐式欧拉
// ============================================================

/// 半隐式欧拉积分器。
///
/// v_{n+1} = v_n + a * dt
/// x_{n+1} = x_n + v_{n+1} * dt
pub struct SymplecticEulerIntegrator;

impl SymplecticEulerIntegrator {
    pub fn new() -> Self {
        SymplecticEulerIntegrator
    }
}

impl Integrator for SymplecticEulerIntegrator {
    fn step(&self, _node: &Node, ctx: &HashMap<String, f64>, dt: f64) -> SolveResult {
        let v_old = ctx.get("v").copied().unwrap_or(0.0);
        let x_old = ctx.get("x").copied().unwrap_or(0.0);
        let a = ctx.get("a").copied().unwrap_or(0.0);

        let v_new = v_old + a * dt;
        let x_new = x_old + v_new * dt;

        let mut map = HashMap::new();
        map.insert("v".to_string(), v_new);
        map.insert("x".to_string(), x_new);
        map.insert("output".to_string(), (x_new + v_new) / 2.0);
        SolveResult::Converged(map)
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
    fn priority(&self) -> u8 {
        100
    }

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
///
/// 安全守卫：
///   - 未知变量数为 0 → 验证等式
///   - 未知变量数 > 1 → Partial(Underdetermined)
///   - 未知变量数合理 → 代数求解 / Newton
pub struct NewtonSolver {
    states: RefCell<HashMap<usize, SolverState>>,
}

impl NewtonSolver {
    pub fn new() -> Self {
        NewtonSolver {
            states: RefCell::new(HashMap::new()),
        }
    }

    fn get_state(&self, node_id: usize, init: f64) -> SolverState {
        self.states
            .borrow_mut()
            .entry(node_id)
            .or_insert_with(|| SolverState::new(init))
            .clone()
    }

    fn set_state(&self, node_id: usize, state: SolverState) {
        self.states.borrow_mut().insert(node_id, state);
    }
}

impl Solver for NewtonSolver {
    fn priority(&self) -> u8 {
        200
    }

    fn supports(&self, node: &Node) -> bool {
        matches!(node.formula, Expr::Eq(_, _))
    }

    fn solve(&self, node: &Node, ctx: &HashMap<String, f64>) -> SolveResult {
        // ---- 安全守卫：检查未知变量个数 ----
        let all_syms = node.formula.symbols();
        let unknowns: Vec<&String> = all_syms.iter().filter(|s| !ctx.contains_key(*s)).collect();

        match unknowns.len() {
            0 => {
                // 全部已知 → 验证等式
                if let Expr::Eq(l, r) = &node.formula {
                    match (l.eval(ctx), r.eval(ctx)) {
                        (Ok(lv), Ok(rv)) => {
                            let diff = (lv - rv).abs();
                            let max_abs = lv.abs().max(rv.abs()).max(1.0);
                            if diff / max_abs < 1e-2 {
                                let mut map = HashMap::new();
                                map.insert("output".to_string(), lv);
                                return SolveResult::Converged(map);
                            }
                            return SolveResult::Failed(format!(
                                "约束不满足: lhs={} != rhs={}",
                                lv, rv
                            ));
                        }
                        _ => return SolveResult::Failed("等式求值失败".into()),
                    }
                }
                SolveResult::Failed("不是等式".into())
            }
            1 => {
                // 正好一个未知 → 代数求解 / Newton
                match solve_eq(&node.formula, ctx) {
                    Ok(result) if !result.symbol.is_empty() => {
                        let mut map = HashMap::new();
                        map.insert(result.symbol.clone(), result.value);
                        map.insert("output".to_string(), result.value);
                        SolveResult::Converged(map)
                    }
                    _ => {
                        // 降级到 Newton
                        let mut state = self.get_state(node.id, 0.0);
                        let result = newton_solve_step(node, ctx, &mut state);
                        self.set_state(node.id, state);
                        result
                    }
                }
            }
            _ => {
                // 多个未知 → 欠定
                SolveResult::partial(HashMap::new(), PartialReason::Underdetermined)
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
        SolveResult::partial(map, PartialReason::NotConverged)
    }
}

// ============================================================
// 默认管理器构造
// ============================================================

/// 创建默认求解器管理器（EvalSolver + NewtonSolver）
pub fn default_solver_manager() -> SolverManager {
    SolverManager::new(vec![
        Box::new(EvalSolver::new()),
        Box::new(NewtonSolver::new()),
    ])
}

/// 创建默认积分器管理器（SymplecticEuler）
pub fn default_integrator_manager() -> IntegratorManager {
    IntegratorManager::new(vec![Box::new(SymplecticEulerIntegrator::new())])
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse_simple_eq;

    // --- SolveResult ---

    #[test]
    fn test_solve_result_node_state_mapping() {
        let m = HashMap::new();
        assert_eq!(SolveResult::Converged(m.clone()).node_state(), NodeState::Green);
        assert_eq!(
            SolveResult::partial(m.clone(), PartialReason::NotConverged).node_state(),
            NodeState::Yellow
        );
        assert_eq!(SolveResult::Failed("err".into()).node_state(), NodeState::Purple);
        assert_eq!(SolveResult::NoOp.node_state(), NodeState::Gray);
    }

    // --- SolverManager 优先级 ---

    struct PriorityTestSolver {
        p: u8,
        name: &'static str,
    }
    impl Solver for PriorityTestSolver {
        fn priority(&self) -> u8 { self.p }
        fn supports(&self, _node: &Node) -> bool { true }
        fn solve(&self, _node: &Node, _ctx: &HashMap<String, f64>) -> SolveResult {
            let mut map = HashMap::new();
            map.insert(self.name.to_string(), self.p as f64);
            SolveResult::Converged(map)
        }
    }

    #[test]
    fn test_solver_priority() {
        let mgr = SolverManager::new(vec![
            Box::new(PriorityTestSolver { p: 200, name: "low" }),
            Box::new(PriorityTestSolver { p: 50, name: "high" }),
        ]);

        let node = Node {
            id: 0,
            formula: Expr::Number(0.0),
            state: NodeState::Gray,
            value: None,
            solve_target: None,
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

    // --- EvalSolver ---

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

    // --- NewtonSolver ---

    #[test]
    fn test_newton_solver_solve_eq() {
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
    fn test_newton_solver_underdetermined() {
        // a + b = 10，两个未知数 → Underdetermined
        let expr = parse_simple_eq("a + b = 10").unwrap();
        let node = Node {
            id: 0,
            formula: expr,
            state: NodeState::Gray,
            value: None,
            solve_target: Some("a".to_string()),
        };
        let solver = NewtonSolver::new();
        let ctx = HashMap::new();
        let result = solver.solve(&node, &ctx);
        match result {
            SolveResult::Partial { reason, .. } => {
                assert_eq!(reason, PartialReason::Underdetermined,
                    "两个未知数应标记 Underdetermined");
            }
            other => panic!("期望 Partial(Underdetermined), 得到 {:?}", other),
        }
    }

    #[test]
    fn test_newton_solve_step_function() {
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

    // --- Integrator ---

    #[test]
    fn test_symplectic_euler_integrator() {
        let node = Node {
            id: 0,
            formula: Expr::Number(0.0),
            state: NodeState::Gray,
            value: None,
            solve_target: None,
        };
        let mut ctx = HashMap::new();
        ctx.insert("v".to_string(), 1.0);
        ctx.insert("x".to_string(), 0.0);
        ctx.insert("a".to_string(), 0.0);

        let integrator = SymplecticEulerIntegrator::new();
        let result = integrator.step(&node, &ctx, 0.01);
        match result {
            SolveResult::Converged(map) => {
                let v = map.get("v").unwrap();
                let x = map.get("x").unwrap();
                assert!((v - 1.0).abs() < 1e-9, "速度应不变");
                assert!((x - 0.01).abs() < 1e-9, "x = v*dt = 0.01");
            }
            _ => panic!("期望 Converged"),
        }
    }
}
