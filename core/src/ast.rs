/// 表达式语法树与符号引擎
///
/// 纯标准库实现，手写极简解析器。
/// 支持 a + b, a - b, a * b, a / b, a ^ b, 数字常量、变量符号。
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
                Ok(base.powf(exp))
            }
            Expr::Eq(_, _) => Err("Eq 不能直接求值".into()),
        }
    }
}

// ============================================================
// 极简解析器
// ============================================================

/// 解析 "a + b = 10" 形式的等式
pub fn parse_simple_eq(input: &str) -> Option<Expr> {
    let parts: Vec<&str> = input.split('=').collect();
    if parts.len() != 2 {
        return None;
    }
    let left = parse_side(parts[0].trim())?;
    let right = parse_side(parts[1].trim())?;
    Some(Expr::Eq(Box::new(left), Box::new(right)))
}

/// 解析 "5"、"a"、"a + b"、"a * b" 等形式
pub fn parse_side(s: &str) -> Option<Expr> {
    let s = s.replace(' ', "");

    // 数字常量
    if let Ok(n) = s.parse::<f64>() {
        return Some(Expr::Number(n));
    }

    // 按优先级从低到高分割：先 + -，后 * /，最后 ^
    // 注意：这里简单实现从左到右处理，不考虑运算符优先级
    // 实际项目应用 nom/pest

    // 从右往左分割 + -（右结合简化处理）
    if let Some(pos) = s.rfind('+') {
        let left = parse_side(&s[..pos])?;
        let right = parse_side(&s[pos + 1..])?;
        return Some(Expr::Add(Box::new(left), Box::new(right)));
    }
    if let Some(pos) = s.rfind('-') {
        if pos > 0 {
            let left = parse_side(&s[..pos])?;
            let right = parse_side(&s[pos + 1..])?;
            return Some(Expr::Sub(Box::new(left), Box::new(right)));
        }
    }

    // 从右往左分割 * /
    if let Some(pos) = s.rfind('*') {
        let left = parse_side(&s[..pos])?;
        let right = parse_side(&s[pos + 1..])?;
        return Some(Expr::Mul(Box::new(left), Box::new(right)));
    }
    if let Some(pos) = s.rfind('/') {
        let left = parse_side(&s[..pos])?;
        let right = parse_side(&s[pos + 1..])?;
        // a/b → a * b^(-1)
        return Some(Expr::Mul(
            Box::new(left),
            Box::new(Expr::Pow(Box::new(right), Box::new(Expr::Number(-1.0)))),
        ));
    }

    // 从右往左分割 ^
    if let Some(pos) = s.rfind('^') {
        let left = parse_side(&s[..pos])?;
        let right = parse_side(&s[pos + 1..])?;
        return Some(Expr::Pow(Box::new(left), Box::new(right)));
    }

    // 纯符号
    if s.chars().all(|c| c.is_alphabetic() || c == '_') {
        return Some(Expr::Symbol(s.to_string()));
    }

    None
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

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
        let expr = parse_side("10 / 2").unwrap();
        let mut env = HashMap::new();
        // 10 * 2^(-1) = 5
        assert!((expr.eval(&env).unwrap() - 5.0).abs() < 1e-9);
    }
}
