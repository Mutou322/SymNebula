use crate::types::{Cluster, RuntimeSnapshot, SolverResult};
use crate::SolverBackend;

/// CPU 后端：委托给 sym-nebula-core 的 Newton 求解器
pub struct CpuBackend;

impl SolverBackend for CpuBackend {
    fn solve(&mut self, cluster: &Cluster, _snapshot: &RuntimeSnapshot) -> SolverResult {
        println!("[CPU] Solving Cluster {} with {} nodes", cluster.id, cluster.num_nodes);
        // TODO: 调用 core 的 NewtonSolver / BlockNewton
        // let mut solver = sym_nebula_core::solver::create_solver(...);
        // solver.solve(...);
        SolverResult::Success
    }
}
