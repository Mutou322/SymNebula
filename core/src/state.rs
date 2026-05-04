/// 节点状态机 — 定义神经元节点的全部可能状态
///
/// Gray   — 静息/待编辑，不参与计算
/// Yellow — 激活/待定，计算中或多解等待限定条件
/// Green  — 输出有效，计算成功
/// Purple — 奇异态，数学异常（除零/维度不匹配），已安全切断

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeState {
    Gray,
    Yellow,
    Green,
    Purple,
}

impl NodeState {
    pub fn is_stable(&self) -> bool {
        matches!(self, NodeState::Green)
    }

    pub fn label(&self) -> &'static str {
        match self {
            NodeState::Gray => "gray",
            NodeState::Yellow => "yellow",
            NodeState::Green => "green",
            NodeState::Purple => "purple",
        }
    }
}
