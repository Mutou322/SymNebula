/// 表达式语法树与符号引擎
///
/// 纯标准库实现，双栈解析器（Dijkstra's Two-Stack Algorithm）。
/// 支持 + - * / ^ 和括号 ()，隐式乘法，等式约束。
/// 除法 a/b 在解析时转化为 a * b^(-1)，统一代数形式。

use std::collections::HashMap;

// ============================================================
// AST 节点类型
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Symbol(String),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    /// 除法在解析时转化为 Mul + Pow：a/b → a * b^(-1)
    Div(Box<Expr>, Box<Expr>),
    Pow(Box<Expr>, Box<Expr>),
    /// 等式约束：左右两侧必须相等
    Eq(Box<Expr>, Box<Expr>),
    /// 一元负号：-expr
    Neg(Box<Expr>),
}

impl Expr {
    /// 提取表达式中的所有自由符号（变量）
    pub fn symbols(&self) -> Vec<String> {
        let mut v = Vec::new();
        self.collect_symbols(&mut v);
        v.sort();
        v.dedup();
        v
    }

    fn collect_symbols(&self, acc: &mut Vec<String>) {
        match self {
            Expr::Number(_) => {}
            Expr::Symbol(s) => acc.push(s.clone()),
            Expr::Neg(a) => a.collect_symbols(acc),
            Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Mul(a, b)
            | Expr::Div(a, b)
            | Expr::Pow(a, b)
            | Expr::Eq(a, b) => {
                a.collect_symbols(acc);
                b.collect_symbols(acc);
            }
        }
    }

    /// 给定变量值绑定，求表达式数值
    pub fn eval(&self, env: &HashMap<String, f64>) -> Result<f64, String> {
        match self {
            Expr::Number(n) => Ok(*n),
            Expr::Symbol(s) => env
                .get(s)
                .copied()
                .ok_or(format!("未定义符号: {}", s)),
            Expr::Neg(a) => Ok(-a.eval(env)?),
            Expr::Add(a, b) => Ok(a.eval(env)? + b.eval(env)?),
            Expr::Sub(a, b) => Ok(a.eval(env)? - b.eval(env)?),
            Expr::Mul(a, b) => Ok(a.eval(env)? * b.eval(env)?),
            Expr::Div(a, b) => {
                let denom = b.eval(env)?;
                if denom == 0.0 {
                    Err("除零错误".into())
                } else {
                    Ok(a.eval(env)? / denom)
                }
            }
            Expr::Pow(a, b) => {
                let base = a.eval(env)?;
                let exp = b.eval(env)?;
                if base == 0.0 && exp < 0.0 {
                    Err("除零错误: 0 的负数次方".into())
                } else {
                    Ok(base.powf(exp))
                }
            }
            Expr::Eq(_, _) => Err("Eq 不能直接求值".into()),
        }
    }
}

// ============================================================
// 词法单元（Token）
// ============================================================

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Symbol(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
}

// ============================================================
// 运算符枚举（用于 Shunting-Yard op_stack）
// ============================================================

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    UnaryMinus,
    LParen,
}

// ============================================================
// 词法分析器（Lexer）
// ============================================================

struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    fn new(input: &str) -> Self {
        Lexer {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.advance();
        }
    }

    fn read_number(&mut self) -> f64 {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s.parse().unwrap_or(0.0)
    }

    fn read_symbol(&mut self) -> String {
        let mut s = String::new();
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        // 如果符号长度 > 1 且全为字母，只取第一个字符（隐式乘法拆包）
        if s.len() > 1 && s.chars().all(|c| c.is_ascii_alphabetic()) {
            self.pos = start + 1;
            s.chars().next().unwrap().to_string()
        } else {
            s
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = vec![];
        while let Some(c) = {
            self.skip_ws();
            self.peek()
        } {
            let token = match c {
                '+' => { self.advance(); Token::Plus }
                '-' => { self.advance(); Token::Minus }
                '*' => { self.advance(); Token::Star }
                '/' => { self.advance(); Token::Slash }
                '^' => { self.advance(); Token::Caret }
                '(' => { self.advance(); Token::LParen }
                ')' => { self.advance(); Token::RParen }
                _ if c.is_ascii_digit() => Token::Number(self.read_number()),
                _ if c.is_alphabetic() || c == '_' => Token::Symbol(self.read_symbol()),
                _ => {
                    self.advance();
                    continue;
                }
            };
            tokens.push(token);
        }
        Ok(tokens)
    }
}

// ============================================================
// 隐式乘法插入
// ============================================================

fn insert_implicit_mul(tokens: Vec<Token>) -> Vec<Token> {
    let mut result = vec![];
    for i in 0..tokens.len() {
        if i > 0 {
            let prev = &tokens[i - 1];
            let curr = &tokens[i];
            let prev_ok = matches!(prev, Token::Number(_) | Token::Symbol(_) | Token::RParen);
            let curr_ok = matches!(curr, Token::Number(_) | Token::Symbol(_) | Token::LParen);
            if prev_ok && curr_ok {
                result.push(Token::Star);
            }
        }
        result.push(tokens[i].clone());
    }
    result
}

// ============================================================
// 双栈解析器（Shunting-Yard 输出 AST）
// ============================================================

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    op_stack: Vec<Op>,
    output_stack: Vec<Expr>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            pos: 0,
            op_stack: vec![],
            output_stack: vec![],
        }
    }

    fn precedence(op: &Op) -> u8 {
        match op {
            Op::Add | Op::Sub => 1,
            Op::Mul | Op::Div => 2,
            Op::Pow => 3,
            Op::UnaryMinus => 4,
            Op::LParen => 0,
        }
    }

    fn right_assoc(op: &Op) -> bool {
        matches!(op, Op::Pow | Op::UnaryMinus)
    }

    fn apply_op(&mut self, op: Op) -> Result<(), String> {
        match op {
            Op::UnaryMinus => {
                let a = self.output_stack.pop().ok_or("一元负号缺少操作数")?;
                self.output_stack.push(Expr::Neg(Box::new(a)));
            }
            _ => {
                let r = self.output_stack.pop().ok_or("缺少右操作数")?;
                let l = self.output_stack.pop().ok_or("缺少左操作数")?;
                let expr = match op {
                    Op::Add => Expr::Add(Box::new(l), Box::new(r)),
                    Op::Sub => Expr::Sub(Box::new(l), Box::new(r)),
                    Op::Mul => Expr::Mul(Box::new(l), Box::new(r)),
                    Op::Div => {
                        // a/b → a * b^(-1)
                        Expr::Mul(
                            Box::new(l),
                            Box::new(Expr::Pow(Box::new(r), Box::new(Expr::Number(-1.0)))),
                        )
                    }
                    Op::Pow => Expr::Pow(Box::new(l), Box::new(r)),
                    _ => unreachable!(),
                };
                self.output_stack.push(expr);
            }
        }
        Ok(())
    }

    fn parse(mut self) -> Result<Expr, String> {
        let mut expect_unary = true;

        while self.pos < self.tokens.len() {
            let token = self.tokens[self.pos].clone();
            self.pos += 1;

            match token {
                Token::Number(n) => {
                    self.output_stack.push(Expr::Number(n));
                    expect_unary = false;
                }
                Token::Symbol(s) => {
                    self.output_stack.push(Expr::Symbol(s));
                    expect_unary = false;
                }
                Token::Minus if expect_unary => {
                    self.op_stack.push(Op::UnaryMinus);
                }
                Token::Plus if expect_unary => {
                    // 一元正号忽略
                }
                op_tok @ (Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Caret) => {
                    let op = match op_tok {
                        Token::Plus => Op::Add,
                        Token::Minus => Op::Sub,
                        Token::Star => Op::Mul,
                        Token::Slash => Op::Div,
                        Token::Caret => Op::Pow,
                        _ => unreachable!(),
                    };
                    let prec = Self::precedence(&op);
                    while let Some(top) = self.op_stack.last() {
                        if *top == Op::LParen {
                            break;
                        }
                        let top_prec = Self::precedence(top);
                        if top_prec > prec || (top_prec == prec && !Self::right_assoc(&op)) {
                            let popped = self.op_stack.pop().unwrap();
                            self.apply_op(popped)?;
                        } else {
                            break;
                        }
                    }
                    self.op_stack.push(op);
                    expect_unary = true;
                }
                Token::LParen => {
                    self.op_stack.push(Op::LParen);
                    expect_unary = true;
                }
                Token::RParen => {
                    while let Some(op) = self.op_stack.pop() {
                        if op == Op::LParen {
                            break;
                        }
                        self.apply_op(op)?;
                    }
                    expect_unary = false;
                }
            }
        }

        while let Some(op) = self.op_stack.pop() {
            if op == Op::LParen {
                return Err("括号不匹配".into());
            }
            self.apply_op(op)?;
        }

        if self.output_stack.len() != 1 {
            return Err(format!(
                "表达式无效: 输出栈有 {} 个元素",
                self.output_stack.len()
            ));
        }

        Ok(self.output_stack.pop().unwrap())
    }
}

// ============================================================
// 公开解析接口
// ============================================================

/// 解析完整表达式（不含等号）
pub fn parse_expression(input: &str) -> Result<Expr, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("空输入".into());
    }
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize()?;
    let tokens = insert_implicit_mul(tokens);
    let parser = Parser::new(tokens);
    parser.parse()
}

/// 解析等式 "a + b = 10"
/// 保留此名称兼容现有接口，返回 Option 以和旧代码兼容
pub fn parse_simple_eq(input: &str) -> Option<Expr> {
    let input = input.trim();
    let parts: Vec<&str> = input.splitn(2, '=').collect();
    if parts.len() != 2 {
        return None;
    }
    let left = parse_expression(parts[0].trim()).ok()?;
    let right = parse_expression(parts[1].trim()).ok()?;
    Some(Expr::Eq(Box::new(left), Box::new(right)))
}

/// 解析单侧表达式（无等号），兼容旧接口
pub fn parse_side(s: &str) -> Option<Expr> {
    parse_expression(s.trim()).ok()
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_simple(expr: &Expr) -> f64 {
        expr.eval(&HashMap::new()).unwrap()
    }

    #[test]
    fn test_parse_and_eval() {
        let expr = parse_simple_eq("a + b = 10").unwrap();
        let syms = expr.symbols();
        assert!(syms.contains(&"a".to_string()));
        assert!(syms.contains(&"b".to_string()));

        // 验证求值
        let mut env = HashMap::new();
        env.insert("a".to_string(), 3.0);
        env.insert("b".to_string(), 7.0);
        match &expr {
            Expr::Eq(l, r) => {
                assert!((l.eval(&env).unwrap() - r.eval(&env).unwrap()).abs() < 1e-9);
            }
            _ => panic!("期望 Eq"),
        }
    }

    #[test]
    fn test_parse_constant() {
        let expr = parse_side("5").unwrap();
        assert_eq!(expr, Expr::Number(5.0));
    }

    #[test]
    fn test_parse_symbol() {
        let expr = parse_side("x").unwrap();
        assert_eq!(expr, Expr::Symbol("x".to_string()));
    }

    #[test]
    fn test_division_to_pow() {
        // 10 / 2 → 10 * 2^(-1) = 5
        let expr = parse_side("10 / 2").unwrap();
        assert!((eval_simple(&expr) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_precedence() {
        // a + b * c → a + (b * c)
        let expr = parse_expression("a + b * c").unwrap();
        let mut env = HashMap::new();
        env.insert("a".to_string(), 1.0);
        env.insert("b".to_string(), 2.0);
        env.insert("c".to_string(), 3.0);
        // 1 + (2 * 3) = 7
        assert!((expr.eval(&env).unwrap() - 7.0).abs() < 1e-9);
    }

    #[test]
    fn test_power_right_assoc() {
        // 2 ^ 3 ^ 2 = 2^(3^2) = 2^9 = 512
        // 如果是左结合：(2^3)^2 = 8^2 = 64
        let expr = parse_expression("2 ^ 3 ^ 2").unwrap();
        assert!((eval_simple(&expr) - 512.0).abs() < 1e-9);
    }

    #[test]
    fn test_parentheses() {
        // (1 + 2) * 3 = 9
        let expr = parse_expression("(1 + 2) * 3").unwrap();
        assert!((eval_simple(&expr) - 9.0).abs() < 1e-9);
    }

    #[test]
    fn test_implicit_mul_number_symbol() {
        // 2a = 2 * a
        let expr = parse_expression("2a").unwrap();
        let mut env = HashMap::new();
        env.insert("a".to_string(), 3.0);
        assert!((expr.eval(&env).unwrap() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn test_implicit_mul_symbol_symbol() {
        // xy = x * y
        let expr = parse_expression("xy").unwrap();
        let mut env = HashMap::new();
        env.insert("x".to_string(), 2.0);
        env.insert("y".to_string(), 3.0);
        assert!((expr.eval(&env).unwrap() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn test_implicit_mul_paren() {
        // 3(x+1) = 3 * (x+1)
        let expr = parse_expression("3(x+1)").unwrap();
        let mut env = HashMap::new();
        env.insert("x".to_string(), 2.0);
        assert!((expr.eval(&env).unwrap() - 9.0).abs() < 1e-9);
    }

    #[test]
    fn test_unary_minus_neg() {
        // -5
        let expr = parse_expression("-5").unwrap();
        assert!((eval_simple(&expr) - -5.0).abs() < 1e-9);
    }

    #[test]
    fn test_unary_minus_expr() {
        // -(x + 1)
        let expr = parse_expression("-(x + 1)").unwrap();
        let mut env = HashMap::new();
        env.insert("x".to_string(), 3.0);
        assert!((expr.eval(&env).unwrap() - -4.0).abs() < 1e-9);
    }

    #[test]
    fn test_newton_gravity() {
        // F = G * m1 * m2 / r^2
        let expr = parse_simple_eq("F = G * m1 * m2 / r^2").expect("万有引力公式解析失败");
        let mut env = HashMap::new();
        env.insert("F".to_string(), 1.98e20);
        env.insert("G".to_string(), 6.674e-11);
        env.insert("m1".to_string(), 5.97e24);
        env.insert("m2".to_string(), 7.35e22);
        env.insert("r".to_string(), 3.84e8);
        match &expr {
            Expr::Eq(l, r) => {
                let lv = l.eval(&env).unwrap();
                let rv = r.eval(&env).unwrap();
                let diff = (lv - rv).abs();
                // 相对误差检查（已知测试数据本身就是近似值）
                let rel_err = diff / lv.abs().max(rv.abs()).max(1.0);
                assert!(
                    rel_err < 1e-2,
                    "万有引力公式验证失败: {} != {} (rel_err={})",
                    lv,
                    rv,
                    rel_err
                );
            }
            _ => panic!("期望 Eq"),
        }
    }

    #[test]
    fn test_gravity_side_expression() {
        // 仅解析右侧表达式：G * m1 * m2 / r^2
        let expr = parse_expression("G * m1 * m2 / r ^ 2").unwrap();
        let mut env = HashMap::new();
        env.insert("G".to_string(), 6.674e-11);
        env.insert("m1".to_string(), 5.97e24);
        env.insert("m2".to_string(), 7.35e22);
        env.insert("r".to_string(), 3.84e8);
        let val = expr.eval(&env).unwrap();
        // 预期：6.674e-11 * 5.97e24 * 7.35e22 / (3.84e8)^2 ≈ 1.98e20
        let expected = 1.98e20;
        let rel_err = (val - expected).abs() / expected.abs().max(1.0);
        assert!(rel_err < 1e-2, "万有引力计算失败: {} (期望 ~{})", val, expected);
    }

    #[test]
    fn test_neg_neg() {
        // --5 = 5
        let expr = parse_expression("--5").unwrap();
        assert!((eval_simple(&expr) - 5.0).abs() < 1e-9);
    }
}
