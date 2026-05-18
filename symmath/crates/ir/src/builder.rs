use slotmap::SlotMap;

use symmath_common::ids::ValueId;

use crate::node::IRNode;

pub struct IRBuilder {
    values: SlotMap<ValueId, IRNode>,
}

impl IRBuilder {
    pub fn new() -> Self {
        Self {
            values: SlotMap::with_key(),
        }
    }

    pub fn push(&mut self, node: IRNode) -> ValueId {
        self.values.insert(node)
    }

    pub fn get(&self, id: ValueId) -> Option<&IRNode> {
        self.values.get(id)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (ValueId, &IRNode)> {
        self.values.iter().map(|(id, node)| (id, node))
    }
}
