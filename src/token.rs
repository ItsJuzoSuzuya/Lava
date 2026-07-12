use std::fmt::{Display};

use serde::Serialize;

use crate::{r#type::Type};

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", content = "value")]
pub enum Token {
  Print,
  Func,
  Let,
  Return,
  Class,
  Constructor,

  Arrow,

  LParen,
  RParen,
  LBrace,
  RBrace,
  Equal, 
  Colon,
  Comma,
  Semicolon,
  Plus,
  Minus,
  Multiply,
  Divide,
  Dot,

  Numeral(i32),
  Identifier(String),
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Print        => write!(f, "{}", "Print"),
            Token::Func         => write!(f, "{}", "Func"),
            Token::Let          => write!(f, "{}", "Let"),
            Token::Return       => write!(f, "{}", "Return"),
            Token::Class        => write!(f, "{}", "Class"),
            Token::Constructor  => write!(f, "{}", "Constructor"),

            Token::Arrow        => write!(f, "{}", "Arrow"),

            Token::LParen       => write!(f, "{}", "LParen"),
            Token::RParen       => write!(f, "{}", "RParen"),
            Token::LBrace       => write!(f, "{}", "LBrace"),
            Token::RBrace       => write!(f, "{}", "RBrace"),
            Token::Equal        => write!(f, "{}", "Equal"),
            Token::Colon        => write!(f, "{}", "Colon"),
            Token::Comma        => write!(f, "{}", "Comma"),
            Token::Semicolon    => write!(f, "{}", "Semicolon"),
            Token::Plus         => write!(f, "{}", "Plus"),
            Token::Minus        => write!(f, "{}", "Minus"),
            Token::Multiply     => write!(f, "{}", "Multiply"),
            Token::Divide       => write!(f, "{}", "Divide"),
            Token::Dot          => write!(f, "{}", "Dot"),

            Token::Numeral(value)   => write!(f, "Int32({})", value),
            Token::Identifier(name) => write!(f, "{}", name)
        }
    }
}

impl Token {
    pub fn to_type(&self) -> Type {
        match self {
            Token::Numeral(_) => Type::Int32,
            Token::Identifier(s) => match s.as_str() {
              "int32" => Type::Int32,
              "String" => Type::String,
              x => Type::Object(x.to_string())
            },
            _ => panic!("No type for Token: {}", self)
        }
    }

    pub fn from_char(c: char) -> Option<Token> {
        match c {
            '(' => { Some(Token::LParen) }
            ')' => { Some(Token::RParen) }
            ';' => { Some(Token::Semicolon) }
            ',' => { Some(Token::Comma) }
            ':' => { Some(Token::Colon) }
            '=' => { Some(Token::Equal) }
            '{' => { Some(Token::LBrace) }
            '}' => { Some(Token::RBrace) }
            '+' => { Some(Token::Plus) }
            '-' => { Some(Token::Minus) }
            '*' => { Some(Token::Multiply) }
            '/' => { Some(Token::Divide) }
            '.' => { Some(Token::Dot) }
            _ => None
        }
    }

    pub fn from_string(token: &str) -> Token {
        match token {
            "class"         => Token::Class,
            "Constructor"   => Token::Constructor,
            "func"          => Token::Func,
            "print"         => Token::Print,
            "let"           => Token::Let,
            "return"        => Token::Return,
            "->"            => Token::Arrow,
            _               => Token::Identifier(token.to_string())
        }
    }
}


