use crate::types::{Cluster, RuntimeSnapshot};

/// 统一求解器后端接口
pub trait SolverBackend {
    /// 求解一个 cluster，返回是否成功收敛
    fn solve(&mut self, cluster: &mut Cluster, snapshot: &RuntimeSnapshot) -> bool;
}
