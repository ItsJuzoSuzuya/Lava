use crate::{span::Span, token::{Token, TokenWithSpan}};

pub struct Lexer {
    pos: usize,
    line: usize,
    col: usize,
    source: String
}

impl Lexer {
    pub fn new(string: String) -> Self {
        Self {
            pos: 0,
            line: 0,
            col: 0,
            source: string,
        }
    }

    pub fn peek(&mut self) -> Option<TokenWithSpan> {
        let mut cache_pos = self.pos;
        if cache_pos >= self.source.len() {
             return None;
        }

        // skip whitespace
        while cache_pos < self.source.len() && self.source.chars().nth(cache_pos).unwrap().is_whitespace() {
            cache_pos += 1;
        }

        let mut token = String::from("");
        let mut c = self.source.chars().nth(cache_pos).unwrap();

        if c == '-' && self.source.chars().nth(cache_pos + 1) == Some('>') {
            let span = Span { line: self.line, col: self.col, len: Token::Arrow.len() };
            return Some(TokenWithSpan { token: Token::Arrow, span: span  });
        }

        if let Some(t) = Token::from_char(c) {
            let span = Span { line: self.line, col: self.col, len: t.len() };
            return Some(TokenWithSpan { token: t, span: span  });
        }

        // Number
        if c.is_digit(10) {
            while c.is_digit(10){
                token += &c.to_string();
                cache_pos += 1;
                c = self.source.chars().nth(cache_pos).unwrap_or(' ')
            }

            let t = Token::Numeral(token.parse::<i32>().unwrap());
            let span = Span { line: self.line, col: self.col, len: t.len() };
            return Some(TokenWithSpan{ token: t, span: span });
        }

        while c.is_alphanumeric() {
            token += &c.to_string();
            cache_pos += 1;
            c = self.source.chars().nth(cache_pos).unwrap_or(' ');
        }

        let token = Token::from_string(&token);
        let span = Span { line: self.line, col: self.col, len: token.len() };
        Some(TokenWithSpan { token, span })
    }

    pub fn get_next_token(&mut self) -> Option<TokenWithSpan> {
        if self.pos >= self.source.len() {
            return None;
        }

        // skip whitespace
        while self.pos < self.source.len() && self.source.chars().nth(self.pos).unwrap().is_whitespace() {
            self.pos += 1;
        }

        let mut c = self.source.chars().nth(self.pos).unwrap();

        if c == '-' && self.source.chars().nth(self.pos + 1) == Some('>') {
            self.pos += 2;
            let span = Span { line: self.line, col: self.col, len: Token::Arrow.len() };
            return Some(TokenWithSpan { token: Token::Arrow, span: span  });
        }

        // Number
        if let Some(t) = Token::from_char(c) {
            self.pos += 1;
            let span = Span { line: self.line, col: self.col, len: t.len() };
            return Some(TokenWithSpan { token: t, span: span  });
        }

        let mut token = String::from("");
        if c.is_digit(10) {
            while c.is_digit(10){
                token += &c.to_string();
                self.pos += 1;
                c = self.source.chars().nth(self.pos).unwrap_or(' ')
            }

            let t = Token::Numeral(token.parse::<i32>().unwrap());
            let span = Span { line: self.line, col: self.col, len: t.len() };
            return Some(TokenWithSpan{ token: t, span: span });
        }

        while c.is_alphanumeric() {
            token += &c.to_string();
            self.pos += 1;
            c = self.source.chars().nth(self.pos).unwrap_or(' ');
        }

        let token = Token::from_string(&token);
        let span = Span { line: self.line, col: self.col, len: token.len() };
        Some(TokenWithSpan { token, span })
    }

    pub fn expect(&mut self, expected_token: Token) -> TokenWithSpan {
        let next_token: TokenWithSpan = self.get_next_token().expect(&format!("[Lexer] Error: Expected {}, found nothing", expected_token).to_string());
        if std::mem::discriminant(&next_token.token) != std::mem::discriminant(&expected_token) {
            panic!("[Lexer] Error: Expected {}, found {}", expected_token, next_token.token);
        }

        return next_token;
    }
}

