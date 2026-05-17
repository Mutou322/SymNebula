use crate::types::{BackendMode, Cluster, NodeStatus, RuntimeSnapshot, GPU_THRESHOLD};
use crate::{CpuBackend, GpuBackend, SolverBackend};

/// 自适应调度器
pub struct AdaptiveScheduler {
    cpu: CpuBackend,
    gpu: Option<GpuBackend>,
    pub mode: BackendMode,
}

impl AdaptiveScheduler {
    pub fn new(gpu_available: bool) -> Self {
        Self {
            cpu: CpuBackend,
            gpu: if gpu_available { Some(GpuBackend) } else { None },
            mode: BackendMode::Cpu,
        }
    }

    /// 求解单个 cluster，更新其 status
    pub fn solve_cluster(&mut self, cluster: &mut Cluster, snapshot: &RuntimeSnapshot) {
        let n = cluster.num_nodes;
        let success = if n <= GPU_THRESHOLD {
            self.mode = BackendMode::Cpu;
            self.cpu.solve(cluster, snapshot)
        } else {
            if let Some(gpu) = &mut self.gpu {
                self.mode = BackendMode::Hybrid;
                gpu.solve(cluster, snapshot)
            } else {
                self.mode = BackendMode::Cpu;
                self.cpu.solve(cluster, snapshot)
            }
        };

        cluster.status = if success {
            NodeStatus::Green
        } else {
            NodeStatus::Purple
        };
    }
}
