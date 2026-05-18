use symmath_ast::arena::AstArena;
use symmath_ast::expr::Expr;
use symmath_ast::ops::{BinaryOp, UnaryOp};
use symmath_common::ids::NodeId;
use symmath_common::symbol::SymbolTable;

use crate::lexer::Lexer;
use crate::token::Token;

fn prefix_binding_power(op: &Token) -> Option<u8> {
    match op {
        Token::Minus => Some(9),
        Token::Ident(_) => Some(9),
        Token::Number(_) => Some(9),
        Token::LParen => Some(9),
        _ => None,
    }
}

fn infix_binding_power(op: &Token) -> Option<(u8, u8)> {
    match op {
        Token::Equal => Some((1, 2)),
        Token::Plus | Token::Minus => Some((3, 4)),
        Token::Star | Token::Slash => Some((5, 6)),
        Token::Caret => Some((7, 8)),
        _ => None,
    }
}

pub struct Parser<'a> {
    lexer: Lexer,
    current: Token,
    arena: &'a mut AstArena,
    symbols: &'a mut SymbolTable,
}

impl<'a> Parser<'a> {
    pub fn new(input: &str, arena: &'a mut AstArena, symbols: &'a mut SymbolTable) -> Self {
        let mut lexer = Lexer::new(input);
        let current = lexer.next_token();
        Self {
            lexer,
            current,
            arena,
            symbols,
        }
    }

    fn advance(&mut self) {
        self.current = self.lexer.next_token();
    }

    fn expect(&mut self, expected: &Token) {
        if std::mem::discriminant(&self.current) != std::mem::discriminant(expected) {
            panic!("expected {:?}, got {:?}", expected, self.current);
        }
        self.advance();
    }

    pub fn parse_expr(&mut self, min_bp: u8) -> NodeId {
        let mut lhs = self.parse_prefix();

        loop {
            if self.current == Token::EOF {
                break;
            }
            if let Some((l_bp, r_bp)) = infix_binding_power(&self.current) {
                if l_bp < min_bp {
                    break;
                }
                lhs = self.parse_infix(lhs, r_bp);
            } else {
                break;
            }
        }

        lhs
    }

    fn parse_prefix(&mut self) -> NodeId {
        match &self.current {
            Token::Number(val) => {
                let v = *val;
                self.advance();
                self.arena.add(Expr::Const(v))
            }
            Token::Ident(name) => {
                let ident = name.clone();
                self.advance();
                if self.current == Token::LParen {
                    // Function call: f(...)
                    self.advance(); // consume (
                    let mut args = Vec::new();
                    if self.current != Token::RParen {
                        args.push(self.parse_expr(0));
                        while self.current == Token::Comma {
                            self.advance();
                            args.push(self.parse_expr(0));
                        }
                    }
                    self.expect(&Token::RParen);
                    let func = self.symbols.intern(&ident);
                    self.arena.add(Expr::Call { func, args })
                } else {
                    // Variable
                    let sym = self.symbols.intern(&ident);
                    self.arena.add(Expr::Var(sym))
                }
            }
            Token::Minus => {
                self.advance();
                let input = self.parse_expr(prefix_binding_power(&Token::Minus).unwrap_or(9));
                self.arena.add(Expr::Unary {
                    op: UnaryOp::Neg,
                    input,
                })
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expr(0);
                self.expect(&Token::RParen);
                expr
            }
            _ => panic!("unexpected token: {:?}", self.current),
        }
    }

    fn parse_infix(&mut self, lhs: NodeId, min_bp: u8) -> NodeId {
        match &self.current {
            Token::Plus => {
                self.advance();
                let rhs = self.parse_expr(min_bp);
                self.arena.add(Expr::Binary {
                    op: BinaryOp::Add,
                    lhs,
                    rhs,
                })
            }
            Token::Minus => {
                self.advance();
                let rhs = self.parse_expr(min_bp);
                self.arena.add(Expr::Binary {
                    op: BinaryOp::Sub,
                    lhs,
                    rhs,
                })
            }
            Token::Star => {
                self.advance();
                let rhs = self.parse_expr(min_bp);
                self.arena.add(Expr::Binary {
                    op: BinaryOp::Mul,
                    lhs,
                    rhs,
                })
            }
            Token::Slash => {
                self.advance();
                let rhs = self.parse_expr(min_bp);
                self.arena.add(Expr::Binary {
                    op: BinaryOp::Div,
                    lhs,
                    rhs,
                })
            }
            Token::Caret => {
                self.advance();
                let rhs = self.parse_expr(min_bp);
                self.arena.add(Expr::Binary {
                    op: BinaryOp::Pow,
                    lhs,
                    rhs,
                })
            }
            Token::Equal => {
                self.advance();
                let rhs = self.parse_expr(min_bp);
                self.arena.add(Expr::Binary {
                    op: BinaryOp::Eq,
                    lhs,
                    rhs,
                })
            }
            _ => panic!("unexpected infix token: {:?}", self.current),
        }
    }

    pub fn parse_full(&mut self) -> Vec<NodeId> {
        let mut exprs = Vec::new();
        while self.current != Token::EOF {
            exprs.push(self.parse_expr(0));
        }
        exprs
    }
}
