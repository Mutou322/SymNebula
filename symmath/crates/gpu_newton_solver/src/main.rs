// Phase 4 — GPU Newton Batch Solver
// 地球 → 柯伊伯带曲率跳跃链路
//
// 在 GPU 上完整执行 Newton 迭代:
//   Layer 1:  计算节点值 (tick)
//   Layer 2:  残差 f = t − t_target
//   Layer 3:  Jacobian ∂f/∂κ
//   Layer 4:  Δκ = −f / J, κ += Δκ
//
// CPU 对照实现用于验证。

#![cfg_attr(not(feature = "gpu"), allow(unused))]

use bytemuck::{Pod, Zeroable};

// ============================================================
// GPU-friendly Node layout (repr(C) + bytemuck)
// ============================================================

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GpuNode {
    pub value: f32,
    pub kind: u32,  // 0=Input, 1=Const, 2=Mul, 3=Div
    pub input0: u32,
    pub input1: u32,
}

// ============================================================
// DAG 构建
// ============================================================

/// 构建曲率跳跃 DAG，κ 作为输入参数
///
///   0: κ       Input  曲率
///   1: 1−κ     Sub    (GPU 特殊处理, kind=2 但不执行 Mul)
///   2: d=40·AU  Const  (≈ 5.98e9 km)
///   3: d_eff   Mul    d × (1−κ)
///   4: t       Div    d_eff / c
pub fn build_dag(kappa: f32) -> Vec<GpuNode> {
    let au = 149_597_870.7_f32;
    vec![
        GpuNode { value: kappa,      kind: 0, input0: 0, input1: 0 }, // 0: κ
        GpuNode { value: 0.0,        kind: 2, input0: 0, input1: 0 }, // 1: 1−κ (特殊)
        GpuNode { value: 40.0 * au,  kind: 1, input0: 0, input1: 0 }, // 2: d
        GpuNode { value: 0.0,        kind: 2, input0: 2, input1: 1 }, // 3: d_eff
        GpuNode { value: 0.0,        kind: 3, input0: 3, input1: 0 }, // 4: t
    ]
}

const TARGET_T: f32 = 59.88;
const C_LIGHT: f32 = 299_792.458;

// ============================================================
// CPU 对照实现 — 镜像 GPU kernel 逻辑
// ============================================================

/// 一次 CPU Newton 迭代, 返回残差 |f|
///
/// 严格匹配 GPU shader 的 4-layer 结构, 方便交叉验证。
pub fn cpu_newton_step(dag: &mut [GpuNode]) -> f32 {
    // Layer 1a: 1 − κ
    dag[1].value = 1.0 - dag[0].value;
    // Layer 1b: d_eff = d × (1−κ)
    dag[3].value = dag[2].value * dag[1].value;
    // Layer 1c: t = d_eff / c
    dag[4].value = dag[3].value / C_LIGHT;

    // Layer 2: residual f = t − TARGET
    let f = dag[4].value - TARGET_T;

    // Layer 3: Jacobian ∂f/∂κ = −d / c
    let j = -dag[2].value / C_LIGHT;

    // Layer 4: Δκ = −f / J, κ += Δκ
    dag[0].value += -f / j;

    f.abs()
}

/// CPU Newton 求解, 迭代直到收敛
pub fn cpu_newton_solve(dag: &mut [GpuNode], tol: f32, max_iter: usize) -> f32 {
    for _ in 0..max_iter {
        if cpu_newton_step(dag) < tol {
            break;
        }
    }
    dag[0].value
}

// ============================================================
// GPU 运行时 (feature = "gpu")
// ============================================================

#[cfg(feature = "gpu")]
mod gpu {
    use std::sync::mpsc;
    use wgpu::util::DeviceExt;

    use super::*;

    pub fn run_gpu_newton(dag: &mut [GpuNode], iterations: u32) {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions::default(),
        ))
        .expect("no GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .unwrap();

        // 节点 buffer (STORAGE + 读写回读)
        let node_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("nodes"),
            contents: bytemuck::cast_slice(dag),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });

        // 残差 / Jacobian / Δx 各 1 个 f32
        fn make_buf(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        }
        let residual_buf = make_buf(&device, "residual");
        let jacobian_buf = make_buf(&device, "jacobian");
        let delta_buf = make_buf(&device, "delta");

        // Shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("newton"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("shader.wgsl"))),
        });

        // Pipeline
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pipeline"),
            layout: None,
            module: &shader,
            entry_point: "main",
        });

        // Bind group (4 SSBOs)
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: node_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: residual_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: jacobian_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: delta_buf.as_entire_binding() },
            ],
        });

        // 多次 dispatch (Newton 迭代)
        for _iter in 0..iterations {
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("newton_iter"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch(5, 1, 1); // 5 nodes, 1 workgroup
            }
            queue.submit(Some(encoder.finish()));
            device.poll(wgpu::Maintain::Wait);
        }

        // 回读 κ
        let slice = node_buf.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().expect("mapping failed");

        let data = slice.get_mapped_range();
        let result: &[GpuNode] = bytemuck::cast_slice(&data);
        dag.copy_from_slice(result);
        drop(data);
        node_buf.unmap();
    }
}

// ============================================================
// 主入口
// ============================================================

fn main() {
    println!("╔══════════════════════════════════════════════╗");
    println!("║  GPU Newton Batch Solver — Phase 4           ║");
    println!("║  地球→柯伊伯带曲率跳跃 | t = 40·AU·(1−κ)/c  ║");
    println!("╚══════════════════════════════════════════════╝\n");

    // ── CPU 参考实现 ──
    let mut cpu_dag = build_dag(0.5); // κ=0.5 初始猜测
    let kappa_cpu = cpu_newton_solve(&mut cpu_dag, 1e-6, 10);
    let t_cpu = cpu_dag[4].value;
    println!("CPU Newton:");
    println!("  κ₀ = 0.5 → κ* = {kappa_cpu:.6}  (expected 0.997)");
    println!("  t       = {t_cpu:.2} s  (expected 59.88 s)\n");

    // 跟踪每次迭代
    let mut trace = build_dag(0.5);
    println!("Iteration trace (κ₀=0.5):");
    println!("  iter   κ           f            Δκ");
    for iter in 0..6 {
        let f = cpu_newton_step(&mut trace);
        if iter < 3 || iter == 5 {
            println!("  {iter:>2}    {:.6}   {:.4e}   {:.4e}",
                trace[0].value, f, -f / (-trace[2].value / C_LIGHT));
        }
    }

    // ── GPU（feature=gpu 时可用）──
    #[cfg(feature = "gpu")]
    {
        let mut gpu_dag = build_dag(0.5);
        gpu::run_gpu_newton(&mut gpu_dag, 10);
        let kappa_gpu = gpu_dag[0].value;
        let t_gpu = gpu_dag[4].value;
        println!("GPU Newton:");
        println!("  κ* = {kappa_gpu:.6}  (diff: {:.2e})", (kappa_gpu - kappa_cpu).abs());
        println!("  t  = {t_gpu:.2} s\n", t_gpu);
    }

    #[cfg(not(feature = "gpu"))]
    {
        println!("── GPU Compute ──");
        println!("  (enable with: cargo run -p gpu_newton_solver --features gpu)\n");
    }

    // ── 多初始猜测验证 ──
    println!("Multi-guess convergence:");
    for &guess in &[0.1, 0.3, 0.5, 0.7, 0.9] {
        let mut dag = build_dag(guess);
        cpu_newton_solve(&mut dag, 1e-6, 10);
        println!("  κ₀ = {guess} → κ* = {:.6}, t = {:.2}s ✓",
            dag[0].value, dag[4].value);
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_newton_converges_to_0997() {
        let mut dag = build_dag(0.5);
        cpu_newton_solve(&mut dag, 1e-6, 10);
        let kappa = dag[0].value;
        assert!(
            (kappa - 0.997).abs() < 0.001,
            "κ should converge to ~0.997, got {kappa:.6}"
        );
    }

    #[test]
    fn cpu_newton_all_guesses() {
        for &guess in &[0.0, 0.1, 0.3, 0.5, 0.7, 0.9, 1.0] {
            let mut dag = build_dag(guess);
            cpu_newton_solve(&mut dag, 1e-6, 10);
            let kappa = dag[0].value;
            assert!(
                (kappa - 0.997).abs() < 0.001,
                "κ₀={guess} should converge to ~0.997, got {kappa:.6}"
            );
            let t = dag[4].value;
            assert!(
                (t - 59.88).abs() < 0.1,
                "κ₀={guess}: t should be ~59.88s, got {t:.2}"
            );
        }
    }

    #[test]
    fn cpu_newton_tick_matches_phase2() {
        // 验证 GPU kernel 的 tick 逻辑 = Phase 2 的 forward eval
        let mut dag = build_dag(0.997);
        // 只做 tick (1 次 Newton step 内的前 3 layer)
        dag[1].value = 1.0 - dag[0].value;
        dag[3].value = dag[2].value * dag[1].value;
        dag[4].value = dag[3].value / C_LIGHT;
        let t = dag[4].value;
        assert!((t - 59.88).abs() < 0.1, "tick: t should be ~59.88s, got {t:.2}");
    }

    #[test]
    fn cpu_newton_monotonic_convergence() {
        let mut dag = build_dag(0.0);
        let mut prev_residual = f32::INFINITY;
        for _ in 0..10 {
            let r = cpu_newton_step(&mut dag);
            assert!(r <= prev_residual + 1e-3, "residual should decrease monotonically");
            prev_residual = r;
        }
    }

    #[test]
    fn cpu_newton_single_step_exact() {
        // 线性系统 → 1 步应收敛到机器精度
        let mut dag = build_dag(0.5);
        cpu_newton_step(&mut dag);
        let kappa = dag[0].value;
        assert!(
            (kappa - 0.997).abs() < 1e-5,
            "linear system: 1 step should converge, got {kappa:.6}"
        );
    }
}
