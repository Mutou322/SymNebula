//! Phase 5 — 分布式 Cluster 并行求解器
//!
//! 概念：4 个 Cluster 并行运行 GPU Batch Newton Solver，
//! 用 tokio async 做 cluster 异步调度，
//! 用 mpsc channel 做边界节点 Δx 同步。
//!
//! 每个 cluster 运行"地球 → 柯伊伯带曲率跳跃" DAG:
//!   κ → 1−κ → d_eff = d·(1−κ) → t = d_eff/c → Newton Δκ
//!
//! 数据流:
//!   GPU compute → copy readback → tokio channel sync → 收敛检查

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

// ─── 常量 ──────────────────────────────────────────────────
const AU: f32 = 149_597_870.7;
const C_LIGHT: f32 = 299_792.458;
#[allow(dead_code)]
const TARGET: f32 = 59.88;
const N_CLUSTERS: u32 = 4;
const NODES_PER_CLUSTER: u32 = 7;
const MAX_ITER: usize = 20;
const CONV_TOL: f32 = 1e-3;

// ─── GPU Node 布局 ────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Node {
    value: f32,
    kind: u32,
    input0: u32,
    input1: u32,
}

/// 每个 cluster 的 DAG:
///   0: κ      (Input)   — Newton 变量
///   1: 1.0    (Const)   — 常量
///   2: 1−κ    (Sub)     — 1.0 − κ
///   3: d      (Const)   — 40·AU
///   4: d_eff  (Mul)     — d × (1−κ)
///   5: c      (Const)   — 光速
///   6: t      (Div)     — d_eff / c  →  residual = t − TARGET
fn build_cluster_nodes(kappa: f32) -> Vec<Node> {
    vec![
        Node { value: kappa,           kind: 0, input0: 0, input1: 0 },
        Node { value: 1.0,             kind: 1, input0: 0, input1: 0 },
        Node { value: 0.0,             kind: 2, input0: 1, input1: 0 },
        Node { value: 40.0 * AU,       kind: 1, input0: 0, input1: 0 },
        Node { value: 0.0,             kind: 3, input0: 3, input1: 2 },
        Node { value: C_LIGHT,         kind: 1, input0: 0, input1: 0 },
        Node { value: 0.0,             kind: 4, input0: 4, input1: 5 },
    ]
}

/// 4 个 cluster, 各给不同初始 κ
fn build_all_nodes() -> Vec<Node> {
    let guesses = [0.1_f32, 0.3, 0.5, 0.7];
    let mut all = Vec::with_capacity((N_CLUSTERS * NODES_PER_CLUSTER) as usize);
    for &k in &guesses {
        all.extend(build_cluster_nodes(k));
    }
    all
}

// ─── GPU 资源 ─────────────────────────────────────────────

struct GpuResources {
    buffer: wgpu::Buffer,
    staging: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

fn init_gpu() -> (wgpu::Device, wgpu::Queue, GpuResources) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(
        &wgpu::RequestAdapterOptions::default(),
    ))
    .expect("no GPU adapter available — need a GPU to run this example");

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))
    .expect("failed to create device");

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cluster_newton"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("shader.wgsl"))),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cluster_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cluster_pipeline_layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cluster_newton_pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: "main",
        compilation_options: wgpu::PipelineCompilationOptions::default(),
    });

    let nodes = build_all_nodes();
    let buffer_size = nodes.len() * std::mem::size_of::<Node>();
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cluster_buffer"),
        contents: bytemuck::cast_slice(&nodes),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cluster_staging"),
        size: buffer_size as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cluster_bind_group"),
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });

    (device, queue, GpuResources { buffer, staging, bind_group, bind_group_layout, pipeline })
}

// ─── GPU Tick ─────────────────────────────────────────────

/// 提交 GPU compute → 读回 κ 值 (同步)
fn gpu_tick(device: &wgpu::Device, queue: &wgpu::Queue, res: &GpuResources) -> [f32; N_CLUSTERS as usize] {
    let buffer_size = (N_CLUSTERS * NODES_PER_CLUSTER) as u64 * std::mem::size_of::<Node>() as u64;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cluster_tick"),
    });

    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("newton_pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&res.pipeline);
        cpass.set_bind_group(0, &res.bind_group, &[]);
        cpass.dispatch_workgroups(N_CLUSTERS, 1, 1);
    }

    encoder.copy_buffer_to_buffer(&res.buffer, 0, &res.staging, 0, buffer_size);
    queue.submit(Some(encoder.finish()));

    // Wait + map readback
    let (tx, rx) = std::sync::mpsc::channel();
    res.staging.slice(..).map_async(wgpu::MapMode::Read, move |_| {
        let _ = tx.send(());
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv().unwrap();

    let mapped = res.staging.slice(..).get_mapped_range();
    let mut kappas = [0.0_f32; N_CLUSTERS as usize];
    let node_size = std::mem::size_of::<Node>();
    for i in 0..N_CLUSTERS as usize {
        let offset = i * (NODES_PER_CLUSTER as usize) * node_size;
        kappas[i] = f32::from_ne_bytes((&mapped[offset..offset + 4]).try_into().unwrap());
    }
    drop(mapped);
    res.staging.unmap();

    kappas
}

/// 用新 κ 重建 GPU buffer + bind_group
fn write_kappas(device: &wgpu::Device, res: &mut GpuResources, kappas: &[f32]) {
    let nodes = build_all_nodes();
    let mut updated = nodes;
    for i in 0..N_CLUSTERS as usize {
        updated[i * (NODES_PER_CLUSTER as usize)] = Node {
            value: kappas[i], kind: 0, input0: 0, input1: 0,
        };
    }

    let new_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cluster_buffer_updated"),
        contents: bytemuck::cast_slice(&updated),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cluster_bg_updated"),
        layout: &res.bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: new_buffer.as_entire_binding(),
        }],
    });

    res.buffer = new_buffer;
    res.bind_group = bind_group;
}

// ─── 异步调度 + 边界同步 ────────────────────────────────

async fn run_scheduler(
    device: wgpu::Device,
    queue: wgpu::Queue,
    res: &mut GpuResources,
) {
    // tokio mpsc ring topology:
    //   tx[i]  → rx[(i+1) % N]
    //   rx[i]  ← tx[(i-1+N) % N]
    let (tx0, rx1) = tokio::sync::mpsc::channel::<f32>(1);
    let (tx1, rx2) = tokio::sync::mpsc::channel::<f32>(1);
    let (tx2, rx3) = tokio::sync::mpsc::channel::<f32>(1);
    let (tx3, rx0) = tokio::sync::mpsc::channel::<f32>(1);
    let senders = [tx0, tx1, tx2, tx3];
    let mut receivers = [rx0, rx1, rx2, rx3];

    let mut kappas = [0.1_f32, 0.3, 0.5, 0.7];

    for iter in 0..MAX_ITER {
        // ── GPU Tick (sync) ──
        let new_kappas = gpu_tick(&device, &queue, res);

        // ── tokio channel 边界同步 (ring) ──
        for i in 0..N_CLUSTERS as usize {
            // 发给邻居: 0→1, 1→2, 2→3, 3→0
            senders[i].send(new_kappas[i]).await.unwrap();
        }
        for i in 0..N_CLUSTERS as usize {
            // 从前驱接收: 0←3, 1←0, 2←1, 3←2
            let from_prev = receivers[i].recv().await.unwrap();
            kappas[i] = (new_kappas[i] + from_prev) * 0.5;
        }

        // ── 更新 GPU buffer ──
        write_kappas(&device, res, &kappas);

        // ── 收敛检查 ──
        let max_diff = kappas.iter()
            .map(|k| (k - 0.997).abs())
            .fold(0.0_f32, f32::max);

        println!(
            "iter {iter:2}: κ = [{:.6} {:.6} {:.6} {:.6}]  max_err={:.2e}",
            kappas[0], kappas[1], kappas[2], kappas[3], max_diff,
        );

        if max_diff < CONV_TOL {
            println!("✓ 收敛于迭代 {iter}");
            return;
        }
    }
    println!("✗ 未收敛 (max_iter={MAX_ITER})");
}

// ─── 主函数 ───────────────────────────────────────────────

#[tokio::main]
async fn main() {
    println!("── Phase 5 分布式 Cluster 并行求解器 ──");
    println!("Cluster DAG: κ → 1−κ → d_eff → t → Newton step");
    println!("4 clusters, ring 拓扑边界同步, tol = {CONV_TOL}");
    println!("节点 kinds: 0=Input 1=Const 2=Sub 3=Mul 4=Div\n");

    let (device, queue, mut resources) = init_gpu();
    run_scheduler(device, queue, &mut resources).await;

    println!("\n── 验证 ──");
    println!("预期: κ → 0.997, t = d·(1−κ)/c → 59.88s");
}

// ─── 测试 ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dag_build() {
        let nodes = build_cluster_nodes(0.5);
        assert_eq!(nodes.len(), NODES_PER_CLUSTER as usize);
        assert_eq!(nodes[0].value, 0.5);
        assert_eq!(nodes[1].value, 1.0);
        assert_eq!(nodes[3].value, 40.0 * AU);
        assert_eq!(nodes[5].value, C_LIGHT);
    }

    #[test]
    fn bytemuck_trait() {
        let n = Node { value: 1.0, kind: 0, input0: 0, input1: 0 };
        assert_eq!(bytemuck::bytes_of(&n).len(), 16);
    }

    #[test]
    fn analytic_solution() {
        let t = 40.0 * AU * (1.0 - 0.997) / C_LIGHT;
        assert!((t - TARGET).abs() < 1.0, "t={t} should be ~59.88");
    }
}
