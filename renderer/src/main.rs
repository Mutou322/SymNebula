// SymNebula Pure wgpu Renderer
//
// 目标：
// - 不依赖 Bevy
// - 纯 GPU Renderer
// - Native Desktop
// - 支持百万级 Nebula Nodes
// - Runtime → GPU Snapshot
// - Green / Yellow / Purple 状态渲染

use std::iter;
use wgpu::util::DeviceExt;
use winit::{
    event::*,
    event_loop::EventLoop,
    window::Window,
};

// ============================================================
// Node Status
// ============================================================

#[repr(u32)]
#[derive(Clone, Copy)]
enum NodeStatus {
    Green = 0,
    Yellow = 1,
    Purple = 2,
}

// ============================================================
// GPU Node Instance
// ============================================================

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct NodeInstance {
    position: [f32; 3],
    energy: f32,
    status: u32,
    _padding: [u32; 3],
}

// ============================================================
// Renderer State
// ============================================================

struct State {
    window: &'static winit::window::Window,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    node_buffer: wgpu::Buffer,
    node_count: u32,
    size: winit::dpi::PhysicalSize<u32>,
}

impl State {
    async fn new(window: &'static winit::window::Window) -> Self {
        let size = window.inner_size();

        // ----------------------------------------------------
        // WGPU Init
        // ----------------------------------------------------

        let instance = wgpu::Instance::default();

        let surface = instance.create_surface(window).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);

        let format = surface_caps.formats[0];

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        // ----------------------------------------------------
        // Generate Nebula Nodes
        // ----------------------------------------------------

        let mut nodes = vec![];

        for x in -200..200 {
            for z in -200..200 {
                let energy = ((x as f32 * 0.03).sin() + 1.0) * 0.5;

                let status = if energy > 0.7 {
                    NodeStatus::Green
                } else if energy > 0.3 {
                    NodeStatus::Yellow
                } else {
                    NodeStatus::Purple
                };

                nodes.push(NodeInstance {
                    position: [x as f32, energy * 10.0, z as f32],
                    energy,
                    status: status as u32,
                    _padding: [0; 3],
                });
            }
        }

        let node_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Node Buffer"),
                contents: bytemuck::cast_slice(&nodes),
                usage: wgpu::BufferUsages::VERTEX,
            });

        // ----------------------------------------------------
        // Shader
        // ----------------------------------------------------

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Nebula Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shader.wgsl").into(),
            ),
        });

        // ----------------------------------------------------
        // Pipeline Layout
        // ----------------------------------------------------

        let pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Pipeline Layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });

        // ----------------------------------------------------
        // Render Pipeline
        // ----------------------------------------------------

        let render_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Render Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<NodeInstance>()
                            as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                offset: 0,
                                shader_location: 0,
                                format: wgpu::VertexFormat::Float32x3,
                            },
                            wgpu::VertexAttribute {
                                offset: 16,
                                shader_location: 1,
                                format: wgpu::VertexFormat::Float32,
                            },
                            wgpu::VertexAttribute {
                                offset: 20,
                                shader_location: 2,
                                format: wgpu::VertexFormat::Uint32,
                            },
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::PointList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            });

        Self {
            window,
            surface,
            device,
            queue,
            config,
            render_pipeline,
            node_buffer,
            node_count: nodes.len() as u32,
            size,
        }
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    // ========================================================
    // Render
    // ========================================================

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass =
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Render Pass"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        },
                    )],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_vertex_buffer(0, self.node_buffer.slice(..));
            render_pass.draw(0..1, 0..self.node_count);
        }

        self.queue.submit(iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

// ============================================================
// Main
// ============================================================

fn main() {
    let event_loop = EventLoop::new().unwrap();

    let window = Box::leak(Box::new(
        event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("SymNebula Pure GPU Renderer"),
            )
            .unwrap(),
    ));

    let mut state = pollster::block_on(State::new(window));

    event_loop
        .run(move |event, target| match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => target.exit(),

            Event::AboutToWait => {
                state.request_redraw();
            }

            Event::WindowEvent {
                event: WindowEvent::RedrawRequested,
                ..
            } => {
                state.render().unwrap();
            }

            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                state.resize(size);
            }

            _ => {}
        })
        .unwrap();
}
