/// Tick 状态彩条可视化 — 完整整合版
///
/// ASCII 表格风格，直接在终端查看每 Tick 的节点状态波形：
/// G=绿 Y=黄 P=紫 -=灰  * 标记首次收敛（转为 Green）的 Tick。
/// 自动限制最多显示 12 个 Tick 列。
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

use crate::ast::Expr;
use crate::engine::Scheduler;
use crate::state::NodeState;

/// 最多显示的 Tick 列数
const MAX_COLS: usize = 12;

/// 单个变量的历史状态序列
#[derive(Debug, Clone)]
struct VarHistory {
    symbol: String,
    statuses: Vec<NodeState>,
}

/// 单个节点的历史记录
#[derive(Debug, Clone)]
struct NodeHistory {
    id: usize,
    label: String,
    is_dynamic: bool,
    vars: Vec<VarHistory>,
}

/// Tick 状态彩条记录器
pub struct TickDisplay {
    nodes: Vec<NodeHistory>,
    ticks: Vec<usize>,
    nid_order: Vec<usize>,
}

/// 移除表达式字符串中冗余的外层括号。
/// 反复剥除最外层匹配的括号对，直到不再被括号包裹。
fn strip_outer_parens(s: &str) -> String {
    let trimmed = s.trim();
    if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
        return trimmed.to_string();
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut depth = 0i32;
    for c in inner.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return trimmed.to_string();
        }
    }
    if depth == 0 {
        strip_outer_parens(inner)
    } else {
        trimmed.to_string()
    }
}

/// 紧凑格式化表达式，用于节点标签。
/// - Number: 大/小值用科学计数法，整数用整数格式，中等值截断小数
/// - 其他: Display 后剥除外层冗余括号
fn format_expr_compact(expr: &Expr, max_chars: usize) -> String {
    let s = match expr {
        Expr::Number(n) => {
            if *n == 0.0 {
                "0".to_string()
            } else if n.abs() >= 1e10 || n.abs() < 1e-4 {
                let e = format!("{:.4e}", n);
                // 去掉尾随的 "e0"
                if e.ends_with("e0") || e.ends_with("e+00") {
                    e[..e.len() - 2].to_string()
                } else {
                    e.replace("e+", "e")
                }
            } else if n.fract() == 0.0 && n.is_finite() {
                format!("{}", *n as i64)
            } else {
                let s = format!("{:.6}", n);
                let trimmed = s
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string();
                if trimmed.is_empty() { "0".to_string() } else { trimmed }
            }
        }
        other => {
            let raw = format!("{}", other);
            strip_outer_parens(&raw)
        }
    };
    if s.len() > max_chars {
        format!("{}…", &s[..max_chars - 1])
    } else {
        s
    }
}

impl TickDisplay {
    pub fn new() -> Self {
        TickDisplay {
            nodes: Vec::new(),
            ticks: Vec::new(),
            nid_order: Vec::new(),
        }
    }

    /// 记录当前 scheduler 的快照
    pub fn record(&mut self, scheduler: &Scheduler) {
        let tick = scheduler.tick;
        self.ticks.push(tick);

        if self.nid_order.is_empty() {
            self.nid_order = scheduler.graph.nodes.iter().map(|n| n.id).collect();
        }

        for &nid in &self.nid_order {
            let node = match scheduler.graph.nodes.iter().find(|n| n.id == nid) {
                Some(n) => n,
                None => continue,
            };

            let nh_idx = match self.nodes.iter().position(|n| n.id == nid) {
                Some(idx) => idx,
                None => {
                    let label = format!("n{}:{}", node.id, format_expr_compact(&node.formula, 20));
                    self.nodes.push(NodeHistory {
                        id: nid,
                        label,
                        is_dynamic: node.is_dynamic,
                        vars: Vec::new(),
                    });
                    self.nodes.len() - 1
                }
            };

            let nh = &mut self.nodes[nh_idx];

            // 从 env 收集该节点的输出符号
            let mut outputs: Vec<(String, f64)> = scheduler
                .env
                .iter()
                .filter(|((nid2, _), _)| *nid2 == nid)
                .map(|((_, sym), val)| (sym.clone(), *val))
                .collect();
            outputs.sort_by(|a, b| a.0.cmp(&b.0));

            if outputs.is_empty() {
                let val = scheduler.get_value(nid, "output").unwrap_or(0.0);
                outputs.push(("output".to_string(), val));
            }

            let current_state = node.state.clone();

            for (sym, _val) in &outputs {
                match nh.vars.iter().position(|v| v.symbol == *sym) {
                    Some(idx) => {
                        nh.vars[idx].statuses.push(current_state.clone());
                    }
                    None => {
                        let pad_count = self.ticks.len() - 1;
                        let mut statuses = vec![NodeState::Gray; pad_count];
                        statuses.push(current_state.clone());
                        nh.vars.push(VarHistory {
                            symbol: sym.clone(),
                            statuses,
                        });
                    }
                }
            }
        }
    }

    fn status_char(state: &NodeState) -> &'static str {
        match state {
            NodeState::Green => "G",
            NodeState::Yellow => "Y",
            NodeState::Purple => "P",
            NodeState::Gray => "-",
        }
    }

    /// 首个 Green 在完整状态序列中的下标
    fn first_green_idx(statuses: &[NodeState]) -> Option<usize> {
        statuses.iter().position(|s| *s == NodeState::Green)
    }

    /// 渲染完整表格 — G/Y/P 横向彩条 + Commit 标记 *
    pub fn render(&self) {
        if self.ticks.is_empty() {
            println!("  (无记录)");
            return;
        }

        let total_ticks = self.ticks.len();
        let start = if total_ticks > MAX_COLS { total_ticks - MAX_COLS } else { 0 };
        let displayed_ticks = &self.ticks[start..];
        let ncol = displayed_ticks.len();

        const LABEL_W: usize = 24;
        const COL_W: usize = 5;

        // 表头
        let header: String = displayed_ticks
            .iter()
            .map(|t| format!("{:>width$}", t, width = COL_W))
            .collect::<Vec<_>>()
            .join(" ");
        println!("  {:>width$}  {}", "Tick →", header, width = LABEL_W);
        let sep_len = ncol * (COL_W + 1) - 1;
        println!("  {:->width$}  {}", "", "-".repeat(sep_len), width = LABEL_W);

        for &nid in &self.nid_order {
            let nh = match self.nodes.iter().find(|n| n.id == nid) {
                Some(n) => n,
                None => continue,
            };

            // 节点标签行
            let dyn_mark = if nh.is_dynamic { " ⚡" } else { "   " };
            println!("  {:>width$}{}", nh.label, dyn_mark, width = LABEL_W - 3);

            // 逐变量状态行
            for vh in &nh.vars {
                let sliced = &vh.statuses[start..];
                let full_conv = Self::first_green_idx(&vh.statuses);
                let visible_conv = full_conv.and_then(|idx| {
                    if idx >= start { Some(idx - start) } else { None }
                });

                let line: String = sliced
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let c = Self::status_char(s);
                        if visible_conv == Some(i) {
                            format!("{:>width$}", format!("{}*", c), width = COL_W)
                        } else {
                            format!("{:>width$}", c, width = COL_W)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("    {:>width$}  {}", vh.symbol, line, width = LABEL_W - 4);
            }

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
            let formula_label = format_expr_compact(&node.formula, 22);
            let dyn_mark = if node.is_dynamic { " ⚡" } else { "  " };
            println!("    n{} [{}{}] {} {}", node.id, c, dyn_mark, formula_label, val_str);
        }
    }
}
