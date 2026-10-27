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
  CharTy,
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

  Break,
  Class,
  Constructor,
  Continue,
  Else,
  Enum,
  Extern,
  False,
  For,
  Func,
  If,
  Impl,
  Let,
  Match,
  Mut,
  Print,
  Pub, 
  Return,
  SelfRef,
  Trait,
  True,
  While,
  Import,
  As,

  AndAnd,
  Arrow,
  CaretEq,
  ColonColon,
  DivideEq,
  DotDot,
  EqEq,
  GtEq,
  LtEq,
  MinusEq,
  MultiplyEq,
  NotEq,
  OrOr,
  PercentEq,
  PlusEq,
  QuestionQuestion,
  ThickArrow,

  Assign,
  BAnd,
  Bang,
  Caret,
  Colon,
  Comma,
  Divide,
  Dot,
  Gt,
  LBrace,
  LBracket,
  LParen,
  Lt,
  Minus,
  Multiply,
  Percent,
  Plus,
  Question,
  RBrace,
  RBracket,
  RParen,
  Semicolon,

  /// A byte like `'a'` or `'\n'`, an integer literal of its value.
  CharLiteral(u8),
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
            Token::CharTy    => write!(f, "char"),
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

            Token::Break        => write!(f, "break"),
            Token::Continue     => write!(f, "continue"),
            Token::Print        => write!(f, "print"),
            Token::Func         => write!(f, "func"),
            Token::If           => write!(f, "if"),
            Token::Else         => write!(f, "else"),
            Token::While        => write!(f, "while"),
            Token::For          => write!(f, "for"),
            Token::Impl         => write!(f, "impl"),
            Token::False        => write!(f, "false"),
            Token::Let          => write!(f, "let"),
            Token::Return       => write!(f, "return"),
            Token::Class        => write!(f, "class"),
            Token::Constructor  => write!(f, "Constructor"),
            Token::SelfRef      => write!(f, "self"),
            Token::Trait        => write!(f, "trait"),
            Token::Match        => write!(f, "match"),
            Token::Mut          => write!(f, "mut"),
            Token::Pub          => write!(f, "pub"),
            Token::Enum         => write!(f, "enum"),
            Token::Extern       => write!(f, "extern"),
            Token::True         => write!(f, "true"),
            Token::Import       => write!(f, "import"),
            Token::As           => write!(f, "as"),

            Token::AndAnd       => write!(f, "&&"),
            Token::OrOr         => write!(f, "||"),
            Token::QuestionQuestion => write!(f, "??"),
            Token::Arrow        => write!(f, "->"),
            Token::ColonColon   => write!(f, "::"),
            Token::DotDot       => write!(f, ".."),
            Token::ThickArrow   => write!(f, "=>"),
            Token::EqEq         => write!(f, "=="),
            Token::NotEq        => write!(f, "!="),
            Token::LtEq         => write!(f, "<="),
            Token::GtEq         => write!(f, ">="),
            Token::PlusEq       => write!(f, "+="),
            Token::MinusEq      => write!(f, "-="),
            Token::MultiplyEq   => write!(f, "*="),
            Token::DivideEq     => write!(f, "/="),
            Token::PercentEq    => write!(f, "%="),
            Token::CaretEq      => write!(f, "^="),

            Token::LParen       => write!(f, "("),
            Token::RParen       => write!(f, ")"),
            Token::LBrace       => write!(f, "{{"),
            Token::RBrace       => write!(f, "}}"),
            Token::LBracket     => write!(f, "["),
            Token::RBracket     => write!(f, "]"),
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
            Token::Lt           => write!(f, "<"),
            Token::Gt           => write!(f, ">"),
            Token::Bang         => write!(f, "!"),
            Token::Caret        => write!(f, "^"),
            Token::Question     => write!(f, "?"),
            Token::Percent      => write!(f, "%"),
            Token::CharLiteral(value)       => write!(f, "'{}'", (*value as char).escape_default()),
            Token::FloatLiteral(value)      => write!(f, "{:?}", value),
            Token::Identifier(name)         => write!(f, "{}", name),
            Token::IntLiteral(value)        => write!(f, "{}", value),
            Token::StringLiteral(literal)   => write!(f, "\"{}\"", literal),
            Token::EOF                      => write!(f, "end of file"),
        }
    }
}

impl Token {
    /// Whether this is a comparison operator like `==` or `<`, which gives a `bool`.
    pub fn is_comparison(&self) -> bool {
        matches!(self, Token::EqEq | Token::NotEq | Token::Lt | Token::LtEq | Token::Gt | Token::GtEq)
    }

    /// The operator of a compound assignment like `+=`, `+` for `a += b`,
    /// which is `a = a + b`. `None` for every other token.
    pub fn compound_operator(&self) -> Option<Token> {
        match self {
            Token::PlusEq => Some(Token::Plus),
            Token::MinusEq => Some(Token::Minus),
            Token::MultiplyEq => Some(Token::Multiply),
            Token::DivideEq => Some(Token::Divide),
            Token::PercentEq => Some(Token::Percent),
            Token::CaretEq => Some(Token::Caret),
            _ => None,
        }
    }

    /// Whether this is `&&` or `||`, which combine `bool`s.
    pub fn is_logical(&self) -> bool {
        matches!(self, Token::AndAnd | Token::OrOr)
    }

    /// Whether this is a built-in type keyword like `int32` or `String`.
    pub fn is_type_keyword(&self) -> bool {
        matches!(self,
            Token::BoolTy | Token::CharTy | Token::Float32Ty | Token::Float64Ty
            | Token::Int8Ty | Token::Int16Ty | Token::Int32Ty | Token::Int64Ty | Token::StringTy
            | Token::UInt8Ty | Token::UInt16Ty | Token::UInt32Ty | Token::UInt64Ty)
    }

    /// Converts a type token into its [`Type`].
    ///
    /// # Panics
    ///
    /// Panics if the token can't be mapped to a [`Type`].
    pub fn to_type(&self) -> Type {
        match self {
            Token::BoolTy    => Type::Bool,
            Token::CharTy    => Type::Char,
            Token::Float32Ty => Type::Float { bits: 32 },
            Token::Float64Ty => Type::Float { bits: 64 },
            Token::Int8Ty    => Type::Int { bits: 8,  signed: true },
            Token::Int16Ty   => Type::Int { bits: 16, signed: true },
            Token::Int32Ty   => Type::Int { bits: 32, signed: true },
            Token::Int64Ty   => Type::Int { bits: 64, signed: true },
            Token::StringTy  => Type::string(),
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
            ('&', '&') => Some(Token::AndAnd),
            ('|', '|') => Some(Token::OrOr),
            ('?', '?') => Some(Token::QuestionQuestion),
            ('-', '>') => Some(Token::Arrow),
            (':', ':') => Some(Token::ColonColon),
            ('.', '.') => Some(Token::DotDot),
            ('=', '=') => Some(Token::EqEq),
            ('>', '=') => Some(Token::GtEq),
            ('<', '=') => Some(Token::LtEq),
            ('!', '=') => Some(Token::NotEq),
            ('=', '>') => Some(Token::ThickArrow),
            ('+', '=') => Some(Token::PlusEq),
            ('-', '=') => Some(Token::MinusEq),
            ('*', '=') => Some(Token::MultiplyEq),
            ('/', '=') => Some(Token::DivideEq),
            ('%', '=') => Some(Token::PercentEq),
            ('^', '=') => Some(Token::CaretEq),
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
            '[' => { Some(Token::LBracket) }
            ']' => { Some(Token::RBracket) }
            '+' => { Some(Token::Plus) }
            '-' => { Some(Token::Minus) }
            '*' => { Some(Token::Multiply) }
            '/' => { Some(Token::Divide) }
            '.' => { Some(Token::Dot) }
            '&' => { Some(Token::BAnd) }
            '<' => { Some(Token::Lt) }
            '>' => { Some(Token::Gt) }
            '!' => { Some(Token::Bang) }
            '^' => { Some(Token::Caret) }
            '?' => { Some(Token::Question) }
            '%' => { Some(Token::Percent) }            _ => None
        }
    }

    /// Turns a whole word into a [`Token`].
    ///
    /// Keywords map to their own [`Token`], everything else becomes a
    /// [`Token::Identifier`].
    pub fn from_string(token: &str) -> Token {
        match token {
            "bool"          => Token::BoolTy,
            "char"          => Token::CharTy,
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

            "break"         => Token::Break,
            "class"         => Token::Class,
            "Constructor"   => Token::Constructor,
            "continue"      => Token::Continue,
            "func"          => Token::Func,
            "if"            => Token::If,
            "else"          => Token::Else,
            "while"         => Token::While,
            "for"           => Token::For,
            "impl"          => Token::Impl,
            "false"         => Token::False,
            "print"         => Token::Print,
            "let"           => Token::Let,
            "return"        => Token::Return,
            "self"          => Token::SelfRef,
            "trait"         => Token::Trait,
            "true"          => Token::True,
            "mut"           => Token::Mut,
            "match"         => Token::Match,
            "pub"           => Token::Pub,
            "enum"          => Token::Enum,
            "extern"        => Token::Extern,
            "import"        => Token::Import,
            "as"            => Token::As,
            _               => Token::Identifier(token.to_string())
        }
    }

    // Returns the token length.
    pub fn len(&self) -> usize {
        self.to_string().len()
    }
}
