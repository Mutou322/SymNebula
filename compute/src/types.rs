/// SymNebula Adaptive Scheduler — 类型定义

/// 计算集群描述
#[derive(Debug, Clone)]
pub struct Cluster {
    pub id: usize,
    pub num_nodes: usize,
}

/// 运行时快照（由 Engine 生成，传递给 SolverBackend）
#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub tick: usize,
}

/// 求解结果
#[derive(Debug, Clone, PartialEq)]
pub enum SolverResult {
    Success,
    Failure,
}

/// 当前使用的后端模式
#[derive(Debug, Clone, PartialEq)]
pub enum BackendMode {
    Cpu,
    Gpu,
    Hybrid,
}

/// CPU → GPU 切换阈值（节点数）
pub const GPU_THRESHOLD: usize = 50_000;
