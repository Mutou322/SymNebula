use crate::types::{BackendMode, Cluster, GPU_THRESHOLD, RuntimeSnapshot, SolverResult};
use crate::{CpuBackend, GpuBackend, SolverBackend};

/// 自适应调度器
///
/// 根据集群大小和 GPU 可用性动态选择后端：
/// - < 50K nodes → CPU 后端
/// - ≥ 50K nodes → GPU 后端（不可用时回退 CPU）
pub struct AdaptiveScheduler {
    cpu_backend: CpuBackend,
    gpu_backend: Option<GpuBackend>,
    pub mode: BackendMode,
}

impl AdaptiveScheduler {
    pub fn new(gpu_available: bool) -> Self {
        Self {
            cpu_backend: CpuBackend,
            gpu_backend: if gpu_available { Some(GpuBackend) } else { None },
            mode: BackendMode::Cpu,
        }
    }

    /// 求解单个 cluster，自动选择后端
    pub fn solve_cluster(&mut self, cluster: &Cluster, snapshot: &RuntimeSnapshot) -> SolverResult {
        match cluster.num_nodes {
            0..GPU_THRESHOLD => {
                self.mode = BackendMode::Cpu;
                self.cpu_backend.solve(cluster, snapshot)
            }
            _ => {
                if let Some(gpu) = &mut self.gpu_backend {
                    self.mode = BackendMode::Gpu;
                    gpu.solve(cluster, snapshot)
                } else {
                    // GPU 不可用，安全回退
                    self.mode = BackendMode::Cpu;
                    self.cpu_backend.solve(cluster, snapshot)
                }
            }
        }
    }
}
