use crate::token::{Token};

pub struct Lexer {
    pos: usize,
    source: String
}

impl Lexer {
    pub fn new(string: String) -> Self {
        Self {
            pos: 0,
            source: string,
        }
    }

    pub fn peek(&mut self) -> Option<Token> {
        if self.pos >= self.source.len() {
             return None;
        }

        let cache_pos = self.pos;

        // skip whitespace
        while self.source.chars().nth(self.pos).unwrap() == ' ' {
            self.pos += 1;
        }

        let mut token = String::from("");
        let mut c = self.source.chars().nth(self.pos).unwrap();

        // Number
        if c.is_digit(10) {
            while c.is_digit(10){
                token += &c.to_string();
                self.pos += 1;
                c = self.source.chars().nth(self.pos).unwrap_or(' ')
            }

            self.pos = cache_pos;
            return Some(Token::Int32(token.parse::<i32>().unwrap()));
        }

        // Single-character punctuation
        match c {
            '(' => { self.pos = cache_pos; return Some(Token::LParen); }
            ')' => { self.pos = cache_pos; return Some(Token::RParen); }
            ';' => { self.pos = cache_pos; return Some(Token::Semicolon); }
            ':' => { self.pos += 1; return Some(Token::Colon); }
            '=' => { self.pos += 1; return Some(Token::Equal); }
            _ => {}
        }

        while c.is_ascii() {
            token += &c.to_string();
            self.pos += 1;
            c = self.source.chars().nth(self.pos).unwrap_or(' ');
        }

        if token.is_empty() {
            panic!("[Lexer] Error: Unexpected character '{}'", c);
        }

        let token = match token.as_str() {
            "print" => Token::Print,
            "let"   => Token::Let,
            _       => Token::Identifier(token)
        };
        self.pos = cache_pos;
        Some(token)
    }

    pub fn get_next_token(&mut self) -> Option<Token> {
        // EOF check
        if self.pos >= self.source.len() {
             return None;
        }

        // skip whitespace
        while self.source.chars().nth(self.pos).unwrap() == ' ' {
            self.pos += 1;
        }

        let mut token = String::from("");
        let mut c = self.source.chars().nth(self.pos).unwrap();

        // Number
        if c.is_digit(10) {
            while c.is_digit(10){
                token += &c.to_string();
                self.pos += 1;
                c = self.source.chars().nth(self.pos).unwrap_or(' ')
            }
            return Some(Token::Int32(token.parse::<i32>().unwrap()));
        }

        // Single-character punctuation
        match c {
            '(' => { self.pos += 1; return Some(Token::LParen); }
            ')' => { self.pos += 1; return Some(Token::RParen); }
            ';' => { self.pos += 1; return Some(Token::Semicolon); }
            ':' => { self.pos += 1; return Some(Token::Colon); }
            '=' => { self.pos += 1; return Some(Token::Equal); }
            _ => {}
        }

        while c.is_ascii() {
            token += &c.to_string();
            self.pos += 1;
            c = self.source.chars().nth(self.pos).unwrap_or(' ');
        }

        if token.is_empty() {
            panic!("[Lexer] Error: Unexpected character '{}'", c);
        }

        let token = match token.as_str() {
            "print" => Token::Print,
            "let"   => Token::Let,
            _       => Token::Identifier(token)
        };
        Some(token)
    }

    pub fn expect(&mut self, expected_token: Token) -> Token {
        let next_token: Token = self.get_next_token().expect(&format!("[Lexer] Error: Expected {}, found nothing", expected_token).to_string());
        if std::mem::discriminant(&next_token) != std::mem::discriminant(&expected_token) {
            panic!("[Lexer] Error: Expected {}, found {}", expected_token, next_token);
        }

        return next_token;
    }
}

