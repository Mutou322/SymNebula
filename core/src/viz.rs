/// Tick 状态彩条可视化
///
/// ASCII 表格风格，直接在终端查看每 Tick 的节点状态波形 + 输出值。
///
/// 用法:
/// ```ignore
/// let mut viz = TickDisplay::new();
/// for _ in 0..N {
///     scheduler.step();
///     viz.record(&scheduler);
/// }
/// viz.render();
/// ```

use crate::engine::Scheduler;
use crate::state::NodeState;

/// 单个节点的 Tick 快照
#[derive(Debug, Clone)]
struct NodeSnapshot {
    id: usize,
    label: String,
    status: NodeState,
    outputs: Vec<(String, f64)>,
    is_dynamic: bool,
}

/// 单个 Tick 的记录
#[derive(Debug, Clone)]
struct TickRecord {
    tick: usize,
    nodes: Vec<NodeSnapshot>,
}

/// Tick 历史记录器
pub struct TickDisplay {
    records: Vec<TickRecord>,
    nid_order: Vec<usize>,
}

impl TickDisplay {
    pub fn new() -> Self {
        TickDisplay {
            records: Vec::new(),
            nid_order: Vec::new(),
        }
    }

    /// 记录当前 scheduler 状态的快照
    pub fn record(&mut self, scheduler: &Scheduler) {
        let tick = scheduler.tick;
        let mut nodes = Vec::new();

        for node in &scheduler.graph.nodes {
            let mut outputs: Vec<(String, f64)> = scheduler
                .env
                .iter()
                .filter(|((nid, _), _)| *nid == node.id)
                .map(|((_, sym), val)| (sym.clone(), *val))
                .collect();
            outputs.sort_by(|a, b| a.0.cmp(&b.0));

            // 标签：nid + 简短公式名
            let formula_raw = format!("{}", node.formula);
            let formula_short = if formula_raw.len() > 16 {
                format!("{}…", &formula_raw[..15])
            } else {
                formula_raw
            };
            let label = format!("n{}:{}", node.id, formula_short);

            nodes.push(NodeSnapshot {
                id: node.id,
                label,
                status: node.state.clone(),
                outputs,
                is_dynamic: node.is_dynamic,
            });
        }

        if self.nid_order.is_empty() {
            self.nid_order = nodes.iter().map(|n| n.id).collect();
        }

        self.records.push(TickRecord { tick, nodes });
    }

    fn status_char(state: &NodeState) -> &'static str {
        match state {
            NodeState::Green => "G",
            NodeState::Yellow => "Y",
            NodeState::Purple => "P",
            NodeState::Gray => "-",
        }
    }

    /// 波形条：用 ░▒▓ 三级灰度表示相对值
    fn waveform(value: f64, max_val: f64, width: usize) -> String {
        if !value.is_finite() {
            return "░".repeat(width);
        }
        let ratio = if max_val <= 0.0 {
            0.0
        } else {
            (value.abs() / max_val).clamp(0.0, 1.0)
        };
        (0..width)
            .map(|i| {
                let pos = i as f64 / width as f64;
                if pos < ratio {
                    if pos < ratio * 0.5 {
                        '░'
                    } else if pos < ratio * 0.85 {
                        '▒'
                    } else {
                        '▓'
                    }
                } else {
                    '░'
                }
            })
            .collect()
    }

    /// 全 Tick 中某个输出的最大值
    fn global_max(&self, node_id: usize, sym: &str) -> f64 {
        let mut max_val = 0.0_f64;
        for rec in &self.records {
            if let Some(ns) = rec.nodes.iter().find(|n| n.id == node_id) {
                for (s, v) in &ns.outputs {
                    if s == sym {
                        max_val = max_val.max(v.abs());
                    }
                }
            }
        }
        max_val
    }

    /// 渲染完整表格
    pub fn render(&self) {
        if self.records.is_empty() {
            println!("  (无记录)");
            return;
        }

        let ticks = self.records.len();
        let max_cols = 10;
        let start = if ticks > max_cols { ticks - max_cols } else { 0 };
        let displayed: Vec<&TickRecord> = self.records[start..].iter().collect();
        let ncol = displayed.len();

        const LABEL_W: usize = 22;
        const BAR_W: usize = 8;

        // 表头
        let header: String = displayed
            .iter()
            .map(|r| format!("{:>6}", r.tick))
            .collect::<Vec<_>>()
            .join(" ");
        println!("  {:>width$} {}", "Tick", header, width = LABEL_W);
        println!("  {:->width$} {}", "", "-".repeat(ncol * 7 - 1), width = LABEL_W);

        for &nid in &self.nid_order {
            let label = self
                .records
                .iter()
                .find_map(|r| r.nodes.iter().find(|n| n.id == nid).map(|n| n.label.clone()))
                .unwrap_or_else(|| format!("n{}", nid));

            let is_dyn = self
                .records
                .iter()
                .find_map(|r| r.nodes.iter().find(|n| n.id == nid).map(|n| n.is_dynamic))
                .unwrap_or(false);

            let dyn_mark = if is_dyn { " ⚡" } else { "   " };
            println!("  {:>width$}{}", label, dyn_mark, width = LABEL_W - 3);

            // 收集所有输出符号
            let mut all_syms: Vec<String> = Vec::new();
            for rec in &displayed {
                if let Some(ns) = rec.nodes.iter().find(|n| n.id == nid) {
                    for (s, _) in &ns.outputs {
                        if !all_syms.contains(s) {
                            all_syms.push(s.clone());
                        }
                    }
                }
            }
            if all_syms.is_empty() {
                all_syms.push("output".to_string());
            }

            for sym in &all_syms {
                let max_val = self.global_max(nid, sym);
                let wave: String = displayed
                    .iter()
                    .map(|rec| {
                        if let Some(ns) = rec.nodes.iter().find(|n| n.id == nid) {
                            if let Some((_, val)) = ns.outputs.iter().find(|(s, _)| s == sym) {
                                Self::waveform(*val, max_val.max(1e-12), BAR_W)
                            } else {
                                "  ---   ".to_string()
                            }
                        } else {
                            "  ---   ".to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("    {:>8}  {}", sym, wave);
            }

            let status_line: String = displayed
                .iter()
                .map(|rec| {
                    if let Some(ns) = rec.nodes.iter().find(|n| n.id == nid) {
                        format!("  {:^3}", Self::status_char(&ns.status))
                    } else {
                        " --- ".to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            println!("    {:>8}  {}", "Status", status_line);
            println!();
        }
    }

    /// 紧凑单行状态打印
    pub fn print_tick(scheduler: &Scheduler) {
        let tick = scheduler.tick;
        let total = scheduler.graph.nodes.len();
        let green = scheduler.graph.nodes.iter().filter(|n| n.state == NodeState::Green).count();
        let yellow = scheduler.graph.nodes.iter().filter(|n| n.state == NodeState::Yellow).count();
        let purple = scheduler.graph.nodes.iter().filter(|n| n.state == NodeState::Purple).count();
        let gray = total - green - yellow - purple;

        let bar = format!(
            "{}{}{}{}",
            "G".repeat(green),
            "Y".repeat(yellow),
            "P".repeat(purple),
            "-".repeat(gray)
        );

        println!("  Tick {:>2} | {} | {:>2}G {:>2}Y {:>2}P {:>2}-", tick, bar, green, yellow, purple, gray);

        for node in &scheduler.graph.nodes {
            let c = Self::status_char(&node.state);
            let val_str = match &node.value {
                Some(v) if v.abs() > 1e-12 => format!("{: >12.4e}", v),
                Some(v) => format!("{: >12.4}", v),
                None => "        ---".to_string(),
            };
            let formula_str = format!("{}", node.formula);
            let label = if formula_str.len() > 22 {
                format!("{}…", &formula_str[..21])
            } else {
                formula_str
            };
            let dyn_mark = if node.is_dynamic { " ⚡" } else { "  " };
            println!("    n{} [{}{}] {} {}", node.id, c, dyn_mark, label, val_str);
        }
    }
}
