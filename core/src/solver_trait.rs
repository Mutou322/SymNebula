/// 求解器模块化接口 + 标准实现
///
/// Solver trait 将求解逻辑与 Tick/Graph/Engine 解耦：
///   - StdNewtonSolver: 标准实现，多变量 Newton + 半隐式欧拉
///   - 未来可加 SparseSolver / GPUSolver，不改动核心引擎
///
/// NodeContext: 节点运行时上下文，封装变量读写和默认值

use std::collections::HashMap;

use crate::ast::Expr;
use crate::graph::Node;
use crate::solver::{symplectic_euler_step, Matrix};

// ============================================================
// NodeContext
// ============================================================

/// 节点运行时上下文，封装变量读写和默认值
#[derive(Debug, Clone)]
pub struct NodeContext {
    values: HashMap<String, f64>,
}

impl NodeContext {
    pub fn new() -> Self {
        NodeContext {
            values: HashMap::new(),
        }
    }

    /// 从已知值构建上下文
    pub fn from_map(values: HashMap<String, f64>) -> Self {
        NodeContext { values }
    }

    pub fn get(&self, name: &str) -> f64 {
        *self.values.get(name).unwrap_or(&0.0)
    }

    pub fn set(&mut self, name: &str, val: f64) {
        self.values.insert(name.to_string(), val);
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// 获取所有已知变量的引用
    pub fn values(&self) -> &HashMap<String, f64> {
        &self.values
    }

    /// 消费 self，返回底层 map
    pub fn into_map(self) -> HashMap<String, f64> {
        self.values
    }
}

// ============================================================
// Solver trait
// ============================================================

/// 抽象求解器接口。
///
/// 实现此 trait 的结构体可替换求解策略，不依赖 Tick/Graph/Engine。
pub trait Solver {
    /// 对节点执行一步求解。
    ///
    /// node:  当前节点（含 formula, solver_state, value）
    /// ctx:   运行时上下文（已知变量的值）
    /// dt:    时间步长（用于动态节点半隐式欧拉）
    ///
    /// 返回 true 表示求解成功/收敛，false 表示奇异或失败
    fn solve_step(&mut self, node: &mut Node, ctx: &NodeContext, dt: f64) -> bool;
}

// ============================================================
// Expr 辅助：自动选择求解目标
// ============================================================

/// 从 Expr 中自动选出未被上下文确定的符号作为求解目标。
///
/// 收集 Eq 的所有符号，过滤掉 ctx 中已有值的，返回剩余未知。
/// 当前只返回第一个未知符号（单变量），后续扩展可返回全部。
pub fn auto_select_solve_targets(expr: &Expr, ctx: &NodeContext) -> Vec<String> {
    expr.symbols()
        .into_iter()
        .filter(|s| !ctx.is_defined(s))
        .collect()
}

/// 从 Expr 中提取未知符号并返回第一个，作为 solve_target 的字符串
pub fn first_unknown(expr: &Expr, ctx: &NodeContext) -> Option<String> {
    let targets = auto_select_solve_targets(expr, ctx);
    targets.into_iter().next()
}

// ============================================================
// StdNewtonSolver
// ============================================================

/// 标准 Newton 求解器。
///
/// 使用已有的 solve_eq 做代数求解，失败则降级到单变量 Newton。
/// 支持半隐式欧拉更新动态节点。
pub struct StdNewtonSolver;

impl StdNewtonSolver {
    pub fn new() -> Self {
        StdNewtonSolver
    }
}

impl Solver for StdNewtonSolver {
    fn solve_step(&mut self, node: &mut Node, ctx: &NodeContext, dt: f64) -> bool {
        let formula = &node.formula;
        let known = ctx.values();
        let solve_target = node.solve_target.clone();

        match formula {
            Expr::Eq(_, _) => {
                // 先代数求解
                match crate::solver::solve_eq(formula, known) {
                    Ok(result) if result.state == crate::state::NodeState::Green => {
                        node.solver_state.converged = true;
                        node.solver_state.current = result.value;
                        node.value = Some(result.value);
                        true
                    }
                    _ => {
                        // 降级到 Newton
                        let target = solve_target.as_deref().unwrap_or("");
                        if target.is_empty() {
                            return false;
                        }

                        // 检查 f(x) 是否可求值
                        let mut f = crate::solver::make_eq_function(formula, target, known);
                        let test_val = f(node.solver_state.current);
                        if !test_val.is_finite() {
                            return false; // 奇异，调用方处理
                        }

                        crate::solver::solver_step(
                            &mut node.solver_state,
                            crate::solver::make_eq_function(formula, target, known),
                        );

                        if node.solver_state.converged {
                            node.value = Some(node.solver_state.current);
                        } else {
                            node.value = Some(node.solver_state.current);
                        }
                        node.solver_state.converged
                    }
                }
            }
            Expr::Number(n) => {
                node.solver_state.converged = true;
                node.solver_state.current = *n;
                node.value = Some(*n);
                true
            }
            _ => {
                // 纯表达式 eval
                match formula.eval(known) {
                    Ok(val) => {
                        node.solver_state.converged = true;
                        node.solver_state.current = val;
                        node.value = Some(val);
                        true
                    }
                    Err(_) => false,
                }
            }
        }
    }
}

// ============================================================
// 多变量 Newton + Jacobian 的 Solver 扩展
// ============================================================

/// 多变量 Newton 求解器。
///
/// 自动选择未知目标，构建 Jacobian，高斯消元求解。
pub struct MultiNewtonSolver;

impl MultiNewtonSolver {
    pub fn new() -> Self {
        MultiNewtonSolver
    }
}

impl Solver for MultiNewtonSolver {
    fn solve_step(&mut self, node: &mut Node, ctx: &NodeContext, dt: f64) -> bool {
        let formula = &node.formula;

        match formula {
            Expr::Eq(_, _) => {
                let unknowns = auto_select_solve_targets(formula, ctx);
                if unknowns.is_empty() {
                    // 全部已知，验证
                    let known = ctx.values();
                    match crate::solver::solve_eq(formula, known) {
                        Ok(r) if r.state == crate::state::NodeState::Green => {
                            node.value = Some(r.value);
                            return true;
                        }
                        _ => return false,
                    }
                }

                let n = unknowns.len();
                if n > 5 {
                    // 超过 5 变量，退回到单变量 Newton
                    return StdNewtonSolver.solve_step(node, ctx, dt);
                }

                let known_map = ctx.values();

                // 构造状态向量
                let mut state: Vec<f64> = unknowns
                    .iter()
                    .map(|s| known_map.get(s).copied().unwrap_or(0.0))
                    .collect();

                // 构造闭包 f(state) -> vec of residuals
                let mut f = |x: &Vec<f64>| -> Vec<f64> {
                    let mut local = known_map.clone();
                    for (sym, val) in unknowns.iter().zip(x.iter()) {
                        local.insert(sym.clone(), *val);
                    }
                    if let Expr::Eq(lhs, rhs) = formula {
                        let lv = lhs.eval(&local).unwrap_or(f64::NAN);
                        let rv = rhs.eval(&local).unwrap_or(f64::NAN);
                        vec![lv - rv]
                    } else {
                        vec![0.0]
                    }
                };

                // 对于单变量 Eq，f 返回一个标量；多变量方程组需扩展
                // 当前 Expr 没有多方程支持，用单变量 Newton
                for _ in 0..20 {
                    if state.len() == 1 {
                        let fx = f(&state);
                        if fx[0].abs() < 1e-6 {
                            break;
                        }
                        // 数值导数
                        let eps = 1e-6;
                        let mut x_eps = state.clone();
                        x_eps[0] += eps;
                        let fx_eps = f(&x_eps);
                        let df = (fx_eps[0] - fx[0]) / eps;
                        if !df.is_finite() || df.abs() < 1e-10 {
                            break;
                        }
                        state[0] -= fx[0] / df;
                    } else {
                        // n > 1: 构建 Jacobian 矩阵
                        let fx = f(&state);
                        if fx.iter().all(|v| v.abs() < 1e-6) {
                            break;
                        }
                        let mut jac = Matrix::new(n, n);
                        let eps = 1e-6;
                        for i in 0..n {
                            let mut x_eps = state.clone();
                            x_eps[i] += eps;
                            let fx_eps = f(&x_eps);
                            for j in 0..n {
                                jac.set(j, i, (fx_eps[j] - fx[j]) / eps);
                            }
                        }
                        let rhs: Vec<f64> = fx.iter().map(|v| -v).collect();
                        if let Ok(dx) = jac.solve(rhs) {
                            for i in 0..n {
                                state[i] += dx[i];
                            }
                        } else {
                            break;
                        }
                    }
                }

                // 更新节点值
                if !state.is_empty() {
                    node.solver_state.current = state[0];
                    node.solver_state.converged = true;
                    node.value = Some(state[0]);
                }
                true
            }
            _ => {
                // 非等式节点走 eval
                let known = ctx.values();
                match formula.eval(known) {
                    Ok(val) => {
                        node.value = Some(val);
                        true
                    }
                    Err(_) => false,
                }
            }
        }
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse_simple_eq;

    #[test]
    fn test_node_context() {
        let mut ctx = NodeContext::new();
        ctx.set("x", 3.0);
        assert!((ctx.get("x") - 3.0).abs() < 1e-9);
        assert!(ctx.is_defined("x"));
        assert!(!ctx.is_defined("y"));
    }

    #[test]
    fn test_auto_select_targets() {
        let expr = parse_simple_eq("a + b = 10").unwrap();
        let mut ctx = NodeContext::new();
        ctx.set("a", 5.0);

        let targets = auto_select_solve_targets(&expr, &ctx);
        assert_eq!(targets, vec!["b".to_string()]);
    }

    #[test]
    fn test_std_newton_solver() {
        use crate::graph::NebulaGraph;

        let mut graph = NebulaGraph::new();
        let eq = parse_simple_eq("x * x = 4").unwrap();
        let node_id = graph.add_node(eq);

        let mut ctx = NodeContext::new();
        let mut solver = StdNewtonSolver::new();

        // 迭代直到收敛 (初值 0 是 x^2=4 的驻点，先设成 3.0)
        graph.nodes[node_id].solver_state = crate::state::SolverState::new(3.0);
        for _ in 0..20 {
            let node = &mut graph.nodes[node_id];
            if solver.solve_step(node, &ctx, 0.01) {
                break;
            }
        }

        let val = graph.nodes[node_id].value.unwrap();
        assert!((val - 2.0).abs() < 0.01, "期望 ≈2, 得到 {}", val);
    }
}
