use std::mem::discriminant;

use crate::{declaration::Declaration, expression::Expr, lexer::{self, Lexer}, statement::Stmt, token::Token};

pub struct Parser {
    lexer: Lexer
}

impl Parser {
    pub fn new(string: String) -> Self {
        Self { lexer: Lexer::new(string) }
    }

    pub fn parse_statement(&mut self) -> Option<Stmt> {
        let cur = match self.lexer.get_next_token() {
            Some(token) => token,
            None => return None
        };

        println!("{}", cur);
        let stmt = match cur {
            Token::Func                 => self.parse_function_declaration(),
            Token::Let                  => self.parse_instantiation(),
            other                       => { 
                let expr = Stmt::Expression(self.parse_expression(other));
                self.lexer.expect(Token::Semicolon);
                return Some(expr);
            }
        };
        Some(stmt)
    }

    fn parse_expression(&mut self, cur: Token) -> Expr {
        match cur {
            Token::Print            => self.parse_print(),
            Token::Int32(value)     => Expr::Int32(value),
            Token::Identifier(name) => {
                if discriminant(&self.lexer.peek().unwrap()) == discriminant(&Token::LParen) {
                    self.lexer.get_next_token();
                    self.lexer.get_next_token();
                    return Expr::FunctionCall(name)
                }
                Expr::Identifier(name)
            }
            _ => panic!("Unknown token: {}", cur)
        }
    }

    fn next_expression(&mut self) -> Expr {
        let cur = self.lexer.get_next_token().expect("Expected expression, found nothin");
        self.parse_expression(cur)
    }

    fn parse_print(&mut self) -> Expr {
        self.lexer.expect(Token::LParen);
        let value: Expr = self.next_expression();
        self.lexer.expect(Token::RParen);
        return Expr::Print(Box::new(value));
    }

    fn parse_function_declaration(&mut self) -> Stmt {
        let name = self.lexer.expect(Token::Identifier(String::new()));
        let params: Vec<Expr> = self.parse_params();
        let body = self.parse_body();

        return
        Stmt::Declaration(Declaration::Function { 
            name: name.to_string(), 
            params: params, 
            body: body, 
            return_type: None,
        })
    }

    fn parse_instantiation(&mut self) -> Stmt {
        let name = self.lexer.expect(Token::Identifier(String::new()));
        println!("{}", name);
        self.lexer.expect(Token::Colon);
        let typename = self.lexer.expect(Token::Identifier(String::new()));
        self.lexer.expect(Token::Equal);
        let value = self.next_expression();
        self.lexer.expect(Token::Semicolon);

        return Stmt::Instantiation { 
            name: name.to_string(),
            typedef: typename.to_type(),
            value: value
        }
    }

    fn parse_params(&mut self) -> Vec<Expr> {
        self.lexer.expect(Token::LParen);
        let mut params: Vec<Expr> = Vec::new();
        while discriminant(&self.lexer.peek().unwrap()) != discriminant(&Token::RParen) {
            let param = self.parse_param();
            params.push(param);
            self.lexer.expect(Token::Comma);
        }
        self.lexer.expect(Token::RParen);
        return params;
    }

    fn parse_param(&mut self) -> Expr {
        let mut token =  self.lexer.get_next_token().unwrap();
        let name = self.parse_expression(token);
        self.lexer.expect(Token::Colon);
        token =  self.lexer.get_next_token().unwrap();
        let typename = self.parse_expression(token);

        let mut default_value = None;
        if discriminant(&self.lexer.peek().unwrap()) == discriminant(&Token::Equal) {
            // Consume Token::Equal
            self.lexer.get_next_token().unwrap();

            token =  self.lexer.get_next_token().unwrap();
            default_value = Some(Box::new(self.parse_expression(token)));
        }

        Expr::Param { name: name.to_string(), typename: typename.to_string(), default_value: default_value}
    }

    fn parse_body(&mut self) -> Vec<Stmt> {
        self.lexer.expect(Token::LBrace);

        let mut body: Vec<Stmt> = Vec::new();
        while discriminant(&self.lexer.peek().unwrap()) != discriminant(&Token::RBrace) {
            body.push(self.parse_statement().unwrap());
        }

        self.lexer.expect(Token::RBrace);
        return body;
    }
}

