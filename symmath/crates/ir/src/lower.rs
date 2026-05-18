use std::collections::HashMap;

use symmath_ast::arena::AstArena;
use symmath_ast::expr::Expr;
use symmath_ast::ops::BinaryOp;
use symmath_common::ids::{NodeId, SymbolId, ValueId};

use crate::builder::IRBuilder;
use crate::node::IRNode;

/// AST → IR lowering。
///
/// 递归下降，每个 AST NodeId 对应一个 IR ValueId。
/// 使用 cache 保证 DAG 结构：同一 AST 节点只 lower 一次。
pub struct Lower {
    cache: HashMap<NodeId, ValueId>,
    sin_sym: SymbolId,
    cos_sym: SymbolId,
}

impl Lower {
    pub fn new(sin_sym: SymbolId, cos_sym: SymbolId) -> Self {
        Self {
            cache: HashMap::new(),
            sin_sym,
            cos_sym,
        }
    }

    pub fn lower_expr(
        &mut self,
        ast: &AstArena,
        ir: &mut IRBuilder,
        node: NodeId,
    ) -> ValueId {
        // DAG cache：已 lower 过的节点直接返回
        if let Some(&id) = self.cache.get(&node) {
            return id;
        }

        let expr = ast.get(node).expect("invalid NodeId");
        let id = match expr {
            Expr::Const(v) => ir.push(IRNode::Const(*v)),
            Expr::Var(sym) => ir.push(IRNode::LoadVar(*sym)),
            Expr::Unary { op: _, input } => {
                let inner = self.lower_expr(ast, ir, *input);
                let zero = ir.push(IRNode::Const(0.0));
                ir.push(IRNode::Sub(zero, inner))
            }
            Expr::Binary { op, lhs, rhs } => {
                let l = self.lower_expr(ast, ir, *lhs);
                let r = self.lower_expr(ast, ir, *rhs);
                match op {
                    BinaryOp::Add => ir.push(IRNode::Add(l, r)),
                    BinaryOp::Sub => ir.push(IRNode::Sub(l, r)),
                    BinaryOp::Mul => ir.push(IRNode::Mul(l, r)),
                    BinaryOp::Div => ir.push(IRNode::Div(l, r)),
                    BinaryOp::Pow => ir.push(IRNode::Mul(l, r)),
                    BinaryOp::Eq => ir.push(IRNode::Eq(l, r)),
                    BinaryOp::Sin | BinaryOp::Cos => unreachable!(),
                }
            }
            Expr::Call { func, args } => {
                let lowered_args: Vec<_> = args
                    .iter()
                    .map(|a| self.lower_expr(ast, ir, *a))
                    .collect();
                if *func == self.sin_sym {
                    ir.push(IRNode::Sin(lowered_args[0]))
                } else if *func == self.cos_sym {
                    ir.push(IRNode::Cos(lowered_args[0]))
                } else {
                    panic!("unsupported function: {:?}", func);
                }
            }
        };

        self.cache.insert(node, id);
        id
    }
}
