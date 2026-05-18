//! Cluster GPU Batch Newton + boundary sync.
//!
//! Each `Cluster` owns a 7-node curvature DAG, a GPU storage buffer, a staging
//! buffer for readback, and a bind group that binds the full buffer to the
//! compute shader.  The shader executes one Newton tick per `tick()` call.

use wgpu::util::DeviceExt;
use bytemuck;

use crate::Node;
use crate::{KIND_INPUT, KIND_CONST, KIND_SUB, KIND_MUL, KIND_DIV, KIND_EQ, KIND_ADD};

// ─── Cluster ───────────────────────────────────────────────────────────────

pub struct Cluster {
    pub id: usize,
    /// GPU storage buffer holding the full node array (STORAGE | COPY_SRC).
    pub gpu_buffer: wgpu::Buffer,
    /// Staging buffer for readback (COPY_DST | MAP_READ).
    pub staging: wgpu::Buffer,
    /// Bind group that maps `gpu_buffer` to `@group(0) @binding(0)`.
    pub bind_group: wgpu::BindGroup,
    /// CPU-side mirror of the node array.
    pub nodes: Vec<Node>,
    /// Indices of variable nodes (currently just [0] = kappa).
    pub variable_nodes: Vec<u32>,
    /// Indices of constraint nodes (currently [6] = t).
    pub constraint_nodes: Vec<u32>,
    /// Right-hand-side values for each constraint node.
    pub constraint_rhs: Vec<f32>,
    /// Whether this cluster has converged locally.
    pub converged: bool,
    /// Latest local residual norm (L2 over constraints).
    pub local_residual: f32,
}

impl Cluster {
    /// Create a cluster with the given node data and GPU resources.
    ///
    /// The GPU buffer is initialised from `nodes` via `create_buffer_init`.
    /// The bind group binds the full buffer at binding 0.  Each cluster
    /// addresses its own portion via `offset = id * NODES_PER_CLUSTER`
    /// inside the WGSL shader.
    pub fn new(
        id: usize,
        nodes: Vec<Node>,
        variable_nodes: Vec<u32>,
        constraint_nodes: Vec<u32>,
        constraint_rhs: Vec<f32>,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let buffer_size = (nodes.len() * std::mem::size_of::<Node>()) as u64;

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("cluster{}_buf", id)),
            contents: bytemuck::cast_slice(&nodes),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("cluster{}_staging", id)),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("cluster{}_bg", id)),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self {
            id,
            gpu_buffer: buffer,
            staging,
            bind_group,
            nodes,
            variable_nodes,
            constraint_nodes,
            constraint_rhs,
            converged: false,
            local_residual: 0.0,
        }
    }

    /// Run one GPU Newton tick.
    ///
    /// Steps:
    /// 1. Encode a compute pass that dispatches one workgroup.
    /// 2. Copy the GPU buffer to the staging buffer.
    /// 3. Submit to the queue.
    /// 4. Map the staging buffer, read back kappa (node 0 value).
    /// 5. Update the CPU-side node 0.
    /// 6. Re-evaluate all dependent nodes on the CPU (`dag_tick`).
    /// 7. Compute the L2 residual norm across constraint nodes.
    ///
    /// Returns the local residual norm.
    pub fn tick(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::ComputePipeline,
    ) -> f32 {
        let buffer_size = (self.nodes.len() * std::mem::size_of::<Node>()) as u64;

        // 1 – encode compute pass
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some(&format!("cluster{}_encoder", self.id)),
        });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(&format!("cluster{}_compute", self.id)),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &self.bind_group, &[]);
            cpass.dispatch_workgroups(1, 1, 1);
        }

        // 2 – copy GPU buffer → staging
        encoder.copy_buffer_to_buffer(&self.gpu_buffer, 0, &self.staging, 0, buffer_size);

        // 3 – submit
        queue.submit([encoder.finish()]);

        // 4 – map staging & read back κ (first field of node 0)
        let id = self.id;
        let buffer_slice = self.staging.slice(..);
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            if let Err(e) = result {
                eprintln!("cluster{id}: staging map failed: {e:?}");
            }
        });
        device.poll(wgpu::Maintain::Wait);

        let kappa = {
            let view = buffer_slice.get_mapped_range();
            // view: BufferView derefs to [u8] — cast to &[Node] and take first
            let nodes_from_gpu: &[Node] = bytemuck::cast_slice(&view[..]);
            nodes_from_gpu[0].value
        };
        // BufferView dropped → buffer unmapped

        // 5 – update CPU-side κ
        self.nodes[0].value = kappa;

        // 6 – CPU-side DAG evaluation (forward pass)
        dag_tick(&mut self.nodes);

        // 7 – compute residual norm
        let residual = compute_residuals(&self.nodes, &self.constraint_nodes, &self.constraint_rhs);
        self.local_residual = residual;
        self.converged = residual < 1e-6;

        residual
    }

    /// Update CPU-side nodes with received boundary values.
    ///
    /// `values` contains `(node_index, new_value)` pairs received from
    /// neighbouring clusters through the boundary sync ring.
    pub fn apply_boundary(&mut self, values: &[(u32, f32)]) {
        for &(node_id, val) in values {
            if let Some(n) = self.nodes.get_mut(node_id as usize) {
                n.value = val;
            }
        }
    }

    /// Rebuild the GPU buffer and bind group from the current CPU-side nodes.
    ///
    /// This is called after boundary synchronisation so the next GPU tick
    /// starts from the blended values.  The staging buffer is also resized
    /// if the node count has changed.
    pub fn rebuild_gpu_buffer(&mut self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout) {
        let buffer_size = (self.nodes.len() * std::mem::size_of::<Node>()) as u64;

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("cluster{}_rebuild", self.id)),
            contents: bytemuck::cast_slice(&self.nodes),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        // Recreate staging buffer in case node count changed.
        self.staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("cluster{}_staging_rebuild", self.id)),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("cluster{}_bg_rebuild", self.id)),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        self.gpu_buffer = buffer;
    }
}

// ─── Private helpers ───────────────────────────────────────────────────────

/// Evaluate every non-input, non-constant node in a single forward pass.
///
/// Assumes the node array is topologically ordered (each node only references
/// nodes with a lower index).  This holds for the 7-node curvature DAG.
fn dag_tick(nodes: &mut [Node]) {
    for i in 0..nodes.len() {
        match nodes[i].kind {
            KIND_INPUT | KIND_CONST => { /* values are set externally */ }
            KIND_SUB => {
                let a = nodes[nodes[i].input0 as usize].value;
                let b = nodes[nodes[i].input1 as usize].value;
                nodes[i].value = a - b;
            }
            KIND_MUL => {
                let a = nodes[nodes[i].input0 as usize].value;
                let b = nodes[nodes[i].input1 as usize].value;
                nodes[i].value = a * b;
            }
            KIND_DIV => {
                let a = nodes[nodes[i].input0 as usize].value;
                let b = nodes[nodes[i].input1 as usize].value;
                nodes[i].value = if b.abs() < 1e-12 { a } else { a / b };
            }
            KIND_ADD => {
                let a = nodes[nodes[i].input0 as usize].value;
                let b = nodes[nodes[i].input1 as usize].value;
                nodes[i].value = a + b;
            }
            KIND_EQ => {
                nodes[i].value = nodes[nodes[i].input0 as usize].value;
            }
            _ => {}
        }
    }
}

/// Compute L2 norm of residuals across all constraint nodes.
///
/// residual_i = nodes[constraint_nodes[i]].value - constraint_rhs[i]
fn compute_residuals(
    nodes: &[Node],
    constraint_nodes: &[u32],
    constraint_rhs: &[f32],
) -> f32 {
    let mut sum_sq = 0.0f32;
    for (i, &node_idx) in constraint_nodes.iter().enumerate() {
        let residual = nodes[node_idx as usize].value - constraint_rhs[i];
        sum_sq += residual * residual;
    }
    sum_sq.sqrt()
}
