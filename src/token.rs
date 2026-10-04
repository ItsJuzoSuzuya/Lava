use std::fmt::Display;

use serde::Serialize;

use crate::r#type::Type;

/// A single token produced by the [`Lexer`](crate::lexer::Lexer).
///
/// The lexer always pairs a token with the [`Span`](crate::span::Span) it came
/// from. Words that aren't keywords become [`Token::Identifier`].
///
/// [`Display`] prints a token as it is spelled in source, so it can be used
/// directly in error messages.
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum Token {
  BoolTy,
  Float32Ty,
  Float64Ty,
  Int8Ty,
  Int16Ty,
  Int32Ty,
  Int64Ty,
  StringTy,
  UInt8Ty,
  UInt16Ty,
  UInt32Ty,
  UInt64Ty,

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
  Import,

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

  FloatLiteral(f64),
  Identifier(String),
  IntLiteral(i64),
  StringLiteral(String),

  EOF
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::BoolTy    => write!(f, "bool"),
            Token::Float32Ty => write!(f, "float32"),
            Token::Float64Ty => write!(f, "float64"),
            Token::Int8Ty    => write!(f, "int8"),
            Token::Int16Ty   => write!(f, "int16"),
            Token::Int32Ty   => write!(f, "int32"),
            Token::Int64Ty   => write!(f, "int64"),
            Token::StringTy  => write!(f, "String"),
            Token::UInt8Ty   => write!(f, "uint8"),
            Token::UInt16Ty  => write!(f, "uint16"),
            Token::UInt32Ty  => write!(f, "uint32"),
            Token::UInt64Ty  => write!(f, "uint64"),

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
            Token::Import       => write!(f, "import"),

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

            Token::FloatLiteral(value)      => write!(f, "{:?}", value),
            Token::Identifier(name)         => write!(f, "{}", name),
            Token::IntLiteral(value)        => write!(f, "{}", value),
            Token::StringLiteral(literal)   => write!(f, "\"{}\"", literal),
            Token::EOF                      => write!(f, "end of file"),
        }
    }
}

impl Token {
    /// Converts a type token into its [`Type`].
    ///
    /// # Panics
    ///
    /// Panics if the token can't be mapped to a [`Type`].
    pub fn to_type(&self) -> Type {
        match self {
            Token::BoolTy    => Type::Bool,
            Token::Float32Ty => Type::Float { bits: 32 },
            Token::Float64Ty => Type::Float { bits: 64 },
            Token::Int8Ty    => Type::Int { bits: 8,  signed: true },
            Token::Int16Ty   => Type::Int { bits: 16, signed: true },
            Token::Int32Ty   => Type::Int { bits: 32, signed: true },
            Token::Int64Ty   => Type::Int { bits: 64, signed: true },
            Token::StringTy  => Type::String,
            Token::UInt8Ty   => Type::Int { bits: 8,  signed: false },
            Token::UInt16Ty  => Type::Int { bits: 16, signed: false },
            Token::UInt32Ty  => Type::Int { bits: 32, signed: false },
            Token::UInt64Ty  => Type::Int { bits: 64, signed: false },

            Token::Identifier(s) => Type::Class(s.to_string()),
            _ => panic!("No type for Token: {}", self)
        }
    }

    /// Tries to build a [`Token`] from two characters.
    ///
    /// Needs to be run before [`Token::from_char`] to ensure that the first
    /// character doesn't belong to a pair.
    pub fn from_two_chars(c1: char, c2: char) -> Option<Token> {
        match (c1, c2) {
            ('-', '>') => Some(Token::Arrow),
            (':', ':') => Some(Token::ColonColon),
            ('=', '>') => Some(Token::ThickArrow),
            _ => None,
        }
    }

    /// Tries to build a [`Token`] from one character.
    ///
    /// Some characters also start a pair token, so check [`Token::from_two_chars`] first.
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

    /// Turns a whole word into a [`Token`].
    ///
    /// Keywords map to their own [`Token`], everything else becomes a
    /// [`Token::Identifier`].
    pub fn from_string(token: &str) -> Token {
        match token {
            "bool"          => Token::BoolTy,
            "float32"       => Token::Float32Ty,
            "float64"       => Token::Float64Ty,
            "int8"          => Token::Int8Ty,
            "int16"         => Token::Int16Ty,
            "int32"         => Token::Int32Ty,
            "int64"         => Token::Int64Ty,
            "String"        => Token::StringTy,
            "uint8"         => Token::UInt8Ty,
            "uint16"        => Token::UInt16Ty,
            "uint32"        => Token::UInt32Ty,
            "uint64"        => Token::UInt64Ty,

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
            "import"        => Token::Import,
            _               => Token::Identifier(token.to_string())
        }
    }

    // Returns the token length.
    pub fn len(&self) -> usize {
        self.to_string().len()
    }
}
