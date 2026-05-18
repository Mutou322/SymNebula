#[derive(Debug, Clone, Copy)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Eq,
    Sin,
    Cos,
}

impl BinaryOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Pow => "^",
            BinaryOp::Eq => "=",
            BinaryOp::Sin => "sin",
            BinaryOp::Cos => "cos",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    Neg,
}

impl UnaryOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
        }
    }
}
