use std::mem::discriminant;

use crate::{declaration::{Declaration, DeclarationKind, TypeParam, Visibility}, diagnostic, expression::{Expr, ExprKind}, field::Field, lexer::{LexError, LexErrorKind, Lexer}, param::Param, span::Span, statement::{MatchArm, Pattern, Stmt, StmtKind}, token::Token, r#type::Type};

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
    /// Stops compiling with the message for this error, see [`diagnostic::throw`].
    ///
    /// `source` is the parsed source code, used to print the offending line.
    fn throw(self, source: &str) -> ! {
        let (message, span) = match self {
            ParseError::ParamTypeMissing { span, context } =>
                (format!("parameter needs a clear type declaration {}", context), span),
            ParseError::ParamTypeNotAllowed { span } =>
                ("parameter name can only be an Identifier or self".to_string(), span),
            ParseError::UnexpectedEOF { span, context } =>
                (format!("unexpected end of file {}", context), span),
            ParseError::UnexpectedToken { expected, got, span, context } =>
                (format!("expected '{}' {}, got '{}'", expected, context, got), span),
            ParseError::ExpectedDeclaration { got, span, context } =>
                (format!("expected declaration {}, got '{}'", context, got), span),
            ParseError::Lex(LexError { kind, span }) => {
                let message = match kind {
                    LexErrorKind::UnexpectedChar(c)      => format!("unexpected character '{}'", c),
                    LexErrorKind::UnterminatedString     => "unterminated string literal".to_string(),
                    LexErrorKind::NumberOutOfRange(num)  => format!("number '{}' doesn't fit into int64", num),
                };
                (message, span)
            }
        };
        diagnostic::throw(message, Some(span), source)
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
    /// Type parameters of the generic function being parsed. Type names
    /// matching one of them become [`Type::Param`].
    type_params: Vec<String>,
    /// Hidden type parameters created by `impl Trait` parameter types. Only
    /// `Some` while a function's parameter list is parsed, the only place
    /// `impl Trait` is allowed.
    impl_params: Option<Vec<TypeParam>>,
}

impl Parser {
    /// Creates a parser for the source code in `string`.
    pub fn new(string: String) -> Self {
        Self { lexer: Lexer::new(string), allow_brace_init: true, type_params: vec![], impl_params: None }
    }

    /// Builds the abstract syntax tree by parsing the whole file.
    ///
    /// Skips import statements, since the [`ModuleGraph`](crate::module::ModuleGraph)
    /// already collected them.
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
            Token::Trait => StmtKind::Declaration(Declaration::new(
                self.parse_trait_declaration(),
                span.clone(),
                Visibility::Private
            )),
            // An impl is as visible as its trait, so it has no visibility of its own
            Token::Impl => StmtKind::Declaration(Declaration::new(
                self.parse_impl_declaration(),
                span.clone(),
                Visibility::Public
            )),
            Token::Let    => self.parse_variable(false),
            Token::Mut    => self.parse_variable(true),
            Token::Match  => self.parse_match(),
            Token::If     => self.parse_if(),
            Token::While  => self.parse_while(),
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
            Token::Trait => StmtKind::Declaration(Declaration::new(
                self.parse_trait_declaration(),
                span.clone(), 
                Visibility::Public
            )),
            other => 
                ParseError::ExpectedDeclaration { 
                    got: other, span, context: "after pub" 
                }.throw(&self.lexer.source)
        }
    }

    /// Parses an if statement. `if` is already consumed.
    ///
    /// ```text
    /// if condition { ... }
    /// if condition { ... } else { ... }
    /// if condition { ... } else if other { ... } else { ... }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the statement contains a syntax error.
    fn parse_if(&mut self) -> StmtKind {
        let condition = self.parse_condition();
        let then_body = self.parse_body();

        let else_body = if matches!(self.peek().0, Token::Else) {
            self.consume(); // 'else'
            if matches!(self.peek().0, Token::If) {
                // `else if` is an if statement inside the else branch
                let (_, if_span) = self.next_token();
                let nested = self.parse_if();
                Some(vec![Stmt::new(nested, if_span, Type::Unknown)])
            } else {
                Some(self.parse_body())
            }
        } else {
            None
        };

        StmtKind::If { condition, then_body, else_body }
    }

    /// Parses a while loop. `while` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if the loop contains a syntax error.
    fn parse_while(&mut self) -> StmtKind {
        let condition = self.parse_condition();
        let body = self.parse_body();
        StmtKind::While { condition, body }
    }

    /// Parses the condition of an `if` or `while`.
    ///
    /// Disables brace initialization, otherwise the `{` in `if x { ... }` would
    /// be parsed as initializing `x`.
    ///
    /// # Panics
    ///
    /// Panics if the condition isn't a valid expression.
    fn parse_condition(&mut self) -> Expr {
        let prev_brace = self.allow_brace_init;
        self.allow_brace_init = false;
        let condition = self.next_expression();
        self.allow_brace_init = prev_brace;
        condition
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
                (other, other_span) =>
                    diagnostic::throw(format!("'-{}' is not a valid pattern", other), Some(other_span), &self.lexer.source),
            },
            other => diagnostic::throw(format!("'{}' is not a valid pattern", other), Some(span), &self.lexer.source),
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
        let mut type_params = self.parse_optional_type_params();

        // The type parameters are only known inside this function, together
        // with those of a generic class around it
        let outer_type_params = self.enter_type_params(&type_params);

        self.impl_params = Some(Vec::new());
        let params = self.parse_params(parent.as_deref());
        type_params.extend(self.impl_params.take().unwrap_or_default());

        let return_type = self.parse_return_type();
        let body = self.parse_body();
        self.type_params = outer_type_params;

        DeclarationKind::Function {
            name: name.to_string(),
            type_params,
            params,
            body: Some(body),
            return_type,
            parent,
            mangled: None,
            visibility,
        }
    }

    /// Parses the type parameters `<T: Show + Clone, U>` of a generic function, starting at `<`.
    ///
    /// # Panics
    ///
    /// Panics if the list contains anything but names with optional trait bounds.
    fn parse_type_params(&mut self) -> Vec<TypeParam> {
        self.expect(Token::Lt, "at start of type parameter list");

        let mut params = Vec::new();
        while !matches!(self.peek_or_eof("in type parameter list"), (Token::Gt, _)) {
            let (name, _) = self.expect(Token::Identifier(String::new()), "as type parameter name");
            let bounds = if matches!(self.peek().0, Token::Colon) {
                self.consume(); // ':'
                self.parse_trait_bounds()
            } else {
                vec![]
            };
            params.push(TypeParam { name: name.to_string(), bounds });

            if !matches!(self.peek_or_eof("in type parameter list"), (Token::Gt, _)) {
                self.expect(Token::Comma, "between type parameters");
            }
        }
        self.expect(Token::Gt, "at end of type parameter list");
        params
    }

    /// Parses trait names joined with `+`, like `Show + Clone`.
    ///
    /// # Panics
    ///
    /// Panics if a bound isn't a name.
    fn parse_trait_bounds(&mut self) -> Vec<String> {
        let mut bounds = Vec::new();
        loop {
            let (bound, _) = self.expect(Token::Identifier(String::new()), "as trait bound");
            bounds.push(bound.to_string());
            if !matches!(self.peek().0, Token::Plus) {
                return bounds;
            }
            self.consume(); // '+'
        }
    }

    /// Parses a trait declaration `trait Name { ... }`. `trait` is already consumed.
    ///
    /// ```text
    /// trait Shape {
    ///     func area(&self) -> float64;
    ///     pub func describe(&self) { ... }
    /// }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the declaration contains a syntax error or the body isn't closed.
    fn parse_trait_declaration(&mut self) -> DeclarationKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in trait declaration");
        let name = name.to_string();
        self.expect(Token::LBrace, "at start of trait body");

        let mut methods = vec![];
        while !matches!(self.peek_or_eof("in trait body"), (Token::RBrace, _)) {
            let mut visibility = Visibility::Private;
            if matches!(self.peek().0, Token::Pub) {
                visibility = Visibility::Public;
                self.consume();
            }

            let (_, func_span) = self.expect(Token::Func, "in trait body");
            let method = self.parse_trait_method(&name, visibility.clone());
            methods.push(Declaration::new(method, func_span, visibility));
        }
        self.expect(Token::RBrace, "at end of trait body");

        DeclarationKind::Trait { name, methods }
    }

    /// Parses an impl block `impl Trait for Type { ... }`. `impl` is already consumed.
    ///
    /// ```text
    /// impl Shape for Circle {
    ///     func area(&self) -> float64 { ... }
    /// }
    /// ```
    ///
    /// Every method needs a body. Impl methods are always public, so they can't be marked `pub`.
    ///
    /// # Panics
    ///
    /// Panics if the impl contains a syntax error or the body isn't closed.
    fn parse_impl_declaration(&mut self) -> DeclarationKind {
        let (trait_name, _) = self.expect(Token::Identifier(String::new()), "as trait name in impl");
        self.expect(Token::For, "after trait name in impl");
        let (target, _) = self.parse_type();
        // Sema gives `self` the real target type, this is only a placeholder
        let target_name = target.to_string();
        self.expect(Token::LBrace, "at start of impl body");

        let mut methods = vec![];
        while !matches!(self.peek_or_eof("in impl body"), (Token::RBrace, _)) {
            let (_, func_span) = self.expect(Token::Func, "in impl body");
            let method = self.parse_function_declaration(Some(target_name.clone()), Visibility::Public);
            methods.push(Declaration::new(method, func_span, Visibility::Public));
        }
        self.expect(Token::RBrace, "at end of impl body");

        DeclarationKind::Impl { trait_name: trait_name.to_string(), target, methods }
    }

    /// Parses a method of the trait `trait_name`. `func` is already consumed.
    ///
    /// Unlike other functions, a trait method may end with `;` instead of a body.
    ///
    /// # Panics
    ///
    /// Panics if the method contains a syntax error.
    fn parse_trait_method(&mut self, trait_name: &str, visibility: Visibility) -> DeclarationKind {
        let (name, _) = self.expect(Token::Identifier(String::new()), "in trait method");
        let params = self.parse_params(Some(trait_name));
        let return_type = self.parse_return_type();
        let body = if matches!(self.peek().0, Token::LBrace) {
            Some(self.parse_body())
        } else {
            self.expect(Token::Semicolon, "after trait method without body");
            None
        };

        DeclarationKind::Function {
            name: name.to_string(),
            type_params: vec![],
            params,
            body,
            return_type,
            parent: Some(trait_name.to_string()),
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
                None => {
                    let span = self.peek().1;
                    ParseError::UnexpectedEOF { span, context: "while parsing body" }.throw(&self.lexer.source)
                }
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
        let type_params = self.parse_optional_type_params();

        // The type parameters are known in the whole class, fields and methods
        let outer_type_params = self.enter_type_params(&type_params);
        let (fields, methods) = self.parse_class_body(name.clone());
        self.type_params = outer_type_params;

        DeclarationKind::Class { name, type_params, fields, methods }
    }

    /// Parses type parameters `<...>` if the next token is `<`, returns none otherwise.
    ///
    /// # Panics
    ///
    /// Panics if the list contains a syntax error.
    fn parse_optional_type_params(&mut self) -> Vec<TypeParam> {
        if matches!(self.peek().0, Token::Lt) {
            self.parse_type_params()
        } else {
            vec![]
        }
    }

    /// Adds `type_params` to the type parameters in scope, and returns the
    /// previous ones to restore when leaving the declaration.
    fn enter_type_params(&mut self, type_params: &[TypeParam]) -> Vec<String> {
        let mut names = self.type_params.clone();
        names.extend(type_params.iter().map(|p| p.name.clone()));
        std::mem::replace(&mut self.type_params, names)
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
                    diagnostic::throw("constructors are always public, `pub` is redundant", Some(next_span), &self.lexer.source);
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
                let ty = if self.type_params.contains(&name) {
                    Type::Param(name)
                } else if matches!(self.peek().0, Token::Lt) {
                    // A generic class or enum with type arguments, like `Box<int32>`
                    self.consume(); // '<'
                    let mut args = Vec::new();
                    while !matches!(self.peek_or_eof("in type argument list"), (Token::Gt, _)) {
                        args.push(self.parse_type().0);
                        if !matches!(self.peek_or_eof("in type argument list"), (Token::Gt, _)) {
                            self.expect(Token::Comma, "between type arguments");
                        }
                    }
                    self.expect(Token::Gt, "at end of type argument list");
                    Type::Generic { name, args }
                } else {
                    Type::Class(name)
                };
                (ty, ty_span)
            }
            // `impl Show` is a hidden type parameter `<T: Show>` of the function
            Token::Impl => {
                self.consume(); // 'impl'
                let bounds = self.parse_trait_bounds();
                let Some(impl_params) = self.impl_params.as_mut() else {
                    diagnostic::throw("'impl Trait' is only allowed as a parameter type", Some(span), &self.lexer.source)
                };

                // Every `impl Trait` parameter is its own type parameter, even with the same bounds
                let base = format!("impl {}", bounds.join(" + "));
                let mut name = base.clone();
                let mut n = 2;
                while impl_params.iter().any(|p| p.name == name) {
                    name = format!("{}#{}", base, n);
                    n += 1;
                }
                impl_params.push(TypeParam { name: name.clone(), bounds });
                (Type::Param(name), span)
            }
            tok => {
                self.consume();
                (tok.to_type(), span)
            }
        }
    }

    /// Tries to parse a type without consuming anything that isn't one.
    ///
    /// Returns `None` if the next token can't start a type. Unlike
    /// [`Parser::parse_type`] this never panics, so callers can back off.
    fn try_parse_type(&mut self) -> Option<Type> {
        match self.peek().0 {
            Token::BAnd => {
                self.consume(); // '&'
                Some(Type::Ref(Box::new(self.try_parse_type()?)))
            }
            Token::Identifier(name) => {
                self.consume();
                if self.type_params.contains(&name) {
                    return Some(Type::Param(name));
                }
                if !matches!(self.peek().0, Token::Lt) {
                    return Some(Type::Class(name));
                }

                // A generic class or enum with type arguments, like `Box<int32>`
                self.consume(); // '<'
                let mut args = Vec::new();
                loop {
                    args.push(self.try_parse_type()?);
                    match self.peek().0 {
                        Token::Comma => self.consume(),
                        Token::Gt => break,
                        _ => return None,
                    }
                }
                self.consume(); // '>'
                Some(Type::Generic { name, args })
            }
            tok if tok.is_type_keyword() => {
                self.consume();
                Some(tok.to_type())
            }
            _ => None,
        }
    }

    /// Tries to parse explicit type arguments `<int64, String>` after a name, starting at `<`.
    ///
    /// They only count as type arguments if everything between `<` and `>` is
    /// a type and the `>` is followed by `(` of a call, `{` of a brace
    /// initializer or `::` of an enum variant. Otherwise the lexer goes back to
    /// the `<`, so it can be read as something else, like a comparison.
    fn try_parse_type_args(&mut self) -> Option<Vec<Type>> {
        let checkpoint = self.lexer.checkpoint();
        let type_args = self.parse_type_args_or_none();
        if type_args.is_none() {
            self.lexer.rewind(checkpoint);
        }
        type_args
    }

    /// The part of [`Parser::try_parse_type_args`] that may give up halfway.
    fn parse_type_args_or_none(&mut self) -> Option<Vec<Type>> {
        self.consume(); // '<'
        let mut type_args = Vec::new();
        loop {
            type_args.push(self.try_parse_type()?);
            match self.peek().0 {
                Token::Comma => self.consume(),
                Token::Gt => break,
                _ => return None,
            }
        }
        self.consume(); // '>'

        let followed_by_use = match self.peek().0 {
            Token::LParen | Token::ColonColon => true,
            Token::LBrace => self.allow_brace_init,
            _ => false,
        };
        followed_by_use.then_some(type_args)
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
            type_params: vec![],
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
                .unwrap_or_else(|| diagnostic::throw("'self' parameter outside of a class method", Some(first_param.span.clone()), &self.lexer.source))
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
        let (name, _) = self.expect(Token::Identifier(String::new()), "in enum declaration");
        let name = name.to_string();
        let type_params = self.parse_optional_type_params();

        let outer_type_params = self.enter_type_params(&type_params);
        let fields = self.parse_enum_body();
        self.type_params = outer_type_params;

        DeclarationKind::Enum { name, type_params, fields }
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
                // `let b: Box<int32> = { ... }` passes the type arguments on
                let (class, type_args) = match &typedef {
                    Some(Type::Generic { name, args }) => (name.clone(), args.clone()),
                    other => {
                        let class = other.as_ref().and_then(Type::class_name).unwrap_or_else(|| diagnostic::throw(
                            "a brace initializer needs a class type annotation, like `let p: Point = { ... }`",
                            Some(span.clone()),
                            &self.lexer.source,
                        ));
                        (class, vec![])
                    }
                };
                let kind = self.parse_brace_inst(class, type_args);
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

    /// Parses the fields of a brace initializer `{ field: value, ... }` for `class`,
    /// with the explicit `type_args` of a generic class.
    ///
    /// The opening `{` is already consumed.
    ///
    /// # Panics
    ///
    /// Panics if a field contains a syntax error or the initializer isn't closed.
    fn parse_brace_inst(&mut self, class: String, type_args: Vec<Type>) -> ExprKind {
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

        ExprKind::StructInit { class, type_args, fields }
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
        let lhs = self.parse_operand(cur, span);
        self.parse_binary(lhs, 0)
    }

    /// Parses an operand of a binary operator starting with the already
    /// consumed `cur` token: a primary followed by any number of `as` conversions.
    ///
    /// `as` binds tighter than binary operators but looser than `-`, so
    /// `-x as uint8 * 2` parses as `((-x) as uint8) * 2`.
    ///
    /// # Panics
    ///
    /// Panics if `cur` can't start an expression.
    fn parse_operand(&mut self, cur: Token, span: Span) -> Expr {
        let mut expr = self.parse_primary(cur, span);
        while matches!(self.peek().0, Token::As) {
            expr = self.parse_conversion(expr);
        }
        expr
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
            let mut rhs = self.parse_operand(cur, rhs_span);

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
            // Parentheses only group, like `(a + b) * c`
            Token::LParen => {
                let prev_brace = self.allow_brace_init;
                self.allow_brace_init = true;
                let inner = self.next_expression();
                self.allow_brace_init = prev_brace;
                self.expect(Token::RParen, "after expression in parentheses");
                return self.parse_postfix(inner);
            }
            Token::False            => ExprKind::Boolean(false),
            Token::Print            => self.parse_print(),
            Token::IntLiteral(value)   => ExprKind::IntLiteral(value),
            Token::FloatLiteral(value) => ExprKind::FloatLiteral(value),
            Token::Identifier(name) => self.parse_identifier(name, span.clone()),
            Token::True             => ExprKind::Boolean(true),
            Token::BAnd             => self.parse_refrence(),
            Token::SelfRef          => self.parse_identifier("self".to_string(), span.clone()),
            Token::StringLiteral(literal) => ExprKind::StringLiteral(literal),
            other => diagnostic::throw(format!("'{}' is not a valid expression", other), Some(span), &self.lexer.source),
        };
        self.parse_postfix(Expr::new(kind, span))
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
    /// initializer, a function call, an assignment or just the identifier
    /// itself. Field accesses and method calls with `.` are parsed afterwards,
    /// see [`Parser::parse_postfix`].
    ///
    /// # Panics
    ///
    /// Panics if the following construct contains a syntax error.
    fn parse_identifier(&mut self, identifier: String, span: Span) -> ExprKind {
        // Explicit type arguments, like `max<int64>(3, 7)`, `Box<int32> { ... }` or `Option<int32>::Some(5)`
        if matches!(self.peek().0, Token::Lt) {
            if let Some(type_args) = self.try_parse_type_args() {
                return match self.peek().0 {
                    Token::LBrace => {
                        self.consume(); // '{'
                        self.parse_brace_inst(identifier, type_args)
                    }
                    Token::ColonColon => self.parse_colon_access(identifier, type_args),
                    _ => self.parse_function_call(identifier, None, type_args),
                };
            }
        }

        match self.peek().0 {
            Token::ColonColon => self.parse_colon_access(identifier, vec![]),
            Token::LBrace if self.allow_brace_init => {
                self.consume();
                return self.parse_brace_inst(identifier, vec![]);
            }
            Token::LParen => self.parse_function_call(identifier, None, vec![]),
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
    fn parse_colon_access(&mut self, name: String, type_args: Vec<Type>) -> ExprKind {
        let (_, colon_span) = self.expect(Token::ColonColon, "in colon access");
        let (tag, _span) = self.expect(Token::Identifier("tag".to_string()), "at tag");

        if matches!(self.peek_or_eof("enum tag construction"), (Token::LParen, _)){
            self.consume();
            let expr = self.next_expression();
            self.expect(Token::RParen, "end of enum const");
            return ExprKind::EnumInit { name, type_args, tag: tag.to_string(), value: Box::new(expr)};
        }

        if let (Token::LBrace, brace_span) = self.peek_or_eof("enum tag construction") {
            self.consume(); // consume '{'
            // Placeholder class name — sema replaces it with the variant's payload type.
            let inner_kind = self.parse_brace_inst(name.clone(), vec![]);
            let inner_expr = Expr::new(inner_kind, brace_span);
            return ExprKind::EnumInit { name, type_args, tag: tag.to_string(), value: Box::new(inner_expr) };
        }

        if !type_args.is_empty() {
            diagnostic::throw(
                format!("type arguments are only allowed when constructing a variant, like {}<...>::{}(...)", name, tag),
                Some(colon_span),
                &self.lexer.source,
            );
        }
        ExprKind::ColonAccess { name, tag: tag.to_string() }
    }

    /// Parses the argument list of a call to `name`, starting at `(`.
    ///
    /// `receiver` is the object for method calls like `obj.method()`,
    /// `type_args` the explicit type arguments of a generic call.
    ///
    /// # Panics
    ///
    /// Panics if an argument is invalid or the list isn't closed.
    fn parse_function_call(&mut self, name: String, receiver: Option<Box<Expr>>, type_args: Vec<Type>) -> ExprKind {
        self.consume(); // consume '('

        let mut params: Vec<Expr> = Vec::new();
        while !matches!(self.peek_or_eof("while parsing function call arguments"), (Token::RParen, _)) {
            params.push(self.next_expression());
            if !matches!(self.peek_or_eof("after function call argument"), (Token::RParen, _)) {
                self.expect(Token::Comma, "between function call arguments");
            }
        }
        self.consume(); // consume ')'
        ExprKind::FunctionCall { name, type_args, params, receiver, mangled: None }
    }

    /// Parses the `.` accesses following the already parsed `expr`, like
    /// `.field`, `.method(...)` or `.method<T>(...)`, in any combination such
    /// as `self.data.get(i).x`.
    ///
    /// A field access followed by `=` becomes an assignment `a.b = value`.
    ///
    /// # Panics
    ///
    /// Panics if a `.` isn't followed by a name.
    fn parse_postfix(&mut self, mut expr: Expr) -> Expr {
        while matches!(self.peek().0, Token::Dot) {
            self.consume(); // '.'
            // The access gets the position of the field or method name
            let (name, span) = self.expect(Token::Identifier(String::new()), "after '.'");
            let name = name.to_string();

            // A method call, possibly with explicit type arguments like `b.convert<float64>()`
            let type_args = if matches!(self.peek().0, Token::Lt) { self.try_parse_type_args() } else { None };
            expr = if type_args.is_some() || matches!(self.peek().0, Token::LParen) {
                let kind = self.parse_function_call(name, Some(Box::new(expr)), type_args.unwrap_or_default());
                Expr::new(kind, span)
            } else {
                Expr::new(ExprKind::FieldAccess { object: Box::new(expr), field: name }, span)
            };
        }

        if matches!(expr.kind, ExprKind::FieldAccess { .. }) && matches!(self.peek().0, Token::Assign) {
            self.consume(); // '='
            let span = expr.span.clone();
            let rhs = self.next_expression();
            expr = Expr::new(ExprKind::Assignment { lhs: Box::new(expr), rhs: Box::new(rhs) }, span);
        }
        expr
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

    /// Parses a conversion `value as Type`, starting at `as`.
    ///
    /// # Panics
    ///
    /// Panics if `as` isn't followed by a type.
    fn parse_conversion(&mut self, value: Expr) -> Expr {
        self.consume(); // consume 'as'
        let (target, _) = self.parse_type();
        let span = value.span.clone();
        Expr::new(ExprKind::Convert { value: Box::new(value), target }, span)
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
        Token::EqEq | Token::NotEq | Token::Lt | Token::LtEq | Token::Gt | Token::GtEq => Some(1),
        Token::Plus | Token::Minus       => Some(2),
        Token::Multiply | Token::Divide  => Some(3),
        _ => None,
    }
}
