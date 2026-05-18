use slotmap::SlotMap;
use std::collections::HashMap;
use symmath_ast::ops::BinaryOp;
use symmath_common::ids::{NodeId, ValueId};
use symmath_ir::builder::IRBuilder;
use symmath_ir::node::IRNode;
use crate::node::{Node, NodeKind};

pub struct ConstraintGraph {
    pub nodes: SlotMap<NodeId, Node>,
}

impl ConstraintGraph {
    pub fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
        }
    }

    pub fn add_node(&mut self, node: Node) -> NodeId {
        self.nodes.insert(node)
    }

    pub fn mark_dirty(&mut self, id: NodeId) {
        if self.nodes[id].dirty {
            return;
        }
        self.nodes[id].dirty = true;
        let deps = self.nodes[id].dependents.clone();
        for dep_id in deps {
            self.mark_dirty(dep_id);
        }
    }

    pub fn dirty_nodes(&self) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.dirty)
            .map(|(id, _)| id)
            .collect()
    }

    pub fn update_node(&mut self, id: NodeId) {
        let kind = self.nodes[id].kind.clone();
        match kind {
            NodeKind::Const(_) => {
                self.nodes[id].dirty = false;
            }
            NodeKind::Input(_) => {
                self.nodes[id].dirty = false;
            }
            NodeKind::Op { op, inputs } => {
                let values: Vec<f64> = inputs
                    .iter()
                    .map(|iid| self.nodes[*iid].value.unwrap())
                    .collect();
                let result = if inputs.len() == 1 {
                    match op {
                        BinaryOp::Sin => values[0].sin(),
                        BinaryOp::Cos => values[0].cos(),
                        _ => values[0],
                    }
                } else {
                    match op {
                        BinaryOp::Add => values[0] + values[1],
                        BinaryOp::Sub => values[0] - values[1],
                        BinaryOp::Mul => values[0] * values[1],
                        BinaryOp::Div => values[0] / values[1],
                        BinaryOp::Pow => values[0] * values[1],
                        BinaryOp::Eq => values[0] - values[1],
                        BinaryOp::Sin | BinaryOp::Cos => unreachable!(),
                    }
                };
                self.nodes[id].value = Some(result);
                self.nodes[id].dirty = false;
            }
            NodeKind::ConstraintEq(..) => {
                self.nodes[id].dirty = false;
            }
        }
    }

    pub fn tick(&mut self) {
        let dirty = self.dirty_nodes();
        for id in dirty {
            self.update_node(id);
        }
    }
}

pub fn build_graph(ir: &IRBuilder) -> (ConstraintGraph, HashMap<ValueId, NodeId>) {
    let mut graph = ConstraintGraph::new();
    let mut mapping: HashMap<ValueId, NodeId> = HashMap::new();

    for (vid, ir_node) in ir.iter() {
        match ir_node {
            IRNode::Const(c) => {
                let node = Node {
                    kind: NodeKind::Const(*c),
                    value: Some(*c),
                    dirty: false,
                    dependents: Vec::new(),
                };
                let id = graph.add_node(node);
                mapping.insert(vid, id);
            }
            IRNode::LoadVar(sym) => {
                let node = Node {
                    kind: NodeKind::Input(*sym),
                    value: None,
                    dirty: true,
                    dependents: Vec::new(),
                };
                let id = graph.add_node(node);
                mapping.insert(vid, id);
            }
            IRNode::Add(lhs, rhs)
            | IRNode::Sub(lhs, rhs)
            | IRNode::Mul(lhs, rhs)
            | IRNode::Div(lhs, rhs) => {
                let lhs_id = mapping[lhs];
                let rhs_id = mapping[rhs];
                let op = match ir_node {
                    IRNode::Add(..) => BinaryOp::Add,
                    IRNode::Sub(..) => BinaryOp::Sub,
                    IRNode::Mul(..) => BinaryOp::Mul,
                    IRNode::Div(..) => BinaryOp::Div,
                    _ => unreachable!(),
                };
                let node = Node {
                    kind: NodeKind::Op {
                        op,
                        inputs: vec![lhs_id, rhs_id],
                    },
                    value: None,
                    dirty: true,
                    dependents: Vec::new(),
                };
                let id = graph.add_node(node);
                mapping.insert(vid, id);
                graph.nodes.get_mut(lhs_id).unwrap().dependents.push(id);
                graph.nodes.get_mut(rhs_id).unwrap().dependents.push(id);
            }
            IRNode::Sin(val) | IRNode::Cos(val) => {
                let val_id = mapping[val];
                let op = match ir_node {
                    IRNode::Sin(_) => BinaryOp::Sin,
                    _ => BinaryOp::Cos,
                };
                let node = Node {
                    kind: NodeKind::Op {
                        op,
                        inputs: vec![val_id],
                    },
                    value: None,
                    dirty: true,
                    dependents: Vec::new(),
                };
                let id = graph.add_node(node);
                mapping.insert(vid, id);
                graph.nodes.get_mut(val_id).unwrap().dependents.push(id);
            }
            IRNode::Eq(lhs, rhs) => {
                let lhs_id = mapping[lhs];
                let rhs_id = mapping[rhs];
                let node = Node {
                    kind: NodeKind::ConstraintEq(lhs_id, rhs_id),
                    value: None,
                    dirty: false,
                    dependents: Vec::new(),
                };
                let id = graph.add_node(node);
                mapping.insert(vid, id);
            }
        }
    }

    (graph, mapping)
}
