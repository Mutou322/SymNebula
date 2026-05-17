use crate::types::{Cluster, RuntimeSnapshot, SolverResult};

/// 统一求解器后端接口
///
/// CPU / GPU 后端均实现此 trait，AdaptiveScheduler 据此调度。
pub trait SolverBackend {
    /// 求解一个 cluster 的所有节点
    fn solve(&mut self, cluster: &Cluster, snapshot: &RuntimeSnapshot) -> SolverResult;
}
