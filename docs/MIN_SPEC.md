# SymNebula 最低配置 / Minimum Hardware Specification

## 中文

### 守门人配置 (Baseline)

最低配置目标为一台 2014 年主流台式机，确保 SymNebula 在此配置上能正常运行。

| 部件 | 型号 | 说明 |
|------|------|------|
| CPU | Intel i5-4460 (Haswell, 4C4T) | **要求支持 AVX2 指令集**，数学运算的核心保障 |
| GPU | NVIDIA GTX 760 4G | 仅用于可视化渲染（OpenGL），**不参与数值计算** |
| 内存 | 8GB DDR3 1600MHz | 低带宽场景，需紧凑内存布局 |
| 存储 | 任意 | 全静态编译，单 exe 可 U 盘直插运行 |

### 工程纪律

1. **Tick 弹性** — 低配机上 Tick 跑得慢但结果分毫不差。不做实时保证。
2. **紧凑内存** — Expr 树用连续内存（Box<[Expr]>），SlotMap 池化管理，减少 DDR3 带宽压力。
3. **CPU 纯算** — 计算全部留在 Rust 内核。GPU 退化为画布，仅做可视化。
4. **IEEE-754 严格** — 跨设备浮点结果一致，编译时强制启用严格模式。
5. **全静态编译** — x86_64-pc-windows-msvc + 静态链接 C 运行库，单 exe 部署。

### 用户预期管理

- **内核负荷指示器** — UI 角落显示当前 Tick 进度，低配用户能直观感知"它在算"。
- **采样丢帧** — Python 界面保持 60fps 响应（缩放/平移），与后台计算帧率完全解耦。

---

## English

### Baseline (Gatekeeper)

The minimum target is a mainstream desktop PC from 2014. SymNebula must run correctly on this hardware.

| Component | Model | Notes |
|-----------|-------|-------|
| CPU | Intel i5-4460 (Haswell, 4C4T) | **AVX2 required** for efficient math operations |
| GPU | NVIDIA GTX 760 4G | Visualization only (OpenGL). **No GPU compute.** |
| RAM | 8GB DDR3 1600MHz | Low bandwidth — compact memory layout required |
| Storage | Any | Static linking, single exe, runs from USB stick |

### Engineering Constraints

1. **Tick Elasticity** — Slow ticks on low-end hardware produce identical results. No real-time guarantee.
2. **Compact Memory** — Expr trees in contiguous storage (Box<[Expr]>), SlotMap pooling to reduce DDR3 bandwidth pressure.
3. **CPU-Only Compute** — All computation stays in the Rust kernel. GPU is a render-only canvas.
4. **IEEE-754 Strict** — Deterministic floating-point across devices. Enable strict mode at compile time.
5. **Static Linking** — `x86_64-pc-windows-msvc` + static C runtime. Single exe, zero dependencies.

### User Experience

- **Kernel load indicator** — UI shows tick progress. Low-end users see "it's working", not "it's frozen".
- **Decoupled framerate** — UI stays at 60fps (pan/zoom) independent of compute tick rate.
