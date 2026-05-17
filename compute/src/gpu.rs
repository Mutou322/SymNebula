use crate::types::{Cluster, RuntimeSnapshot};
use crate::SolverBackend;

/// GPU 后端：模拟并行迭代，步长 0.2
pub struct GpuBackend;

impl SolverBackend for GpuBackend {
    fn solve(&mut self, cluster: &mut Cluster, _snapshot: &RuntimeSnapshot) -> bool {
        println!("[GPU] Solving Cluster {} with {} nodes", cluster.id, cluster.num_nodes);
        for x in cluster.x_cluster.iter_mut() {
            *x += 0.2;
        }
        true
    }
}
