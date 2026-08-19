use std::fmt::Display;

use serde::Serialize;

use crate::r#type::Type;

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum Token {
  BoolTy,
  Int32Ty,
  StringTy,

  Class,
  Constructor,
  Enum,
  False,
  Func,
  Let,
  Match,
  Mut,
  Print,
  Pub, 
  Return,
  SelfRef,
  True,

  Arrow,
  ColonColon,
  ThickArrow,

  Assign, 
  BAnd,
  Colon,
  Comma,
  Divide,
  Dot,
  LBrace,
  LParen,
  Minus,
  Multiply,
  Plus,
  RBrace,
  RParen,
  Semicolon,

  Identifier(String),
  Numeral(i32),
  StringLiteral(String),

  EOF
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::BoolTy   => write!(f, "bool"),
            Token::Int32Ty  => write!(f, "int32"),
            Token::StringTy => write!(f, "String"),

            Token::Print        => write!(f, "print"),
            Token::Func         => write!(f, "func"),
            Token::False        => write!(f, "false"),
            Token::Let          => write!(f, "let"),
            Token::Return       => write!(f, "return"),
            Token::Class        => write!(f, "class"),
            Token::Constructor  => write!(f, "Constructor"),
            Token::SelfRef      => write!(f, "self"),
            Token::Match        => write!(f, "match"),
            Token::Mut          => write!(f, "mut"),
            Token::Pub          => write!(f, "pub"),
            Token::Enum         => write!(f, "enum"),
            Token::True         => write!(f, "true"),

            Token::Arrow        => write!(f, "->"),
            Token::ColonColon   => write!(f, "::"),
            Token::ThickArrow   => write!(f, "=>"),

            Token::LParen       => write!(f, "("),
            Token::RParen       => write!(f, ")"),
            Token::LBrace       => write!(f, "{{"),
            Token::RBrace       => write!(f, "}}"),
            Token::Assign       => write!(f, "="),
            Token::Colon        => write!(f, ":"),
            Token::Comma        => write!(f, ","),
            Token::Semicolon    => write!(f, ";"),
            Token::Plus         => write!(f, "+"),
            Token::Minus        => write!(f, "-"),
            Token::Multiply     => write!(f, "*"),
            Token::Divide       => write!(f, "/"),
            Token::Dot          => write!(f, "."),
            Token::BAnd         => write!(f, "&"),

            Token::Numeral(value)           => write!(f, "{}", value),
            Token::StringLiteral(literal)   => write!(f, "\"{}\"", literal),
            Token::Identifier(name)         => write!(f, "{}", name),
            Token::EOF                      => write!(f, "end of file"),
        }
    }
}

impl Token {
    pub fn to_type(&self) -> Type {
        match self {
            Token::BoolTy => Type::Bool(false),
            Token::Int32Ty => Type::Int32,
            Token::StringTy => Type::String,

            Token::Numeral(_) => Type::Int32,
            Token::Identifier(s) => Type::Class(s.to_string()),
            _ => panic!("No type for Token: {}", self)
        }
    }

    pub fn from_two_chars(c1: char, c2: char) -> Option<Token> {
        match (c1, c2) {
            ('-', '>') => Some(Token::Arrow),
            (':', ':') => Some(Token::ColonColon),
            ('=', '>') => Some(Token::ThickArrow),
            _ => None,
        }
    }

    pub fn from_char(c: char) -> Option<Token> {
        match c {
            '(' => { Some(Token::LParen) }
            ')' => { Some(Token::RParen) }
            ';' => { Some(Token::Semicolon) }
            ',' => { Some(Token::Comma) }
            ':' => { Some(Token::Colon) }
            '=' => { Some(Token::Assign) }
            '{' => { Some(Token::LBrace) }
            '}' => { Some(Token::RBrace) }
            '+' => { Some(Token::Plus) }
            '-' => { Some(Token::Minus) }
            '*' => { Some(Token::Multiply) }
            '/' => { Some(Token::Divide) }
            '.' => { Some(Token::Dot) }
            '&' => { Some(Token::BAnd) }
            _ => None
        }
    }

    pub fn from_string(token: &str) -> Token {
        match token {
            "bool"          => Token::BoolTy,
            "int32"         => Token::Int32Ty,
            "String"        => Token::StringTy,

            "class"         => Token::Class,
            "Constructor"   => Token::Constructor,
            "func"          => Token::Func,
            "false"         => Token::False,
            "print"         => Token::Print,
            "let"           => Token::Let,
            "return"        => Token::Return,
            "self"          => Token::SelfRef,
            "true"          => Token::True,
            "mut"           => Token::Mut,
            "match"         => Token::Match,
            "pub"           => Token::Pub,
            "enum"          => Token::Enum,
            _               => Token::Identifier(token.to_string())
        }
    }

    pub fn len(&self) -> usize {
        self.to_string().len()
    }
}
