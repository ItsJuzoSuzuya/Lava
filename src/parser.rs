use std::mem::discriminant;

use crate::{declaration::{Declaration, DeclarationKind, Visibility}, expression::{Expr, ExprKind}, field::Field, lexer::{LexError, LexErrorKind, Lexer}, param::Param, span::Span, statement::{MatchArm, Pattern, Stmt, StmtKind}, token::Token, r#type::Type};
use crate::ModuleGraph;

/// Errors that can occur while parsing a source file.
enum ParseError {
    ParamTypeMissing { span: Span, context: &'static str },
    ParamTypeNotAllowed { span: Span },
    UnexpectedEOF { span: Span, context: &'static str },
    UnexpectedToken { expected: Token, got: Token, span: Span, context: &'static str },
    ExpectedDeclaration { got: Token, span: Span, context: &'static str },
    Lex(LexError),
}

impl ParseError {
    /// Panics with an error message for this error.
    ///
    /// `source` is the parsed source code, used to print the offending line.
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
            ParseError::ExpectedDeclaration { got, span, context } => {
                let line_txt = source.lines().nth(span.line).unwrap_or("");
                let pointer = " ".repeat(span.col) + "^";
                panic!("\nerror: expected declaration {}, got '{}'\n  --> {}:{}\n   |\n   | {}\n   | {}\n",
                    context, got, span.line + 1, span.col + 1, line_txt, pointer)
            }
            ParseError::Lex(LexError { kind, span }) => {
                let msg = match kind {
                    LexErrorKind::UnexpectedChar(c)      => format!("unexpected character '{}'", c),
                    LexErrorKind::UnterminatedString     => "unterminated string literal".to_string(),
                    LexErrorKind::NumberOutOfRange(num)  => format!("number '{}' doesn't fit into int64", num),
                };
                let line_txt = source.lines().nth(span.line).unwrap_or("");
                let pointer = " ".repeat(span.col) + "^";
                panic!("\nerror: {}\n  --> {}:{}\n   |\n   | {}\n   | {}\n",
                    msg, span.line + 1, span.col + 1, line_txt, pointer)
            }
        }
    }
}

/// Turns source code into an abstract syntax tree of [`Stmt`]s.
///
/// Pulls tokens from a [`Lexer`] on demand while parsing.
pub struct Parser {
    lexer: Lexer,
    /// Whether `Name { ... }` may be parsed as a brace initializer.
    ///
    /// Turned off while parsing a `match` scrutinee, where the `{` after an
    /// identifier opens the match body instead. Save the old value and restore
    /// it afterwards rather than resetting to `true`, so nested contexts work.
    allow_brace_init: bool,
}

impl Parser {
    /// Creates a parser for the source code in `string`.
    pub fn new(string: String) -> Self {
        Self { lexer: Lexer::new(string), allow_brace_init: true }
    }

    /// Parses the whole file and returns only its import statements.
    ///
    /// Used for building the [`ModuleGraph`] before the modules are parsed for
    /// compilation with [`Parser::parse_file`].
    ///
    /// # Panics
    ///
    /// Panics if the file contains a syntax error.
    pub fn parse_imports(&mut self, ) -> Vec<StmtKind> {
        let mut ast = vec![];
        while let Some(stmt) = self.parse_statement(){
            if matches!(stmt.kind, StmtKind::Import(_)) {
                ast.push(stmt.kind);
            }
        }
        ast
    }

    /// Builds the abstract syntax tree by parsing the whole file.
    ///
    /// Skips import statements, since the [`ModuleGraph`] already collected them
    /// through [`Parser::parse_imports`].
    ///
    /// # Panics
    ///
    /// Panics if the file contains a syntax error.
    pub fn parse_file(&mut self) -> Vec<Stmt> {
        let mut ast = vec![];
        while let Some(stmt) = self.parse_statement(){
            if !matches!(stmt.kind, StmtKind::Import(_)) {
                ast.push(stmt);
            }
        }
        ast
    }

    /// Consumes the next token and checks that it has the same variant as `token`.
    ///
    /// Only the variant is compared, so `Token::Identifier(String::new())` matches
    /// any identifier. `context` describes where the token was expected and ends
    /// up in the error message.
    ///
    /// # Panics
    ///
    /// Panics if lexing fails or the token doesn't match `token`.
    fn expect(&mut self, token: Token, context: &'static str) -> (Token, Span) {
        let (got, span) = self.next_token();
        if discriminant(&got) == discriminant(&token) {
            return (got, span);
        }

        if matches!(got, Token::EOF) {
            ParseError::UnexpectedEOF { span, context }.throw(&self.lexer.source)
        } else {
            ParseError::UnexpectedToken { expected: token, got, span, context }.throw(&self.lexer.source)
        }
    }

    /// Consumes and returns the next token.
    ///
    /// # Panics
    ///
    /// Panics if lexing fails.
    fn next_token(&mut self) -> (Token, Span) {
        match self.lexer.get_next_token() {
            Ok(t) => t,
            Err(e) => ParseError::Lex(e).throw(&self.lexer.source),
        }
    }

    /// Returns the next token without consuming it.
    ///
    /// # Panics
    ///
    /// Panics if lexing fails.
    fn peek(&mut self) -> (Token, Span) {
        match self.lexer.peek() {
            Ok(t) => t,
            Err(e) => ParseError::Lex(e).throw(&self.lexer.source),
        }
    }

    /// Consumes the next token without returning it.
    ///
    /// # Panics
    ///
    /// Panics if lexing fails.
    fn consume(&mut self) {
        self.next_token();
    }

    /// Returns the next token without consuming it, treating EOF as an error.
    ///
    /// Use this inside constructs that must be closed, like bodies and lists.
    /// `context` ends up in the error message.
    ///
    /// # Panics
    ///
    /// Panics if lexing fails or the next token is [`Token::EOF`].
    fn peek_or_eof(&mut self, context: &'static str) -> (Token, Span) {
        match self.peek() {
            (Token::EOF, span) => ParseError::UnexpectedEOF { span, context }.throw(&self.lexer.source),
            t => t,
        }
    }

    // --- Statement Parsing ---

    /// Parses the next statement.
    ///
    /// Returns `None` once the end of the file is reached.
    ///
    /// # Panics
    ///
    /// Panics if the statement contains a syntax error.
    pub fn parse_statement(&mut self) -> Option<Stmt> {
        let (cur, span) = match self.next_token() {
            (Token::EOF, _) => return None,
            t => t,
        };

        let kind = match cur {
            Token::Pub    => self.parse_pub(),
            Token::Func => StmtKind::Declaration(Declaration::new(
                self.parse_function_declaration(None, Visibility::Private),
                span.clone(),
                Visibility::Private
            )),
            Token::Class => StmtKind::Declaration(Declaration::new(
                self.parse_class_declaration(),
                span.clone(),
                Visibility::Private
            )),
            Token::Enum => StmtKind::Declaration(Declaration::new(
                self.parse_enum_declaration(), 
                span.clone(), 
                Visibility::Private
            )),
            Token::Let    => self.parse_variable(false),
            Token::Mut    => self.parse_variable(true),
            Token::Match  => self.parse_match(),
            Token::Return => self.parse_return(),
            Token::Import => self.parse_import(),
            other         => self.parse_expr_stmt(other, span.clone()),
        };
        Some(Stmt::new(kind, span, Type::Unknown))
    }

    /// Parses a public declaration. `pub` is already consumed.
    ///
    /// ```text
    /// pub func my_func() { ... }
    /// pub class World { ... }
    /// pub enum Color { ... }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `pub` isn't followed by `func`, `class` or `enum`.
    fn parse_pub(&mut self) -> StmtKind {
        let (decl, span) = self.next_token();

        match decl {
            Token::Func => StmtKind::Declaration(Declaration::new(
                self.parse_function_declaration(None, Visibility::Public),
                span.clone(),
                Visibility::Public
            )),
            Token::Class => StmtKind::Declaration(Declaration::new(
                self.parse_class_declaration(),
                span.clone(),
                Visibility::Public
            )),
            Token::Enum => StmtKind::Declaration(Declaration::new(
                self.parse_enum_declaration(), 
                span.clone(), 
                Visibility::Public
            )),
            other => 
                ParseError::ExpectedDeclaration { 
                    got: other, span, context: "after pub" 
                }.throw(&self.lexer.source)
        }
    }

    /// Parses a match statement. `match` is already consumed.
    ///
    /// ```text
    /// match x { pattern => expr, pattern => { ... } }
    /// ```
    ///
    /// Disables brace initialization while parsing the scrutinee, because
    /// otherwise the `{` in `match x { ... }` would be parsed as initializing `x`.
    ///
    /// # Panics
    ///
    /// Panics if the match contains a syntax error or the body isn't closed.
    fn parse_match(&mut self) -> StmtKind {
        let prev_brace = self.allow_brace_init;
        self.allow_brace_init = false;

        // scrutinee is the object that is being matched on
        let scrutinee = self.next_expression();
        self.allow_brace_init = prev_brace;

        self.expect(Token::LBrace, "at start of match body");

        let mut arms: Vec<MatchArm> = Vec::new();
        while !matches!(self.peek_or_eof("in match body"), (Token::RBrace, _)) {
            arms.push(self.parse_match_arm());
            if matches!(self.peek_or_eof("after match arm"), (Token::Comma, _)) {
                self.consume();
            }
        }
        self.expect(Token::RBrace, "at end of match body");
        StmtKind::Match { scrutinee, arms }
    }
    
    /// Parses a single match arm, `pattern => body`.
    ///
    /// The body is either a block `{ ... }` or a single expression, which gets
    /// wrapped into a one-statement body.
    ///
    /// # Panics
    ///
    /// Panics if the arm contains a syntax error.
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

    /// Parses the pattern of a match arm.
    ///
    /// Supported patterns are the wildcard `_`, enum variants like `Color::Red`
    /// or `Shape::Circle(c)` with an optional binding, identifiers and
    /// bool, int and string literals.
    ///
    /// # Panics
    ///
    /// Panics if the next token can't start a pattern.
    fn parse_pattern(&mut self) -> (Pattern, Span) {
        let (tok, span) = self.next_token();

        match tok {
            Token::Identifier(name) if name == "_" => (Pattern::Wildcard, span),
            Token::Identifier(name) => {
                if matches!(self.peek().0, Token::ColonColon) {
                    self.consume();
                    let (variant_tok, _) = self.expect(
                        Token::Identifier("variant".to_string()),
                        "after '::' in pattern",
                    );
                    let variant = variant_tok.to_string();
                    let binding = if matches!(self.peek().0, Token::LParen) {
                        self.consume();
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
            Token::IntLiteral(v) => (Pattern::Literal(Expr::new(ExprKind::IntLiteral(v), span.clone())), span),
            Token::StringLiteral(s) => (Pattern::Literal(Expr::new(ExprKind::StringLiteral(s), span.clone())), span),
            // Negative int literal, like `-2`
            Token::Minus => match self.next_token() {
                (Token::IntLiteral(v), _) => (Pattern::Literal(Expr::new(ExprKind::IntLiteral(-v), span.clone())), span),
                (other, other_span) => panic!("\nerror: '-{}' is not a valid pattern\n  --> {}:{}\n",
                    other, other_span.line + 1, other_span.col + 1),
            },
            other => panic!("\nerror: '{}' is not a valid pattern\n  --> {}:{}\n",
                other, span.line + 1, span.col + 1),
        }
    }

    // --- Declaration Parsing ---

    /// Parses a function declaration. `func` is already consumed.
    ///
    /// ```text
    /// func name(params) -> ReturnType { ... }
    /// ```
    ///
    /// `parent` is the name of the class when parsing a method.
    ///
    /// # Panics
    ///
    /// Panics if the declaration contains a syntax error.
    fn parse_function_declaration(&mut self, parent: Option<String>, visibility: Visibility) -> DeclarationKind {
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
            visibility,
        }
    }

    /// Parses an optional `-> Type` return type.
    ///
    /// Returns `None` if the next token isn't `->`.
    ///
    /// # Panics
    ///
    /// Panics if `->` isn't followed by a type.
    fn parse_return_type(&mut self) -> Option<Type> {
        if !matches!(self.peek().0, Token::Arrow) {
            return None;
        }
        self.consume(); // consume '->'
        let ty = self.parse_type();
        Some(ty.0)
    }

    /// Consumes the next token as an identifier expression.
    ///
    /// # Panics
    ///
    /// Panics if the next token isn't an identifier.
    fn next_identifier(&mut self, context: &'static str) -> Expr {
        let (cur, span) = self.next_token();

        match cur {
            Token::Identifier(name) => Expr::new(ExprKind::Identifier(name), span),
            t => ParseError::UnexpectedToken { expected: Token::Identifier("".to_string()), got: t, span, context }.throw(&self.lexer.source)
        }
    }

    /// Parses a block `{ ... }` of statements.
    ///
    /// # Panics
    ///
    /// Panics if a statement contains a syntax error or the block isn't closed.
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

    /// Parses a return statement `return expr;`. `return` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if the value is missing or isn't followed by `;`.
    fn parse_return(&mut self) -> StmtKind {
        let expr = self.next_expression();
        self.expect(Token::Semicolon, "after return value");
        StmtKind::Return(expr)
    }

    /// Parses an import statement. `import` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if the module path is invalid or isn't followed by `;`.
    fn parse_import(&mut self) -> StmtKind {
        let module = self.parse_module_import();
        self.expect(Token::Semicolon, "after import");
        StmtKind::Import(module)
    }

    /// Parses the module path of an import.
    ///
    /// ```text
    /// math
    /// math::vector
    /// math::{add, sub}
    /// ```
    ///
    /// An item list `{ ... }` ends the path.
    ///
    /// # Panics
    ///
    /// Panics if the path contains anything other than identifiers, `::` and
    /// a trailing item list.
    fn parse_module_import(&mut self) -> Expr {
        let (first_tok, first_span) = self.expect(
            Token::Identifier(String::new()),
            "at start of module path",
        );
        let first_name = match first_tok {
            Token::Identifier(n) => n,
            _ => unreachable!(),
        };

        let mut module_path: Vec<Expr> =
            vec![Expr::new(ExprKind::Identifier(first_name.clone()), first_span.clone())];
        let mut objects: Option<Vec<Expr>> = None;
        let mut name = first_name;

        while matches!(self.peek().0, Token::ColonColon) {
            self.consume(); // '::'

            if matches!(self.peek_or_eof("after '::' in module path"), (Token::LBrace, _)) {
                self.consume(); // '{'
                let mut list: Vec<Expr> = Vec::new();
                while !matches!(self.peek_or_eof("in import list"), (Token::RBrace, _)) {
                    let (tok, span) = self.expect(
                        Token::Identifier(String::new()),
                        "in import list",
                    );
                    let name = match tok {
                        Token::Identifier(n) => n,
                        _ => unreachable!(),
                    };
                    list.push(Expr::new(ExprKind::Identifier(name), span));
                    if !matches!(self.peek_or_eof("in import list"), (Token::RBrace, _)) {
                        self.expect(Token::Comma, "between imported items");
                    }
                }
                self.expect(Token::RBrace, "at end of import list");
                objects = Some(list);
                break;
            }

            let (tok, span) = self.expect(
                Token::Identifier(String::new()),
                "after '::' in module path",
            );
            name = tok.to_string().clone();
            module_path.push(Expr::new(ExprKind::Identifier(name.clone()), span));
        }

        Expr::new(
            ExprKind::ModuleImport { name: name, module: module_path, objects },
            first_span,
        )
    }

    /// Parses a class declaration `class Name { ... }`. `class` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if the declaration contains a syntax error.
    fn parse_class_declaration(&mut self) -> DeclarationKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in class declaration");
        let name = name.to_string();
        let (fields, methods) = self.parse_class_body(name.clone());

        DeclarationKind::Class { name, fields, methods }
    }

    /// Parses a class body `{ ... }` into its fields and methods.
    ///
    /// # Panics
    ///
    /// Panics if the body contains a syntax error or isn't closed.
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

    /// Parses one field, constructor or method of a class body, optionally
    /// prefixed with `pub`, and pushes it to `fields` or `methods`.
    ///
    /// # Panics
    ///
    /// Panics if the item contains a syntax error, or if a constructor is
    /// marked `pub` (constructors are always public).
    fn parse_class_body_stmt(&mut self, class_name: &String, fields: &mut Vec<Field>, methods: &mut Vec<Declaration>) {
        let mut visibility = Visibility::Private;
        if matches!(self.peek().0, Token::Pub) {
            self.consume();
            visibility = Visibility::Public;
        }

        let (next, next_span) = self.peek_or_eof("while parsing class body");
        match next {
            Token::Identifier(_) => {
                let field = self.parse_field(visibility);
                fields.push(field);
                self.expect(Token::Semicolon, "after class field");
            }
            Token::Constructor => {
                if matches!(visibility, Visibility::Public) {
                    panic!("Constructors are always public — `pub` is redundant");
                }
                self.consume(); // consume 'Constructor'
                let decl_kind = self.parse_constructor(&class_name);
                methods.push(Declaration::new(decl_kind, next_span, Visibility::Private));
            }
            Token::Func => {
                self.consume(); // consume 'func'
                let decl_kind = self.parse_function_declaration(Some(class_name.clone()), visibility);
                methods.push(Declaration::new(decl_kind, next_span, Visibility::Private));
            }
            tok => ParseError::UnexpectedToken {
                expected: Token::Identifier("field or method".to_string()),
                got: tok,
                span: next_span,
                context: "in class body",
            }.throw(&self.lexer.source),
        }
    }

    /// Parses a class field `name: Type` with an optional `= default` value.
    ///
    /// The trailing `;` is left for the caller.
    ///
    /// # Panics
    ///
    /// Panics if the name isn't an identifier or the type is missing.
    fn parse_field(&mut self, visibility: Visibility) -> Field {
        let first_param = self.next_expression();
        let name = match first_param.kind {
            ExprKind::Identifier(name) => name.to_string(),
            _ => ParseError::ParamTypeNotAllowed { span: first_param.span }.throw(&self.lexer.source)
        };

        self.expect(Token::Colon, "after parameter name");
        let (ty, ty_span) = self.parse_type();

        let mut default_value = None;
        if matches!(self.peek().0, Token::Assign) {
            self.consume(); // consume '='
            default_value = Some(Box::new(self.next_expression()));
        }

        Field {
            name: name.to_string(),
            ty,
            ty_span,
            value: default_value,
            visibility: visibility
        }
    }

    /// Parses a type, like `int32`, `&String` or a class name.
    ///
    /// Identifiers become [`Type::Class`], `&` wraps the inner type in [`Type::Ref`].
    ///
    /// # Panics
    ///
    /// Panics if the next token isn't a type.
    fn parse_type(&mut self) -> (Type, Span) {
        let (tok, span) = self.peek_or_eof("as parameter type");
        match tok {
            Token::BAnd => {
                self.consume(); // '&'
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
                self.consume();
                (tok.to_type(), span)
            }
        }
    }

    /// Parses a constructor of class `type_name`. `Constructor` is already consumed.
    ///
    /// ```text
    /// Constructor(params) { ... }
    /// Constructor(params);
    /// ```
    ///
    /// Without a body the constructor ends with `;`.
    ///
    /// # Panics
    ///
    /// Panics if the constructor contains a syntax error.
    fn parse_constructor(&mut self, type_name: &String) -> DeclarationKind {
        let params = self.parse_params(Some(type_name));
        let return_type = self.parse_return_type();
        let body = if matches!(self.peek().0, Token::LBrace) {
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
            visibility: Visibility::Public
        }
    }

    /// Parses a comma separated parameter list `( ... )`.
    ///
    /// `parent` is the name of the class when parsing a method, which gives
    /// `self` its type.
    ///
    /// # Panics
    ///
    /// Panics if a parameter contains a syntax error or the list isn't closed.
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

    /// Parses a single parameter `name: Type` with an optional `= default` value.
    ///
    /// Accepts a leading `mut` and `&`. A parameter named `self` takes no type
    /// annotation and gets the type of the `parent` class.
    ///
    /// # Panics
    ///
    /// Panics if the name isn't an identifier, the type is missing, or `self`
    /// is used outside of a class.
    fn parse_param(&mut self, parent: Option<&str>) -> Param {
        let mut mutable = false;
        if matches!(self.peek().0, Token::Mut) {
            self.consume();
            mutable = true;
        }

        if matches!(self.peek().0, Token::BAnd) {
            self.consume();
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
        if matches!(self.peek().0, Token::Assign) {
            self.consume(); // consume '='
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


    /// Parses an enum declaration `enum Name { ... }`. `enum` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if the declaration contains a syntax error.
    fn parse_enum_declaration(&mut self) -> DeclarationKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in class declaration");
        let name = name.to_string();
        let fields = self.parse_enum_body();

        DeclarationKind::Enum{ name, fields }
    }

    /// Parses an enum body of `Variant => Type` entries separated by commas.
    ///
    /// Returns each variant's name, index, payload type and the type's span.
    /// Indices are assigned in declaration order, starting at 0.
    ///
    /// # Panics
    ///
    /// Panics if a variant contains a syntax error or the body isn't closed.
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
                self.consume();
            }
        }
        self.expect(Token::RBrace, "enum body");

        fields
    }

    // --- Variable Parsing ---

    /// Parses a variable declaration. `let` or `mut` is already consumed.
    ///
    /// ```text
    /// let name: Type = value;
    /// mut name = value;
    /// let name: Class = { field: value };
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the declaration contains a syntax error, or if a brace
    /// initializer is used without a type annotation.
    fn parse_variable(&mut self, mutable: bool) -> StmtKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in variable declaration");
        let typedef = if matches!(self.peek_or_eof("at instantiation"), (Token::Colon, _)) {
            self.consume();
            let typedef = self.parse_type();
            Some(typedef.0)
        } else { None };

        self.expect(Token::Assign, "after type in variable declaration");

        let value = match self.peek() {
            (Token::LBrace, span) => {
                self.consume(); // consume '{'
                let class = typedef.as_ref().and_then(Type::class_name).expect("type undefined");
                let kind = self.parse_brace_inst(class);
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

    /// Parses the fields of a brace initializer `{ field: value, ... }` for `class`.
    ///
    /// The opening `{` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if a field contains a syntax error or the initializer isn't closed.
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
        self.consume(); // consume '}'

        ExprKind::StructInit { class, fields }
    }

    /// Parses an expression statement `expr;`, starting with the already consumed
    /// `cur_token`.
    ///
    /// # Panics
    ///
    /// Panics if the expression is invalid or isn't followed by `;`.
    fn parse_expr_stmt(&mut self, cur_token: Token, span: Span) -> StmtKind {
        let expr = self.parse_expression(cur_token, span);
        self.expect(Token::Semicolon, "after expression statement");
        StmtKind::Expression(expr)
    }

    // --- Expression Parsing ---

    /// Parses an expression starting with the already consumed `cur` token.
    ///
    /// `*` and `/` bind tighter than `+` and `-`, and operators of the same
    /// precedence are left associative, so `a - b * c - d` parses as
    /// `(a - (b * c)) - d`.
    ///
    /// # Panics
    ///
    /// Panics if `cur` can't start an expression.
    fn parse_expression(&mut self, cur: Token, span: Span) -> Expr {
        let lhs = self.parse_primary(cur, span);
        self.parse_binary(lhs, 0)
    }

    /// Parses the binary operators following the already parsed `lhs`, using
    /// precedence climbing.
    ///
    /// Only operators with a precedence of at least `min_precedence` are
    /// consumed, the rest is left for the caller.
    ///
    /// # Panics
    ///
    /// Panics if an operator isn't followed by an operand.
    fn parse_binary(&mut self, mut lhs: Expr, min_precedence: u8) -> Expr {
        while let Some(precedence) = binary_precedence(&self.peek().0) {
            if precedence < min_precedence {
                break;
            }
            let (op, _) = self.next_token();
            let (cur, rhs_span) = self.next_token();
            let mut rhs = self.parse_primary(cur, rhs_span);

            // Operators that bind tighter than `op` belong to its right side
            while let Some(next_precedence) = binary_precedence(&self.peek().0) {
                if next_precedence <= precedence {
                    break;
                }
                rhs = self.parse_binary(rhs, next_precedence);
            }

            let span = lhs.span.clone();
            lhs = Expr::new(ExprKind::BinaryExpr { lhs: Box::new(lhs), op, rhs: Box::new(rhs) }, span);
        }
        lhs
    }

    /// Parses a single operand starting with the already consumed `cur` token,
    /// without any binary operator following it.
    ///
    /// # Panics
    ///
    /// Panics if `cur` can't start an expression.
    fn parse_primary(&mut self, cur: Token, span: Span) -> Expr {
        let kind = match cur {
            Token::Minus            => return self.parse_negation(span),
            Token::False            => ExprKind::Boolean(false),
            Token::Print            => self.parse_print(),
            Token::IntLiteral(value)   => ExprKind::IntLiteral(value),
            Token::FloatLiteral(value) => ExprKind::FloatLiteral(value),
            Token::Identifier(name) => self.parse_identifier(name, None, span.clone()),
            Token::True             => ExprKind::Boolean(true),
            Token::BAnd             => self.parse_refrence(),
            Token::SelfRef          => self.parse_identifier("self".to_string(), None, span.clone()),
            Token::StringLiteral(literal) => ExprKind::StringLiteral(literal),
            other => panic!("\nerror: '{}' is not a valid expression\n  --> {}:{}\n",
                other, span.line + 1, span.col + 1),
        };
        Expr::new(kind, span)
    }

    /// Consumes the next token and parses the expression it starts.
    ///
    /// # Panics
    ///
    /// Panics if the next token can't start an expression.
    fn next_expression(&mut self) -> Expr {
        let (cur, span) = self.next_token();
        self.parse_expression(cur, span)
    }

    /// Parses a negation `-operand`. `-` is already consumed, `span` is its position.
    ///
    /// Only the operand right after `-` is negated, so `-x + 1` is `(-x) + 1`.
    ///
    /// # Panics
    ///
    /// Panics if `-` isn't followed by an expression.
    fn parse_negation(&mut self, span: Span) -> Expr {
        let (cur, operand_span) = self.next_token();
        let operand = self.parse_primary(cur, operand_span);
        let mut negated = -operand;
        negated.span = span;
        negated
    }

    /// Parses a print call `print(expr)`. `print` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if the argument isn't wrapped in parentheses.
    fn parse_print(&mut self) -> ExprKind {
        self.expect(Token::LParen, "after 'print'");
        let value = self.next_expression();
        self.expect(Token::RParen, "after print argument");
        ExprKind::Print(Box::new(value))
    }

    /// Parses whatever follows an already consumed identifier.
    ///
    /// Depending on the next token this becomes a `::` access, a brace
    /// initializer, a function call, a field access, an assignment or just the
    /// identifier itself.
    ///
    /// # Panics
    ///
    /// Panics if the following construct contains a syntax error.
    fn parse_identifier(&mut self, identifier: String, prev: Option<String>, span: Span) -> ExprKind {
        match self.peek().0 {
            Token::ColonColon => self.parse_colon_access(identifier),
            Token::LBrace if self.allow_brace_init => {
                self.consume();
                return self.parse_brace_inst(identifier);
            }
            Token::LParen => self.parse_function_call(identifier, prev),
            Token::Dot    => self.parse_field_access(identifier),
            Token::Assign => self.parse_assignment(identifier, span),
            _             => ExprKind::Identifier(identifier),
        }
    }

    /// Parses a `::` access on `name`.
    ///
    /// ```text
    /// Color::Red
    /// Shape::Circle(value)
    /// Shape::Rect { w: 1, h: 2 }
    /// ```
    ///
    /// With a payload this becomes an [`ExprKind::EnumInit`], otherwise an
    /// [`ExprKind::ColonAccess`].
    ///
    /// # Panics
    ///
    /// Panics if `::` isn't followed by an identifier or the payload is invalid.
    fn parse_colon_access(&mut self, name: String) -> ExprKind {
        self.expect(Token::ColonColon, "in colon access");
        let (tag, _span) = self.expect(Token::Identifier("tag".to_string()), "at tag");

        if matches!(self.peek_or_eof("enum tag construction"), (Token::LParen, _)){
            self.consume();
            let expr = self.next_expression();
            self.expect(Token::RParen, "end of enum const");
            return ExprKind::EnumInit { name, tag: tag.to_string(), value: Box::new(expr)};
        }

        if let (Token::LBrace, brace_span) = self.peek_or_eof("enum tag construction") {
            self.consume(); // consume '{'
            // Placeholder class name — sema replaces it with the variant's payload type.
            let inner_kind = self.parse_brace_inst(name.clone());
            let inner_expr = Expr::new(inner_kind, brace_span);
            return ExprKind::EnumInit { name, tag: tag.to_string(), value: Box::new(inner_expr) };
        }

        ExprKind::ColonAccess { name, tag: tag.to_string() }
    }

    /// Parses the argument list of a call to `name`, starting at `(`.
    ///
    /// `parent_name` is the receiver for method calls like `obj.method()`.
    ///
    /// # Panics
    ///
    /// Panics if an argument is invalid or the list isn't closed.
    fn parse_function_call(&mut self, name: String, parent_name: Option<String>) -> ExprKind {
        self.consume(); // consume '('

        let mut params: Vec<Expr> = Vec::new();
        while !matches!(self.peek_or_eof("while parsing function call arguments"), (Token::RParen, _)) {
            params.push(self.next_expression());
            if !matches!(self.peek_or_eof("after function call argument"), (Token::RParen, _)) {
                self.expect(Token::Comma, "between function call arguments");
            }
        }
        self.consume(); // consume ')'
        ExprKind::FunctionCall { name, params, parent_name, mangled: None }
    }

    /// Parses a `.` access on `name`, starting at `.`.
    ///
    /// Becomes a method call for `name.method(...)`, an assignment for
    /// `name.field = value` and a field access otherwise.
    ///
    /// # Panics
    ///
    /// Panics if `.` isn't followed by an identifier.
    fn parse_field_access(&mut self, name: String) -> ExprKind {
        let (_, dot_span) = self.expect(Token::Dot, "in field access");
        let (tok, field_span) = self.expect(Token::Identifier("field name".to_string()), "after '.'");
        let field_name = match tok {
            Token::Identifier(n) => n,
            _ => unreachable!(),
        };

        if matches!(self.peek().0, Token::LParen) {
            return self.parse_function_call(field_name, Some(name));
        }

        let field_expr = Expr::new(ExprKind::Identifier(field_name), field_span);
        let fa_kind = ExprKind::FieldAccess { name, field: Box::new(field_expr) };

        if matches!(self.peek().0, Token::Assign) {
            self.consume(); // '='
            let lhs = Expr::new(fa_kind, dot_span);
            let rhs = self.next_expression();
            return ExprKind::Assignment {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }

        fa_kind
    }

    /// Parses an assignment `name = value`, starting at `=`.
    ///
    /// # Panics
    ///
    /// Panics if the value is invalid.
    fn parse_assignment(&mut self, name: String, span: Span) -> ExprKind {
        self.consume(); // consume '='
        let lhs = Expr::new(ExprKind::Identifier(name), span);
        let rhs = self.next_expression();
        ExprKind::Assignment {
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    /// Parses a reference `&expr`. `&` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if the referenced expression is invalid.
    fn parse_refrence(&mut self) -> ExprKind {
        let inner = self.next_expression();
        ExprKind::Ref(Box::new(inner))
    }
}

/// Precedence of `token` as a binary operator, higher binds tighter.
/// `None` if it isn't a binary operator.
fn binary_precedence(token: &Token) -> Option<u8> {
    match token {
        Token::Plus | Token::Minus       => Some(1),
        Token::Multiply | Token::Divide  => Some(2),
        _ => None,
    }
}
