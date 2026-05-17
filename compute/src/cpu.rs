use crate::types::{Cluster, RuntimeSnapshot};
use crate::SolverBackend;

/// CPU 后端：模拟 Newton 迭代，步长 0.1
pub struct CpuBackend;

impl SolverBackend for CpuBackend {
    fn solve(&mut self, cluster: &mut Cluster, _snapshot: &RuntimeSnapshot) -> bool {
        println!("[CPU] Solving Cluster {} with {} nodes", cluster.id, cluster.num_nodes);
        for x in cluster.x_cluster.iter_mut() {
            *x += 0.1;
        }
        true
    }
}
