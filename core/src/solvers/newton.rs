/// 等式求解器（NewtonSolver）
///
/// 优先代数求解（solve_eq），失败则降级到单变量 Newton。
/// 内部持有每个节点的 SolverState，独立维护迭代状态。
///
/// 安全守卫：
///   - 未知变量数为 0 → 验证等式
///   - 未知变量数 > 1 → Partial(Underdetermined)
///   - 未知变量数合理 → 代数求解 / Newton

use std::cell::RefCell;
use std::collections::HashMap;

use crate::ast::Expr;
use crate::graph::Node;
use crate::solver::{solve_eq, solver_step, make_eq_function};
use crate::solver_trait::{Solver, SolveResult, PartialReason};
use crate::state::SolverState;

/// 等式求解器。
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
    fn name(&self) -> &'static str {
        "newton"
    }

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
                                SolveResult::Converged(map)
                            } else {
                                SolveResult::Failed(format!("约束不满足: lhs={} != rhs={}", lv, rv))
                            }
                        }
                        _ => SolveResult::Failed("等式求值失败".into()),
                    }
                } else {
                    SolveResult::Failed("不是等式".into())
                }
            }
            1 => {
                // 正好一个未知 → 代数求解 / Newton
                match solve_eq(&node.formula, ctx) {
                    Ok(result) if !result.symbol.is_empty() => {
                        // 代数求解成功，数值合法性检查
                        match crate::guard::num::ensure_finite(result.value) {
                            Ok(val) => {
                                let mut map = HashMap::new();
                                map.insert(result.symbol.clone(), val);
                                map.insert("output".to_string(), val);
                                SolveResult::Converged(map)
                            }
                            Err(e) => SolveResult::Failed(e.into()),
                        }
                    }
                    _ => {
                        // 降级到 Newton
                        let mut state = self.get_state(node.id, 3.0);
                        let result = newton_solve_step_guarded(node, ctx, &mut state);
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

/// Newton 一步（带数值守卫）。
pub fn newton_solve_step_guarded(
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

    // 数值合法性检查
    let mut outputs_valid = true;
    for v in map.values() {
        if !v.is_finite() {
            outputs_valid = false;
            break;
        }
    }

    if !outputs_valid {
        return SolveResult::Failed("NaN/Inf in Newton step".into());
    }

    if state.converged {
        return SolveResult::Converged(map);
    }

    // Newton 未收敛，尝试 Bisection fallback
    let bisect_f = make_eq_function(&node.formula, target, ctx);
    let low = state.current - 5.0;
    let high = state.current + 5.0;
    if let Some(root) = crate::solver::bisection_fallback(bisect_f, low, high, 1e-6, 50) {
        state.current = root;
        state.residual = 0.0;
        state.converged = true;
        let mut bmap = HashMap::new();
        bmap.insert(target.to_string(), root);
        bmap.insert("output".to_string(), root);
        return SolveResult::Converged(bmap);
    }

    SolveResult::partial(map, PartialReason::NotConverged)
}
