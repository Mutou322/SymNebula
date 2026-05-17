// ============================================================
// SymNebula Adaptive Scheduler
//
// CPU/GPU 自适应调度
// 统一 SolverBackend trait，根据集群大小动态选择后端
// ============================================================

pub mod types;
pub mod trait_;
pub mod cpu;
pub mod gpu;
pub mod scheduler;

pub use types::*;
pub use trait_::SolverBackend;
pub use cpu::CpuBackend;
pub use gpu::GpuBackend;
pub use scheduler::AdaptiveScheduler;
