use std::fmt::{Display};

use serde::Serialize;

use crate::{r#type::Type};

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", content = "value")]
pub enum Token {
  Print,
  Func,
  Let,

  LParen,
  RParen,
  LBrace,
  RBrace,
  Equal, 
  Colon,
  Comma,
  Semicolon,

  Int32(i32),
  Identifier(String),
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Print        => write!(f, "{}", "Print"),
            Token::Func         => write!(f, "{}", "Func"),
            Token::Let          => write!(f, "{}", "Let"),

            Token::LParen       => write!(f, "{}", "LParen"),
            Token::RParen       => write!(f, "{}", "RParen"),
            Token::LBrace       => write!(f, "{}", "LBrace"),
            Token::RBrace       => write!(f, "{}", "RBrace"),
            Token::Equal        => write!(f, "{}", "Equal"),
            Token::Colon        => write!(f, "{}", "Colon"),
            Token::Comma        => write!(f, "{}", "Comma"),
            Token::Semicolon    => write!(f, "{}", "Semicolon"),

            Token::Int32(value)    => write!(f, "Int32({})", value),
            Token::Identifier(name) => write!(f, "Identifier({})", name)
        }
    }
}

impl Token {
    pub fn to_type(&self) -> Type {
        match self {
            Token::Int32(_) => Type::Int32,
            _ => panic!("No type for Token: {}", self)
        }
    }
}


