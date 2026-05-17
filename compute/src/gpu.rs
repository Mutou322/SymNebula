use crate::types::{Cluster, RuntimeSnapshot, SolverResult};
use crate::SolverBackend;

/// GPU 后端：通过 wgpu compute shader 并行求解大集群
///
/// 当前为占位实现，后续替换为真正的 WGSL compute pipeline。
pub struct GpuBackend;

impl SolverBackend for GpuBackend {
    fn solve(&mut self, cluster: &Cluster, _snapshot: &RuntimeSnapshot) -> SolverResult {
        println!("[GPU] Solving Cluster {} with {} nodes", cluster.id, cluster.num_nodes);
        // TODO: 实现 wgpu compute shader 并行求解
        // 1. 构建 ComputePipeline（WGSL）
        // 2. 上传 cluster 数据到 storage buffer
        // 3. dispatch workgroups
        // 4. 读取结果回 CPU
        SolverResult::Success
    }
}
