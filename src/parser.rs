use std::mem::discriminant;

use crate::{declaration::{Declaration, DeclarationKind}, expression::{Expr, ExprKind}, field::Field, lexer::Lexer, param::Param, span::Span, statement::{MatchArm, Pattern, Stmt, StmtKind}, token::Token, r#type::Type};

enum ParseError {
    ParamTypeMissing { span: Span, context: &'static str },
    ParamTypeNotAllowed { span: Span },
    UnexpectedEOF { span: Span, context: &'static str },
    UnexpectedToken { expected: Token, got: Token, span: Span, context: &'static str },
}

impl ParseError {
    fn throw(self, source: &str) -> ! {
        match self {
            ParseError::ParamTypeMissing { span, context } => {
                panic!("\nerror: parameter needs a clear type declaration {} \n  --> {}:{}\n",
                    context, span.line + 1, span.col + 1)
            }
            ParseError::ParamTypeNotAllowed { span } => {
                panic!("\nerror: parameter name can only be an Identifier or self \n  --> {}:{}\n",
                    span.line + 1, span.col + 1)
            }
            ParseError::UnexpectedEOF { span, context } => {
                panic!("\nerror: unexpected end of file {}\n  --> {}:{}\n",
                    context, span.line + 1, span.col + 1)
            }
            ParseError::UnexpectedToken { expected, got, span, context } => {
                let line_txt = source.lines().nth(span.line).unwrap_or("");
                let pointer = " ".repeat(span.col) + "^";
                panic!("\nerror: expected '{}' {}, got '{}'\n  --> {}:{}\n   |\n   | {}\n   | {}\n",
                    expected, context, got, span.line + 1, span.col + 1, line_txt, pointer)
            }
        }
    }
}

pub struct Parser {
    lexer: Lexer,
    allow_brace_init: bool,
}

impl Parser {
    pub fn new(string: &String) -> Self {
        Self { lexer: Lexer::new(string.to_string()), allow_brace_init: true }
    }

    fn expect(&mut self, token: Token, context: &'static str) -> (Token, Span) {
        match self.lexer.expect(token.clone()) {
            Ok(t) => t,
            Err((got, span)) => {
                if matches!(got, Token::EOF) {
                    ParseError::UnexpectedEOF { span, context }.throw(&self.lexer.source)
                } else {
                    ParseError::UnexpectedToken { expected: token, got, span, context }.throw(&self.lexer.source)
                }
            }
        }
    }

    fn peek_or_eof(&mut self, context: &'static str) -> (Token, Span) {
        match self.lexer.peek() {
            Ok(t) => t,
            Err(t)=> {
                if t.0 == Token::EOF {
                    return t
                }

                ParseError::UnexpectedEOF {
                    span: Span { line: 0, col: 0, len: 0 },
                    context,
                }.throw(&self.lexer.source)
            }
        }
    }

    // --- Statement Parsing ---
    pub fn parse_statement(&mut self) -> Option<Stmt> {
        let (cur, span) = match self.lexer.get_next_token() {
            Ok(t) => t,
            Err((Token::EOF, _)) => return None,
            Err((_, span)) => ParseError::UnexpectedEOF {
                span,
                context: "expected expression",
            }.throw(&self.lexer.source),
        };

        if &discriminant(&cur) == &discriminant(&Token::EOF) {
            return None;
        }

        let kind = match cur {
            Token::Func => StmtKind::Declaration(Declaration::new(
                self.parse_function_declaration(None, false),
                span.clone(),
                Type::Unknown,
            )),
            Token::Class => StmtKind::Declaration(Declaration::new(
                self.parse_class_declaration(),
                span.clone(),
                Type::Unknown,
            )),
            Token::Enum => StmtKind::Declaration(Declaration::new(
                    self.parse_enum_declaration(), 
                    span.clone(), 
                    Type::Unknown
            )),
            Token::Mut    => self.parse_variable(true),
            Token::Match  => self.parse_match(),
            Token::Let    => self.parse_variable(false),
            Token::Return => self.parse_return(),
            other         => self.parse_expr_stmt(other, span.clone()),
        };
        Some(Stmt::new(kind, span, Type::Unknown))
    }

    fn parse_match(&mut self) -> StmtKind {
        let prev_brace = self.allow_brace_init;
        self.allow_brace_init = false;
        let scrutinee = self.next_expression();
        self.allow_brace_init = prev_brace;
        self.expect(Token::LBrace, "at start of match body");

        let mut arms: Vec<MatchArm> = Vec::new();
        while !matches!(self.peek_or_eof("in match body"), (Token::RBrace, _)) {
            arms.push(self.parse_match_arm());
            if matches!(self.peek_or_eof("after match arm"), (Token::Comma, _)) {
                self.lexer.consume();
            }
        }
        self.expect(Token::RBrace, "at end of match body");
        StmtKind::Match { scrutinee, arms }
    }

    fn parse_match_arm(&mut self) -> MatchArm {
        let (pattern, arm_span) = self.parse_pattern();
        self.expect(Token::ThickArrow, "after match pattern");

        let body = if matches!(self.peek_or_eof("match arm body"), (Token::LBrace, _)) {
            self.parse_body()
        } else {
            let expr = self.next_expression();
            let expr_span = expr.span.clone();
            vec![Stmt::new(StmtKind::Expression(expr), expr_span, Type::Unknown)]
        };

        MatchArm { pattern, body, span: arm_span }
    }

    fn parse_pattern(&mut self) -> (Pattern, Span) {
        let (tok, span) = match self.lexer.get_next_token() {
            Ok(t) => t,
            Err((_, span)) => ParseError::UnexpectedEOF { span, context: "expected pattern" }
                .throw(&self.lexer.source),
        };

        match tok {
            Token::Identifier(name) if name == "_" => (Pattern::Wildcard, span),
            Token::Identifier(name) => {
                if matches!(self.lexer.peek(), Ok((Token::ColonColon, _))) {
                    self.lexer.consume();
                    let (variant_tok, _) = self.expect(
                        Token::Identifier("variant".to_string()),
                        "after '::' in pattern",
                    );
                    let variant = variant_tok.to_string();
                    let binding = if matches!(self.lexer.peek(), Ok((Token::LParen, _))) {
                        self.lexer.consume();
                        let (bind_tok, _) = self.expect(
                            Token::Identifier("binding".to_string()),
                            "in pattern binding",
                        );
                        self.expect(Token::RParen, "after pattern binding");
                        Some(bind_tok.to_string())
                    } else {
                        None
                    };
                    (Pattern::EnumVariant { enum_name: name, variant, binding }, span)
                } else {
                    (Pattern::Literal(Expr::new(ExprKind::Identifier(name), span.clone())), span)
                }
            }
            Token::True  => (Pattern::Literal(Expr::new(ExprKind::Boolean(true),  span.clone())), span),
            Token::False => (Pattern::Literal(Expr::new(ExprKind::Boolean(false), span.clone())), span),
            Token::Numeral(v) => (Pattern::Literal(Expr::new(ExprKind::Int32(v), span.clone())), span),
            Token::StringLiteral(s) => (Pattern::Literal(Expr::new(ExprKind::StringLiteral(s), span.clone())), span),
            other => panic!("\nerror: '{}' is not a valid pattern\n  --> {}:{}\n",
                other, span.line + 1, span.col + 1),
        }
    }

    fn parse_function_declaration(&mut self, parent: Option<String>, public: bool) -> DeclarationKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in function declaration");
        let params = self.parse_params(parent.as_deref());
        let return_type = self.parse_return_type();
        let body = self.parse_body();

        DeclarationKind::Function {
            name: name.to_string(),
            params,
            body: Some(body),
            return_type,
            parent,
            mangled: None,
            public,
        }
    }

    fn parse_return_type(&mut self) -> Option<Type> {
        if !matches!(self.lexer.peek(), Ok((Token::Arrow, _))) {
            return None;
        }
        self.lexer.consume(); // consume '->'
        let ty = self.parse_type();
        Some(ty.0)
    }

    fn next_identifier(&mut self, context: &'static str) -> Expr {
        let (cur, span) = match self.lexer.get_next_token() {
            Ok(t) => t,
            Err((_, span)) => ParseError::UnexpectedEOF {
                span,
                context: "expected expression",
            }.throw(&self.lexer.source),
        };

        match cur {
            Token::Identifier(name) => Expr::new(ExprKind::Identifier(name), span),
            t => ParseError::UnexpectedToken { expected: Token::Identifier("".to_string()), got: t, span, context }.throw(&self.lexer.source)
        }
    }

    fn parse_body(&mut self) -> Vec<Stmt> {
        self.expect(Token::LBrace, "at start of body");

        let mut body: Vec<Stmt> = Vec::new();
        while !matches!(self.peek_or_eof("while parsing body"), (Token::RBrace, _)) {
            match self.parse_statement() {
                Some(s) => body.push(s),
                None => panic!("unexpected end of file while parsing body"),
            }
        }

        self.expect(Token::RBrace, "at end of body");
        body
    }

    fn parse_return(&mut self) -> StmtKind {
        let expr = self.next_expression();
        self.expect(Token::Semicolon, "after return value");
        StmtKind::Return(expr)
    }

    // Class Parsing
    fn parse_class_declaration(&mut self) -> DeclarationKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in class declaration");
        let name = name.to_string();
        let (fields, methods) = self.parse_class_body(name.clone());

        DeclarationKind::Class { name, fields, methods }
    }

    fn parse_class_body(&mut self, class_name: String) -> (Vec<Field>, Vec<Declaration>) {
        self.expect(Token::LBrace, "at start of class body");

        let mut fields: Vec<Field> = Vec::new();
        let mut methods: Vec<Declaration> = Vec::new();
        while !matches!(self.peek_or_eof("while parsing class body"), (Token::RBrace, _)) {
            self.parse_class_body_stmt(&class_name, &mut fields, &mut methods);
        }

        self.expect(Token::RBrace, "at end of class body");
        (fields, methods)
    }

    fn parse_class_body_stmt(&mut self, class_name: &String, fields: &mut Vec<Field>, methods: &mut Vec<Declaration>) {
        let mut public = false;
        if matches!(self.lexer.peek(), Ok((Token::Pub, _))) {
            self.lexer.consume();
            public = true;
        }

        let (next, next_span) = self.peek_or_eof("while parsing class body");
        match next {
            Token::Identifier(_) => {
                let field = self.parse_field(public);
                fields.push(field);
                self.expect(Token::Semicolon, "after class field");
            }
            Token::Constructor => {
                if public {
                    panic!("Constructors are always public — `pub` is redundant");
                }
                self.lexer.consume(); // consume 'Constructor'
                let decl_kind = self.parse_constructor(&class_name);
                methods.push(Declaration::new(decl_kind, next_span, Type::Class(class_name.clone())));
            }
            Token::Func => {
                self.lexer.consume(); // consume 'func'
                let decl_kind = self.parse_function_declaration(Some(class_name.clone()), public);
                methods.push(Declaration::new(decl_kind, next_span, Type::Unknown));
            }
            tok => ParseError::UnexpectedToken {
                expected: Token::Identifier("field or method".to_string()),
                got: tok,
                span: next_span,
                context: "in class body",
            }.throw(&self.lexer.source),
        }
    }

    fn parse_field(&mut self, public: bool) -> Field {
        let first_param = self.next_expression();
        let name = match first_param.kind {
            ExprKind::Identifier(name) => name.to_string(),
            _ => ParseError::ParamTypeNotAllowed { span: first_param.span }.throw(&self.lexer.source)
        };

        self.expect(Token::Colon, "after parameter name");
        let (ty, ty_span) = self.parse_type();

        let mut default_value = None;
        if matches!(self.lexer.peek(), Ok((Token::Assign, _))) {
            self.lexer.consume(); // consume '='
            default_value = Some(Box::new(self.next_expression()));
        }

        Field {
            name: name.to_string(),
            ty,
            ty_span,
            value: default_value,
            public
        }
    }

    fn parse_type(&mut self) -> (Type, Span) {
        let (tok, span) = self.peek_or_eof("as parameter type");
        match tok {
            Token::BAnd => {
                self.lexer.consume(); // '&'
                let (inner, inner_span) = self.parse_type();
                (Type::Ref(Box::new(inner)), inner_span)
            }
            Token::Identifier(_) => {
                let (tok, ty_span) = self.expect(Token::Identifier("param type".to_string()), "as parameter type");
                let name = match tok {
                    Token::Identifier(n) => n,
                    _ => unreachable!(),
                };
                let ty = Type::Class(name.to_string());
                (ty, ty_span)
            }
            tok => {
                self.lexer.consume();
                (tok.to_type(), span)
            }
        }
    }

    fn parse_constructor(&mut self, type_name: &String) -> DeclarationKind {
        let params = self.parse_params(Some(type_name));
        let return_type = self.parse_return_type();
        let body = if matches!(self.lexer.peek().unwrap(), (Token::LBrace, _)) {
            Some(self.parse_body())
        } else {
            self.expect(Token::Semicolon, "after constructor declaration without body");
            None
        };

        DeclarationKind::Function {
            name: type_name.to_string(),
            params,
            body,
            return_type,
            parent: None,
            mangled: None,
            public: true,
        }
    }

    // Parameter Parsing
    fn parse_params(&mut self, parent: Option<&str>) -> Vec<Param> {
        self.expect(Token::LParen, "at start of parameter list");
        let mut params: Vec<Param> = Vec::new();

        while !matches!(self.peek_or_eof("while parsing parameters"), (Token::RParen, _)) {
            params.push(self.parse_param(parent));
            if !matches!(self.peek_or_eof("after parameter"), (Token::RParen, _)) {
                self.expect(Token::Comma, "between parameters");
            }
        }
        self.expect(Token::RParen, "at end of parameter list");
        params
    }

    fn parse_param(&mut self, parent: Option<&str>) -> Param {
        let mut mutable = false;
        if matches!(self.lexer.peek(), Ok((Token::Mut, _))) {
            self.lexer.consume();
            mutable = true;
        }

        if matches!(self.lexer.peek().unwrap().0, Token::BAnd) {
            self.lexer.consume();
        }

        let first_param = self.next_expression();
        let name = match first_param.kind {
            ExprKind::Identifier(name) => name.to_string(),
            _ => ParseError::ParamTypeNotAllowed { span: first_param.span }.throw(&self.lexer.source)
        };
        if name == "self" {
            let class_name = parent
                .expect("`self` parameter outside of a class method")
                .to_string();
            return Param {
                name,
                ty: Type::Class(class_name),
                ty_span: first_param.span,
                value: None,
                mutable,
            };
        }


        self.expect(Token::Colon, "after parameter name");
        let (ty, ty_span) = self.parse_type();

        let mut default_value = None;
        if matches!(self.lexer.peek(), Ok((Token::Assign, _))) {
            self.lexer.consume(); // consume '='
            default_value = Some(Box::new(self.next_expression()));
        }

        Param {
            name: name.to_string(),
            ty,
            ty_span,
            value: default_value,
            mutable,
        }
    }


    // Parse Enum
    
    fn parse_enum_declaration(&mut self) -> DeclarationKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in class declaration");
        let name = name.to_string();
        let fields = self.parse_enum_body();

        DeclarationKind::Enum{ name, fields }
    }

    fn parse_enum_body(&mut self) -> Vec<(String, u32, Type, Span)>{
        self.expect(Token::LBrace, "enum body");
        let mut index: u32 = 0;
        let mut fields: Vec<(String, u32, Type, Span)> = Vec::new();
        while !matches!(self.peek_or_eof("enum body"), (Token::RBrace, _)) {
            let name = self.expect(Token::Identifier("enum name".to_string()), "in enum body").0.to_string();
            self.expect(Token::ThickArrow, "enum body");
            let (ty, ty_span) = self.parse_type();
            fields.push((name, index, ty, ty_span));
            index += 1;

            if matches!(self.peek_or_eof(", or }"), (Token::Comma, _)) {
                self.lexer.consume();
            }
        }
        self.expect(Token::RBrace, "enum body");

        fields
    }

    fn parse_variable(&mut self, mutable: bool) -> StmtKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in variable declaration");
        let typedef = if matches!(self.peek_or_eof("at instantiation"), (Token::Colon, _)) {
            self.lexer.consume();
            let typedef = self.parse_type();
            Some(typedef.0)
        } else { None };

        self.expect(Token::Assign, "after type in variable declaration");

        let value = match self.lexer.peek() {
            Ok((Token::LBrace, span)) => {
                let span = span.clone();
                self.lexer.consume(); // consume '{'
                let kind = self.parse_brace_inst(typedef.clone().expect("type undefined").to_string());
                Expr::new(kind, span)
            }
            _ => self.next_expression(),
        };
        self.expect(Token::Semicolon, "after variable declaration");

        StmtKind::Instantiation {
            name: name.to_string(),
            typedef,
            value,
            mutable
        }
    }

    fn parse_brace_inst(&mut self, class: String) -> ExprKind {
        let mut fields: Vec<(String, Expr)> = Vec::new();

        while !matches!(self.peek_or_eof("in struct initializer"), (Token::RBrace, _)) {
            let (name_tok, _) = self.expect(
                Token::Identifier(String::new()),
                "as field name in struct initializer",
            );
            self.expect(Token::Colon, "after field name in struct initializer");
            let value = self.next_expression();
            fields.push((name_tok.to_string(), value));

            if !matches!(self.peek_or_eof("after struct field"), (Token::RBrace, _)) {
                self.expect(Token::Comma, "between struct fields");
            }
        }
        self.lexer.consume(); // consume '}'

        ExprKind::StructInit { class, fields }
    }

    // Expr Stmt Parsing
    fn parse_expr_stmt(&mut self, cur_token: Token, span: Span) -> StmtKind {
        let expr = self.parse_expression(cur_token, span);
        self.expect(Token::Semicolon, "after expression statement");
        StmtKind::Expression(expr)
    }

    // --- Expression Parsing ---
    fn parse_expression(&mut self, cur: Token, span: Span) -> Expr {
        let lhs_kind = match cur {
            Token::False            => ExprKind::Boolean(false),
            Token::Print            => self.parse_print(),
            Token::Numeral(value)   => ExprKind::Int32(value),
            Token::Identifier(name) => self.parse_identifier(name, None, span.clone()),
            Token::True             => ExprKind::Boolean(true),
            Token::BAnd             => self.parse_refrence(),
            Token::SelfRef          => self.parse_identifier("self".to_string(), None, span.clone()),
            Token::StringLiteral(literal) => ExprKind::StringLiteral(literal),
            other => panic!("\nerror: '{}' is not a valid expression\n  --> {}:{}\n",
                other, span.line + 1, span.col + 1),
        };

        match self.lexer.peek() {
            Ok((Token::Plus, _)) | Ok((Token::Minus, _)) |
            Ok((Token::Multiply, _)) | Ok((Token::Divide, _)) => {
                let (op, _) = match self.lexer.get_next_token() {
                    Ok(t) => t,
                    Err((_, span)) => ParseError::UnexpectedEOF {
                        span,
                        context: "expected expression",
                    }.throw(&self.lexer.source),
                };

                let lhs = Expr::new(lhs_kind, span.clone());
                let rhs = self.next_expression();
                Expr::new(
                    ExprKind::BinaryExpr { lhs: Box::new(lhs), op, rhs: Box::new(rhs) },
                    span,
                )
            }
            _ => Expr::new(lhs_kind, span)
        }
    }

    fn next_expression(&mut self) -> Expr {
        let (cur, span) = match self.lexer.get_next_token() {
            Ok(t) => t,
            Err((_, span)) => ParseError::UnexpectedEOF {
                span,
                context: "expected expression",
            }.throw(&self.lexer.source),
        };
        self.parse_expression(cur, span)
    }

    // Print Parsing
    fn parse_print(&mut self) -> ExprKind {
        self.expect(Token::LParen, "after 'print'");
        let value = self.next_expression();
        self.expect(Token::RParen, "after print argument");
        ExprKind::Print(Box::new(value))
    }

    // Identifier Parsing
    fn parse_identifier(&mut self, identifier: String, prev: Option<String>, span: Span) -> ExprKind {
        match self.lexer.peek() {
            Ok((Token::ColonColon, _))  => self.parse_colon_access(identifier),
            Ok((Token::LBrace, _)) if self.allow_brace_init => {
                self.lexer.consume();
                return self.parse_brace_inst(identifier);
            }
            Ok((Token::LParen, _)) => self.parse_function_call(identifier, prev),
            Ok((Token::Dot, _))    => self.parse_field_access(identifier),
            Ok((Token::Assign, _)) => self.parse_assignment(identifier, span),
            _                      => ExprKind::Identifier(identifier),
        }
    }

    fn parse_colon_access(&mut self, name: String) -> ExprKind {
        self.expect(Token::ColonColon, "in colon access");
        let (tag, _span) = self.expect(Token::Identifier("tag".to_string()), "at tag");

        if matches!(self.peek_or_eof("enum tag construction"), (Token::LParen, _)){
            self.lexer.consume();
            let expr = self.next_expression();
            self.expect(Token::RParen, "end of enum const");
            return ExprKind::EnumInit { name, tag: tag.to_string(), value: Box::new(expr)};
        }

        if let (Token::LBrace, brace_span) = self.peek_or_eof("enum tag construction") {
            self.lexer.consume(); // consume '{'
            // Placeholder class name — sema replaces it with the variant's payload type.
            let inner_kind = self.parse_brace_inst(name.clone());
            let inner_expr = Expr::new(inner_kind, brace_span);
            return ExprKind::EnumInit { name, tag: tag.to_string(), value: Box::new(inner_expr) };
        }

        ExprKind::ColonAccess { name, tag: tag.to_string() }
    }

    fn parse_function_call(&mut self, name: String, parent_name: Option<String>) -> ExprKind {
        self.lexer.consume(); // consume '('

        let mut params: Vec<Expr> = Vec::new();
        while !matches!(self.peek_or_eof("while parsing function call arguments"), (Token::RParen, _)) {
            params.push(self.next_expression());
            if !matches!(self.peek_or_eof("after function call argument"), (Token::RParen, _)) {
                self.expect(Token::Comma, "between function call arguments");
            }
        }
        self.lexer.consume(); // consume ')'
        ExprKind::FunctionCall { name, params, parent_name, mangled: None }
    }

    fn parse_field_access(&mut self, name: String) -> ExprKind {
        let (_, dot_span) = self.expect(Token::Dot, "in field access");
        let (tok, field_span) = self.expect(Token::Identifier("field name".to_string()), "after '.'");
        let field_name = match tok {
            Token::Identifier(n) => n,
            _ => unreachable!(),
        };

        if matches!(self.lexer.peek(), Ok((Token::LParen, _))) {
            return self.parse_function_call(field_name, Some(name));
        }

        let field_expr = Expr::new(ExprKind::Identifier(field_name), field_span);
        let fa_kind = ExprKind::FieldAccess { name, field: Box::new(field_expr) };

        if matches!(self.lexer.peek(), Ok((Token::Assign, _))) {
            self.lexer.consume(); // '='
            let lhs = Expr::new(fa_kind, dot_span);
            let rhs = self.next_expression();
            return ExprKind::Assignment {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }

        fa_kind
    }

    fn parse_assignment(&mut self, name: String, span: Span) -> ExprKind {
        self.lexer.consume(); // consume '='
        let lhs = Expr::new(ExprKind::Identifier(name), span);
        let rhs = self.next_expression();
        ExprKind::Assignment {
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    fn parse_refrence(&mut self) -> ExprKind {
        let inner = self.next_expression();
        ExprKind::Ref(Box::new(inner))
    }
}
