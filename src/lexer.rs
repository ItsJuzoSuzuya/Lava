use crate::{span::Span, token::Token};

/// What went wrong while turning source text into a token.
#[derive(Debug)]
pub enum LexErrorKind {
    /// A character that can't start any token, e.g. `@` or `$`.
    UnexpectedChar(char),
    /// A string literal whose closing `"` is missing.
    UnterminatedString,
    /// An integer literal that doesn't fit into an `i64`.
    NumberOutOfRange(String),
}

/// An error produced by the [`Lexer`], with the [`Span`] where it occurred.
#[derive(Debug)]
pub struct LexError {
    pub kind: LexErrorKind,
    pub span: Span,
}

/// A saved position of a [`Lexer`], to go back to with [`Lexer::rewind`].
#[derive(Clone, Copy)]
pub struct Checkpoint {
    line: usize,
    col: usize,
    pos: usize,
}

/// Turns source code into [`Token`]s, one at a time.
///
/// Tracks the current line and column, so every token comes with the [`Span`]
/// it was read from.
pub struct Lexer {
    line: usize,
    col: usize,
    pos: usize,
    pub source: String
}

impl Lexer {
    /// Creates a lexer positioned at the start of `string`.
    pub fn new(string: String) -> Self {
        Self {
            line: 0,
            col: 0,
            pos: 0,
            source: string,
        }
    }

    /// Lexes the next token without advancing the lexer.
    pub fn peek(&mut self) -> Result<(Token, Span), LexError> {
        let checkpoint = self.checkpoint();
        let result = self.get_next_token();
        self.rewind(checkpoint);

        return result
    }

    /// Saves the current position, so the parser can try parsing something
    /// and go back with [`Lexer::rewind`] if it doesn't fit.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint { line: self.line, col: self.col, pos: self.pos }
    }

    /// Goes back to a position saved with [`Lexer::checkpoint`].
    pub fn rewind(&mut self, checkpoint: Checkpoint) {
        self.line = checkpoint.line;
        self.col = checkpoint.col;
        self.pos = checkpoint.pos;
    }

    /// Lexes the next token and advances the lexer past it.
    ///
    /// Skips whitespace first. Returns [`Token::EOF`] at the end of the source,
    /// and keeps returning it on further calls.
    pub fn get_next_token(&mut self) -> Result<(Token, Span), LexError> {
        // Insert EOF at the end of the source code and return EOF.
        if self.pos >= self.source.len() {
            let span = Span { len: Token::EOF.len(), col: self.col, line: self.line };
            return Ok((Token::EOF, span));
        }

        // Skip whitespace or return EOF when no char 
        loop {
            match self.source.chars().nth(self.pos) {
                Some(c) => 
                    if c == '\n' {
                        self.line += 1;
                        self.col = 0;
                        self.pos += 1;
                    } else if c.is_whitespace() {
                        self.col += 1;
                        self.pos += 1;
                    } else {     
                        break 
                    },
                None => {
                    let span = Span { len: Token::EOF.len(), col: self.col, line: self.line };
                    return Ok((Token::EOF, span))
                }
            }
        }

        // Get first char of the word
        let mut c = match self.source.chars().nth(self.pos) {
            Some(c) => c,
            None => {
                let span = Span { line: self.line, col: self.col, len: Token::EOF.len() };
                return Ok((Token::EOF, span))
            }
        };

        // Cache beginning of the word 
        let start_line = self.line;
        let start_col = self.col;

        // First check if char belongs to a pair
        if let Some(c2) = self.source.chars().nth(self.pos + 1) {
            if let Some(t) = Token::from_two_chars(c, c2) {
                self.pos += 2;
                self.col += 2;
                let span = Span { line: start_line, col: start_col, len: t.len() };
                return Ok((t, span));
            }
        }

        // If not a pair maybe a char token?
        if let Some(t) = Token::from_char(c) {
            self.pos += 1;
            self.col += 1;
            let span = Span { line: start_line, col: start_col, len: t.len() };
            return Ok((t, span));
        }

        // Create [`Token::StringLiteral`] from "my_string" 
        let mut token = String::from("");
        if c == '"' {
            self.pos += 1;
            self.col += 1;
            loop {
                c = match self.source.chars().nth(self.pos) {
                    Some('"') => break,
                    Some(c) => c,
                    None => {
                        let span = Span { line: start_line, col: start_col, len: token.len() + 1 };
                        return Err(LexError { kind: LexErrorKind::UnterminatedString, span });
                    }
                };
                token += &c.to_string();
                self.pos += 1;
                // col gets reset when a newline char occurs
                self.col += 1;

                if c == '\n' {
                    self.line += 1;
                    self.col = 0;
                }
            }
            // skip the closing " char
            self.pos += 1;
            self.col += 1;

            let t = Token::StringLiteral(token);
            let span = Span { line: start_line, col: start_col, len: t.len() };
            return Ok((t, span));
        }

        // Create [`Token::IntLiteral`] or [`Token::FloatLiteral`] from digits
        if c.is_digit(10) {
            while c.is_digit(10){
                token += &c.to_string();
                self.pos += 1;
                self.col += 1;
                c = self.source.chars().nth(self.pos).unwrap_or(' ')
            }

            // A '.' followed by a digit continues the number as a float
            let next_is_digit = self.source.chars().nth(self.pos + 1).is_some_and(|n| n.is_digit(10));
            if c == '.' && next_is_digit {
                token += ".";
                self.pos += 1;
                self.col += 1;
                c = self.source.chars().nth(self.pos).unwrap_or(' ');
                while c.is_digit(10) {
                    token += &c.to_string();
                    self.pos += 1;
                    self.col += 1;
                    c = self.source.chars().nth(self.pos).unwrap_or(' ')
                }

                // Digits and a single '.' always parse as f64
                let t = Token::FloatLiteral(token.parse::<f64>().unwrap());
                let span = Span { line: start_line, col: start_col, len: token.len() };
                return Ok((t, span));
            }

            let value = match token.parse::<i64>() {
                Ok(v) => v,
                Err(_) => {
                    let span = Span { line: start_line, col: start_col, len: token.len() };
                    return Err(LexError { kind: LexErrorKind::NumberOutOfRange(token), span });
                }
            };
            let t = Token::IntLiteral(value);
            let span = Span { line: start_line, col: start_col, len: token.len() };
            return Ok((t, span));
        }

        // Keywords and identifiers start with a letter or '_', anything else is unknown
        if !c.is_alphabetic() && c != '_' {
            self.pos += 1;
            self.col += 1;
            let span = Span { line: start_line, col: start_col, len: 1 };
            return Err(LexError { kind: LexErrorKind::UnexpectedChar(c), span });
        }

        // Lex keywords and identifiers
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lexes all of `source` until EOF, EOF excluded.
    fn lex_all(source: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(source.to_string());
        let mut tokens = Vec::new();
        loop {
            match lexer.get_next_token() {
                Ok((Token::EOF, _)) => return tokens,
                Ok((token, _)) => tokens.push(token),
                Err(e) => panic!("unexpected lex error {:?}", e),
            }
        }
    }

    /// Lexes `source` and returns the first error.
    fn lex_error(source: &str) -> LexError {
        let mut lexer = Lexer::new(source.to_string());
        loop {
            match lexer.get_next_token() {
                Ok((Token::EOF, _)) => panic!("expected a lex error in {:?}", source),
                Ok(_) => continue,
                Err(e) => return e,
            }
        }
    }

    fn ident(name: &str) -> Token {
        Token::Identifier(name.to_string())
    }

    #[test]
    fn empty_source_is_eof() {
        assert_eq!(lex_all(""), vec![]);
        assert_eq!(lex_all("   \n\t  \n"), vec![]);
    }

    #[test]
    fn eof_repeats() {
        let mut lexer = Lexer::new("x".to_string());
        lexer.get_next_token().unwrap();
        assert_eq!(lexer.get_next_token().unwrap().0, Token::EOF);
        assert_eq!(lexer.get_next_token().unwrap().0, Token::EOF);
    }

    #[test]
    fn keywords() {
        assert_eq!(
            lex_all("class Constructor enum false func import let match mut print pub return self true"),
            vec![
                Token::Class, Token::Constructor, Token::Enum, Token::False, Token::Func, Token::Import,
                Token::Let, Token::Match, Token::Mut, Token::Print, Token::Pub, Token::Return,
                Token::SelfRef, Token::True,
            ],
        );
    }

    #[test]
    fn type_keywords() {
        assert_eq!(
            lex_all("bool String int8 int16 int32 int64 uint8 uint16 uint32 uint64 float32 float64"),
            vec![
                Token::BoolTy, Token::StringTy, Token::Int8Ty, Token::Int16Ty, Token::Int32Ty, Token::Int64Ty,
                Token::UInt8Ty, Token::UInt16Ty, Token::UInt32Ty, Token::UInt64Ty, Token::Float32Ty, Token::Float64Ty,
            ],
        );
    }

    #[test]
    fn identifiers() {
        assert_eq!(lex_all("x foo_bar _hidden a1 int33"), vec![ident("x"), ident("foo_bar"), ident("_hidden"), ident("a1"), ident("int33")]);
    }

    #[test]
    fn keyword_prefix_is_identifier() {
        assert_eq!(lex_all("lets classy funcs"), vec![ident("lets"), ident("classy"), ident("funcs")]);
    }

    #[test]
    fn single_char_tokens() {
        assert_eq!(
            lex_all("( ) { } ; , : = + - * / . &"),
            vec![
                Token::LParen, Token::RParen, Token::LBrace, Token::RBrace, Token::Semicolon, Token::Comma,
                Token::Colon, Token::Assign, Token::Plus, Token::Minus, Token::Multiply, Token::Divide,
                Token::Dot, Token::BAnd,
            ],
        );
    }

    #[test]
    fn angle_brackets() {
        assert_eq!(lex_all("max<int64>(3)"), vec![
            ident("max"), Token::Lt, Token::Int64Ty, Token::Gt, Token::LParen, Token::IntLiteral(3), Token::RParen,
        ]);
    }

    #[test]
    fn arrows_take_precedence_over_angle_brackets() {
        assert_eq!(lex_all("-> => < >"), vec![Token::Arrow, Token::ThickArrow, Token::Lt, Token::Gt]);
    }

    #[test]
    fn rewind_goes_back_to_checkpoint() {
        let mut lexer = Lexer::new("a < b".to_string());
        lexer.get_next_token().unwrap();
        let checkpoint = lexer.checkpoint();
        lexer.get_next_token().unwrap();
        lexer.get_next_token().unwrap();
        lexer.rewind(checkpoint);
        let (token, span) = lexer.get_next_token().unwrap();
        assert_eq!(token, Token::Lt);
        assert_eq!(span.col, 2);
    }

    #[test]
    fn two_char_tokens() {
        assert_eq!(lex_all("-> :: =>"), vec![Token::Arrow, Token::ColonColon, Token::ThickArrow]);
    }

    #[test]
    fn two_char_tokens_without_spaces() {
        assert_eq!(lex_all("a::b"), vec![ident("a"), Token::ColonColon, ident("b")]);
        assert_eq!(lex_all(")->x"), vec![Token::RParen, Token::Arrow, ident("x")]);
    }

    #[test]
    fn int_literals() {
        assert_eq!(lex_all("0 7 42 2147483648"), vec![
            Token::IntLiteral(0), Token::IntLiteral(7), Token::IntLiteral(42), Token::IntLiteral(2147483648),
        ]);
    }

    #[test]
    fn int64_max_literal() {
        assert_eq!(lex_all("9223372036854775807"), vec![Token::IntLiteral(i64::MAX)]);
    }

    #[test]
    fn float_literals() {
        assert_eq!(lex_all("1.5 0.25 10.0"), vec![
            Token::FloatLiteral(1.5), Token::FloatLiteral(0.25), Token::FloatLiteral(10.0),
        ]);
    }

    #[test]
    fn dot_without_digit_ends_the_number() {
        assert_eq!(lex_all("5.x"), vec![Token::IntLiteral(5), Token::Dot, ident("x")]);
    }

    #[test]
    fn minus_is_a_separate_token() {
        assert_eq!(lex_all("-5"), vec![Token::Minus, Token::IntLiteral(5)]);
        assert_eq!(lex_all("a-b"), vec![ident("a"), Token::Minus, ident("b")]);
    }

    #[test]
    fn string_literals() {
        assert_eq!(lex_all("\"hello\" \"\" \"a b c\""), vec![
            Token::StringLiteral("hello".to_string()),
            Token::StringLiteral("".to_string()),
            Token::StringLiteral("a b c".to_string()),
        ]);
    }

    #[test]
    fn string_keeps_keywords_and_symbols() {
        assert_eq!(lex_all("\"let x = 5;\""), vec![Token::StringLiteral("let x = 5;".to_string())]);
    }

    #[test]
    fn statement() {
        assert_eq!(lex_all("let x: int32 = 5;"), vec![
            Token::Let, ident("x"), Token::Colon, Token::Int32Ty, Token::Assign, Token::IntLiteral(5), Token::Semicolon,
        ]);
    }

    #[test]
    fn span_positions() {
        let mut lexer = Lexer::new("let x\n  = 5;".to_string());
        let positions: Vec<(usize, usize, usize)> = std::iter::from_fn(|| match lexer.get_next_token().unwrap() {
            (Token::EOF, _) => None,
            (_, span) => Some((span.line, span.col, span.len)),
        }).collect();
        assert_eq!(positions, vec![(0, 0, 3), (0, 4, 1), (1, 2, 1), (1, 4, 1), (1, 5, 1)]);
    }

    #[test]
    fn span_after_string_with_newline() {
        let mut lexer = Lexer::new("\"a\nb\" x".to_string());
        lexer.get_next_token().unwrap();
        let (token, span) = lexer.get_next_token().unwrap();
        assert_eq!(token, ident("x"));
        assert_eq!((span.line, span.col), (1, 3));
    }

    #[test]
    fn peek_does_not_advance() {
        let mut lexer = Lexer::new("a b".to_string());
        assert_eq!(lexer.peek().unwrap().0, ident("a"));
        assert_eq!(lexer.peek().unwrap().0, ident("a"));
        assert_eq!(lexer.get_next_token().unwrap().0, ident("a"));
        assert_eq!(lexer.peek().unwrap().0, ident("b"));
    }

    #[test]
    fn unexpected_char() {
        let error = lex_error("let x = 5 @ 3;");
        assert!(matches!(error.kind, LexErrorKind::UnexpectedChar('@')));
        assert_eq!((error.span.line, error.span.col), (0, 10));
    }

    #[test]
    fn unexpected_chars() {
        for c in ['$', '#', '!', '?', '%', '|', '^', '~', '\''] {
            let error = lex_error(&format!("a {} b", c));
            assert!(matches!(error.kind, LexErrorKind::UnexpectedChar(got) if got == c), "for {:?}", c);
        }
    }

    #[test]
    fn unterminated_string() {
        let error = lex_error("print(\"hello);");
        assert!(matches!(error.kind, LexErrorKind::UnterminatedString));
        assert_eq!((error.span.line, error.span.col), (0, 6));
    }

    #[test]
    fn number_too_big() {
        let error = lex_error("99999999999999999999");
        assert!(matches!(error.kind, LexErrorKind::NumberOutOfRange(ref n) if n == "99999999999999999999"));
    }

    #[test]
    fn int64_max_plus_one_is_too_big() {
        let error = lex_error("9223372036854775808");
        assert!(matches!(error.kind, LexErrorKind::NumberOutOfRange(_)));
    }
}

