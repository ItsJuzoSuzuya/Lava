use std::{any::type_name, mem::discriminant};

use crate::{declaration::Declaration, expression::Expr, lexer::Lexer, statement::{Param, Stmt}, token::Token, r#type::Type};

pub struct Parser {
    lexer: Lexer
}

impl Parser {
    pub fn new(string: String) -> Self {
        Self { lexer: Lexer::new(string) }
    }

    // --- Statement Parsing --- //
    pub fn parse_statement(&mut self) -> Option<Stmt> {
        let cur = match self.lexer.get_next_token() {
            Some(token) => token,
            None => return None,
        };

        let token = match cur.token {
            Token::Func                 => self.parse_function_declaration(),
            Token::Class                => self.parse_class_declaration(),
            Token::Let                  => self.parse_instantiation(),
            Token::Return               => self.parse_return(),
            other                       => self.parse_expr_stmt(other),
        };
        Some(token)
    }

    // Function Parsing 
    fn parse_function_declaration(&mut self) -> Stmt {
        let name = self.lexer.expect(Token::Identifier(String::new()));
        let params: Vec<Param> = self.parse_params();
        let return_type: Option<Type> = self.parse_return_type().map(Type::from);
        let body = self.parse_body();

        return
        Stmt::Declaration(Declaration::Function { 
            name: name.token.to_string(), 
            params: params, 
            body: body, 
            return_type: return_type,
        })
    }

    fn parse_body(&mut self) -> Vec<Stmt> {
        self.lexer.expect(Token::LBrace);

        let mut body: Vec<Stmt> = Vec::new();
        while discriminant(&self.lexer.peek().unwrap().token) != discriminant(&Token::RBrace) {
            let stmt = match self.parse_statement() {
                Some(s) => s,
                None => panic!("File ended unexpectedly while parsing body")
            };
            body.push(stmt);
        }

        self.lexer.expect(Token::RBrace);
        return body;
    }

    fn parse_return(&mut self) -> Stmt {
        let expr = self.next_expression();
        self.lexer.expect(Token::Semicolon);
        Stmt::Return(expr)
    }

    // Class Parsing
    fn parse_class_declaration(&mut self) -> Stmt {
        let name = self.lexer.expect(Token::Identifier(String::new()));
        let (fields, methods) = self.parse_class_body(name.token.to_string());

        return
        Stmt::Declaration(Declaration::Class{ 
            name: name.token.to_string(), 
            fields: fields, 
            methods: methods, 
        })
    }

    fn parse_class_body(&mut self, class_name: String) -> (Vec<Param>, Vec<Stmt>) {
        self.lexer.expect(Token::LBrace);

        let mut fields: Vec<Param> = Vec::new();
        let mut methods: Vec<Stmt> = Vec::new();
        while discriminant(&self.lexer.peek().unwrap().token) != discriminant(&Token::RBrace) {
            let next = self.lexer.peek().unwrap();
            match next.token {
                Token::Identifier(_) => {
                    fields.push(self.parse_param());
                    self.lexer.expect(Token::Semicolon);
                },
                Token::Constructor => {
                    self.lexer.get_next_token();
                    methods.push(self.parse_constructor(&class_name));
                },
                Token::Func => {
                    let method = match self.parse_function_declaration() {
                        Stmt::Declaration(decl) => match decl {
                            Declaration::Function { mut name, params, body, return_type } => {
                                name = class_name.clone() + "_" + &name;
                                Stmt::Declaration(Declaration::Function { name, params, body, return_type })
                            }
                            _ => panic!()
                        }
                        _ => panic!()
                    };
                    methods.push(method);
                }
                _ => {
                    let stmt = match self.parse_statement() {
                        Some(s) => s,
                        None => panic!("File ended unexpectedly while parsing body")
                    };
                    methods.push(stmt);
                }
            }

        }

        self.lexer.expect(Token::RBrace);
        return (fields, methods);
    }

    fn parse_constructor(&mut self, type_name: &String) -> Stmt{
        let params: Vec<Param> = self.parse_params();
        let return_type: Option<Type> = self.parse_return_type().map(Type::from);
        let body = self.parse_body();
        println!("Oh a Constructor! Lets Build A Function! {}()", type_name);

        return
        Stmt::Declaration(Declaration::Function { 
            name: type_name.to_string(), 
            params: params, 
            body: body, 
            return_type: return_type,
        })
    }

    fn parse_instantiation(&mut self) -> Stmt {
        let name = self.lexer.expect(Token::Identifier(String::new()));
        println!("{}", name.token);
        self.lexer.expect(Token::Colon);
        let typename = self.lexer.expect(Token::Identifier(String::new()));
        self.lexer.expect(Token::Equal);
        let value = self.next_expression();
        self.lexer.expect(Token::Semicolon);

        return Stmt::Instantiation { 
            name: name.token.to_string(),
            typedef: typename.token.to_type(),
            value: value
        }
    }

    // Expr Stmt Parsing
    fn parse_expr_stmt(&mut self, cur_token: Token) -> Stmt {
        let expr = Stmt::Expression(self.parse_expression(cur_token));
        self.lexer.expect(Token::Semicolon);
        return expr;
    }

    // --- Expression Parsing --- //
    fn parse_expression(&mut self, cur: Token) -> Expr {
        let lhs = match cur {
            Token::Print            => self.parse_print(),
            Token::Numeral(value)   => Expr::Int32(value),
            Token::Identifier(name) => self.parse_identifier(name),
            _ => panic!("Not a valid expression token: {}", cur)
        };

        match self.lexer.peek().unwrap().token {
            Token::Plus | Token::Minus | Token::Multiply | Token::Divide => {
                let op = self.lexer.get_next_token().unwrap().token;
                let rhs = self.next_expression();
                Expr::BinaryExpr { lhs: Box::new(lhs), op: op, rhs: Box::new(rhs)}
            }
            _ => lhs
        }
    }

    fn next_expression(&mut self) -> Expr {
        let cur = self.lexer.get_next_token().expect("Expected expression, found nothing");
        self.parse_expression(cur.token)
    }

    // Print Parsing
    fn parse_print(&mut self) -> Expr {
        self.lexer.expect(Token::LParen);
        let value: Expr = self.next_expression();
        self.lexer.expect(Token::RParen);
        return Expr::Print(Box::new(value));
    }

    // Identifier Parsing
    fn parse_identifier(&mut self, name: String) -> Expr {
        let peek = match self.lexer.peek() {
            Some(t) => t,
            None => return Expr::Identifier(name)
        };

        match peek.token {
            Token::LParen => self.parse_function_call(name),
            Token::Dot => self.parse_field_access(name),
            _ => return Expr::Identifier(name),
        }
    }

    fn parse_function_call(&mut self, name: String) -> Expr {
            self.lexer.get_next_token();
            let mut params: Vec<Expr> = Vec::new();
            while discriminant(&self.lexer.peek().unwrap().token) != discriminant(&Token::RParen) {
                let cur = self.lexer.get_next_token().unwrap();
                params.push(self.parse_expression(cur.token));
                if discriminant(&self.lexer.peek().unwrap().token) != discriminant(&Token::RParen) {
                    self.lexer.expect(Token::Comma);
                }
            }
            self.lexer.get_next_token();
            return Expr::FunctionCall{ name, params }
    }

    fn parse_field_access(&mut self, name: String) -> Expr {
        self.lexer.get_next_token();
        let field = Box::new(self.next_expression());
        return Expr::FieldAccess{ name, field }
    }

    // Parameter Parsing
    fn parse_params(&mut self) -> Vec<Param> {
        self.lexer.expect(Token::LParen);
        let mut params: Vec<Param> = Vec::new();
        while discriminant(&self.lexer.peek().unwrap().token) != discriminant(&Token::RParen) {
            let param = self.parse_param();
            params.push(param);
            if discriminant(&self.lexer.peek().unwrap().token) != discriminant(&Token::RParen) {
                self.lexer.expect(Token::Comma);
            }
        }
        self.lexer.expect(Token::RParen);
        return params;
    }

    fn parse_param(&mut self) -> Param {
        let name = self.next_expression();
        self.lexer.expect(Token::Colon);
        let typename = self.next_expression();

        let mut default_value = None;
        if discriminant(&self.lexer.peek().unwrap().token) == discriminant(&Token::Equal) {
            // Consume Token::Equal
            self.lexer.get_next_token().unwrap();

            default_value = Some(Box::new(self.next_expression()));
        }

        Param { name: name.to_string(), ty: Type::from(typename), value: default_value}
    }

    fn parse_return_type(&mut self) -> Option<Expr> {
        if discriminant(&self.lexer.peek().unwrap().token) != discriminant(&Token::Arrow) {
            return None;
        }

        self.lexer.get_next_token();
        let rt = self.next_expression();
        return Some(rt);
    }
}

