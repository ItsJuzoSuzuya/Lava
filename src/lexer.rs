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
        let mut cache_pos = self.pos;
        if cache_pos >= self.source.len() {
             return None;
        }

        // skip whitespace
        while self.source.chars().nth(cache_pos).unwrap() == ' ' {
            if cache_pos >= self.source.len() {
                 return None;
            }
            cache_pos += 1;
        }

        let mut token = String::from("");
        let mut c = self.source.chars().nth(cache_pos).unwrap();

        if c == '-' && self.source.chars().nth(cache_pos + 1) == Some('>') {
            return Some(Token::Arrow);
        }

        if let Some(t) = Token::from_char(c) {
            return Some(t);
        }

        // Number
        if c.is_digit(10) {
            while c.is_digit(10){
                token += &c.to_string();
                cache_pos += 1;
                c = self.source.chars().nth(cache_pos).unwrap_or(' ')
            }

            return Some(Token::Int32(token.parse::<i32>().unwrap()));
        }

        while c.is_alphanumeric() {
            token += &c.to_string();
            cache_pos += 1;
            c = self.source.chars().nth(cache_pos).unwrap_or(' ');
        }

        let token = Token::from_string(&token);
        Some(token)
    }

    pub fn get_next_token(&mut self) -> Option<Token> {
        if self.pos >= self.source.len() {
            return None;
        }

        // skip whitespace
        while self.source.chars().nth(self.pos).unwrap() == ' ' {
            if self.pos >= self.source.len() {
                return None;
            }
            self.pos += 1;
        }

        let mut c = self.source.chars().nth(self.pos).unwrap();

        if c == '-' && self.source.chars().nth(self.pos + 1) == Some('>') {
            self.pos += 2;
            return Some(Token::Arrow);
        }

        // Number
        if let Some(t) = Token::from_char(c) {
            self.pos += 1;
            return Some(t);
        }

        let mut token = String::from("");
        if c.is_digit(10) {
            while c.is_digit(10){
                token += &c.to_string();
                self.pos += 1;
                c = self.source.chars().nth(self.pos).unwrap_or(' ')
            }
            return Some(Token::Int32(token.parse::<i32>().unwrap()));
        }

        while c.is_alphanumeric() {
            token += &c.to_string();
            self.pos += 1;
            c = self.source.chars().nth(self.pos).unwrap_or(' ');
        }

        let token = Token::from_string(&token);
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

