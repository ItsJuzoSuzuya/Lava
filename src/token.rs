use std::fmt::{Display};

use serde::Serialize;

use crate::{r#type::Type};

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", content = "value")]
pub enum Token {
  Print,
  Func,
  Let,

  Arrow,

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

            Token::Arrow        => write!(f, "{}", "Arrow"),

            Token::LParen       => write!(f, "{}", "LParen"),
            Token::RParen       => write!(f, "{}", "RParen"),
            Token::LBrace       => write!(f, "{}", "LBrace"),
            Token::RBrace       => write!(f, "{}", "RBrace"),
            Token::Equal        => write!(f, "{}", "Equal"),
            Token::Colon        => write!(f, "{}", "Colon"),
            Token::Comma        => write!(f, "{}", "Comma"),
            Token::Semicolon    => write!(f, "{}", "Semicolon"),

            Token::Int32(value)    => write!(f, "Int32({})", value),
            Token::Identifier(name) => write!(f, "{}", name)
        }
    }
}

impl Token {
    pub fn to_type(&self) -> Type {
        match self {
            Token::Int32(_) => Type::Int32,
            Token::Identifier(s) => match s.as_str() {
              "int32" => Type::Int32,
              _ => panic!("Unknown type: {}", s)
            },
            _ => panic!("No type for Token: {}", self)
        }
    }

    pub fn from_char(c: char) -> Option<Token> {
        match c {
            '(' => { return Some(Token::LParen); }
            ')' => { return Some(Token::RParen); }
            ';' => { return Some(Token::Semicolon); }
            ',' => { return Some(Token::Comma); }
            ':' => { return Some(Token::Colon); }
            '=' => { return Some(Token::Equal); }
            '{' => { return Some(Token::LBrace); }
            '}' => { return Some(Token::RBrace); }
            _ => None
        }
    }

    pub fn from_string(token: &str) -> Token {
        match token {
            "func"  => Token::Func,
            "print" => Token::Print,
            "let"   => Token::Let,
            "->"    => Token::Arrow,
            _       => Token::Identifier(token.to_string())
        }
    }
}


