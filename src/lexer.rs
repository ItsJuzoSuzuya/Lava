use crate::{span::Span, token::Token};

pub struct Lexer {
    line: usize,
    col: usize,
    pos: usize,
    pub source: String
}

impl Lexer {
    pub fn new(string: String) -> Self {
        Self {
            line: 0,
            col: 0,
            pos: 0,
            source: string,
        }
    }

    pub fn peek(&mut self) -> Result<(Token, Span), (Token, Span)> {
        let saved_pos = self.pos;
        let saved_line = self.line;
        let saved_col = self.col;
        let result = self.get_next_token();
        self.pos = saved_pos;
        self.line = saved_line;
        self.col = saved_col;
        result
    }

    pub fn get_next_token(&mut self) -> Result<(Token, Span), (Token, Span)> {
        if self.pos >= self.source.len() {
            let span = Span { len: Token::EOF.len(), col: self.col, line: self.line };
            return Err((Token::EOF, span));
        }

        // skip whitespace
        loop {
            match self.source.chars().nth(self.pos) {
                Some(c) => if c.is_whitespace() {
                    if c == '\n' {
                        self.line += 1;
                        self.col = 0;
                    } else {
                        self.col += 1;
                    }
                    self.pos += 1;
                } else { break },
                None => {
                    let span = Span { len: Token::EOF.len(), col: self.col, line: self.line };
                    return Err((Token::EOF, span))
                }
            }
        }

        let mut c = match self.source.chars().nth(self.pos) {
            Some(c) => c,
            None => {
                let span = Span { line: self.line, col: self.col, len: Token::EOF.len() };
                return Ok((Token::EOF, span))
            }
        };

        let start_line = self.line;
        let start_col = self.col;

        if let Some(c2) = self.source.chars().nth(self.pos + 1) {
            if let Some(t) = Token::from_two_chars(c, c2) {
                self.pos += 2;
                self.col += 2;
                let span = Span { line: start_line, col: start_col, len: t.len() };
                return Ok((t, span));
            }
        }

        if let Some(t) = Token::from_char(c) {
            self.pos += 1;
            self.col += 1;
            let span = Span { line: start_line, col: start_col, len: t.len() };
            return Ok((t, span));
        }

        let mut token = String::from("");
        if c == '"' {
            self.pos += 1;
            self.col += 1;
            c = self.source.chars().nth(self.pos).unwrap_or(' ');
            while c != '"' {
                token += &c.to_string();
                self.pos += 1;
                if c == '\n' {
                    self.line += 1;
                    self.col = 0;
                } else {
                    self.col += 1;
                }
                c = self.source.chars().nth(self.pos).unwrap_or(' ');
            }
            self.pos += 1;
            self.col += 1;

            let t = Token::StringLiteral(token);
            let span = Span { line: start_line, col: start_col, len: t.len() };
            return Ok((t, span));
        }

        if c.is_digit(10) {
            while c.is_digit(10){
                token += &c.to_string();
                self.pos += 1;
                self.col += 1;
                c = self.source.chars().nth(self.pos).unwrap_or(' ')
            }

            let t = Token::Numeral(token.parse::<i32>().unwrap());
            let span = Span { line: start_line, col: start_col, len: t.len() };
            return Ok((t, span));
        }

        while c.is_alphanumeric() || c == '_' {
            token += &c.to_string();
            self.pos += 1;
            self.col += 1;
            c = self.source.chars().nth(self.pos).unwrap_or(' ');
        }

        let token = Token::from_string(&token);
        let span = Span { line: start_line, col: start_col, len: token.len() };
        return Ok((token, span));
    }

    pub fn consume(&mut self) {
        let _ = self.get_next_token();
    }

    pub fn expect(&mut self, expected_token: Token) -> Result<(Token, Span), (Token, Span)>  {
        let (next_token, span) = match self.get_next_token() {
            Ok(t) => t,
            Err(t) => return Err(t)
        };
        if std::mem::discriminant(&next_token) != std::mem::discriminant(&expected_token) {
            return Err((next_token, span));
        }

        return Ok((next_token, span));
    }
}

