#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Equal,
    LParen,
    RParen,
    Comma,
    EOF,
}

impl Token {
    pub fn is_binary_op(&self) -> bool {
        matches!(self, Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Caret | Token::Equal)
    }
}
