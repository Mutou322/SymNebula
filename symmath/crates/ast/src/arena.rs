use slotmap::SlotMap;

use symmath_common::ids::NodeId;

use crate::expr::Expr;

pub struct AstArena {
    nodes: SlotMap<NodeId, Expr>,
}

impl AstArena {
    pub fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
        }
    }

    pub fn add(&mut self, expr: Expr) -> NodeId {
        self.nodes.insert(expr)
    }

    pub fn get(&self, id: NodeId) -> Option<&Expr> {
        self.nodes.get(id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}
