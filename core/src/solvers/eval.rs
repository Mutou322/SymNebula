/// 纯表达式求解器（EvalSolver）
///
/// 处理所有无等号的节点（Number、纯表达式）。

use std::collections::HashMap;

use crate::ast::Expr;
use crate::graph::Node;
use crate::solver_trait::Solver;
use symnebula_macros::safe_solver;

pub struct EvalSolver;

impl EvalSolver {
    pub fn new() -> Self {
        EvalSolver
    }
}

#[safe_solver(priority = 100, name = "eval")]
impl Solver for EvalSolver {
    fn supports(&self, node: &Node) -> bool {
        !matches!(node.formula, Expr::Eq(_, _))
    }

    fn solve(
        &self,
        node: &Node,
        ctx: &HashMap<String, f64>,
    ) -> core::result::Result<std::collections::HashMap<String, f64>, &'static str> {
        let mut map = HashMap::new();
        match &node.formula {
            Expr::Number(n) => {
                map.insert("output".to_string(), *n);
                Ok(map)
            }
            expr => match expr.eval(ctx) {
                Ok(val) => {
                    map.insert("output".to_string(), val);
                    Ok(map)
                }
                Err(_e) => Err("eval error"),
            },
        }
    }
}
