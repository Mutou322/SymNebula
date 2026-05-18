use symmath_common::ids::NodeId;

use crate::graph::ConstraintGraph;

#[allow(dead_code)]
pub struct Transaction<'a> {
    graph: &'a mut ConstraintGraph,
    snapshot: Vec<(NodeId, Option<f64>)>,
}

impl<'a> Transaction<'a> {
    pub fn new(graph: &'a mut ConstraintGraph) -> Self {
        let snapshot = graph
            .dirty_nodes()
            .iter()
            .flat_map(|&id| {
                let val = graph.nodes.get(id).and_then(|n| n.value);
                Some((id, val))
            })
            .collect();
        Self { graph, snapshot }
    }

    pub fn commit(self) {
        // Just consume self, nothing to restore
    }

    pub fn rollback(self) {
        for (id, val) in self.snapshot {
            if let Some(node) = self.graph.nodes.get_mut(id) {
                node.value = val;
                node.dirty = true;
            }
        }
    }
}
