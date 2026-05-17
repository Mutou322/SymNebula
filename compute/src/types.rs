/// SymNebula Adaptive Scheduler — 类型定义

/// 节点状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Green,
    Yellow,
    Purple,
}

/// 计算集群
#[derive(Debug, Clone)]
pub struct Cluster {
    pub id: usize,
    pub num_nodes: usize,
    /// 当前迭代猜测值（X_cluster）
    pub x_cluster: Vec<f64>,
    pub status: NodeStatus,
}

/// 运行时快照
#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub tick: usize,
}

/// 后端模式
#[derive(Debug, Clone, PartialEq)]
pub enum BackendMode {
    Cpu,
    Gpu,
    Hybrid,
}

/// CPU → GPU 切换阈值
pub const GPU_THRESHOLD: usize = 50_000;
