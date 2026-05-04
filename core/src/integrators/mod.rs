/// 积分器模块 — 时间推进器集合
///
/// 与 Solver 的区别：
///   Solver      解 F(x) = 0，输出稳态值
///   Integrator  做 state(t + dt)，输出下一时刻值
///
/// Dynamic 节点（位置、速度等）应使用 Integrator，不是 Solver。
///
/// 每个积分器是一个独立的文件，通过 Integrator trait 注册到 IntegratorManager。

pub mod symplectic;
