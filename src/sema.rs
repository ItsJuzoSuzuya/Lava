use std::{cell::Cell, collections::{HashMap, HashSet}};

use crate::{declaration::{Declaration, DeclarationKind, Extern, ExternItem, TypeParam, Visibility}, diagnostic::{self, CompileError}, module::{self, Import}, runtime, expression::{Block, Expr, ExprKind, MatchArm, Pattern, prelude_variant_enum}, field::Field, param::Param, span::Span, statement::{Stmt, StmtKind}, token::Token, r#type::Type};

enum SemanticError {
    AmbiguousMethod { method: String, class: String, traits: Vec<String>, span: Span },
    AssignOnUnmutable { var: String, span: Span },
    AssignToSelf { span: Span },
    BuiltinRedeclared { name: String, span: Span },
    CannotInferTypeParam { param: String, function: String, span: Span },
    CannotMoveBorrowed { what: String, span: Span },
    CannotNegate { ty: Type, span: Span },
    CannotPrintUnique { span: Span },
    DuplicateImpl { trait_name: String, ty: String, span: Span },
    GenericMethod { method: String, span: Span },
    InvalidConversion { from: Type, to: Type, span: Span },
    InvalidImplTarget { ty: Type, span: Span },
    InvalidOperands { op: Token, ty: Type, span: Span },
    InvalidReceiver { recv_ty: Type, span: Span, context: &'static str },
    LiteralOutOfRange { value: i64, ty: Type, span: Span },
    MissingArgument { function: String, index: usize, span: Span },
    MissingTraitMethod { method: String, trait_name: String, ty: String, span: Span },
    MissingTypeArgs { name: String, span: Span },
    MoveInLoop { var: String, span: Span },
    MutatingCallOnImmutable { method: String, var: String, span: Span },
    NotATraitMethod { method: String, trait_name: String, span: Span },
    NotGeneric { function: String, span: Span },
    NotIndexable { ty: Type, span: Span },
    NotAList { ty: Type, span: Span },
    ListPatternLength { items: u64, len: u64, rest: bool, span: Span },
    BlockWithoutValue { what: &'static str, span: Span },
    IfWithoutElse { span: Span },
    WithoutValue { what: &'static str, span: Span },
    NonExhaustiveMatch { span: Span },
    ArrayLength { expected: u64, got: usize, span: Span },
    ArrayIndexOutOfRange { index: i64, len: u64, span: Span },
    EmptyArrayType { span: Span },
    ExternType { ty: Type, span: Span },
    ExternReturnType { ty: Type, span: Span },
    ModuleNotImported { module: String, span: Span },
    UnknownImport { item: String, module: String, span: Span },
    StringImmutable { span: Span },
    CannotPrintArray { span: Span },
    OutsideLoop { keyword: &'static str, span: Span },
    PrivateFieldAccess { field_name: String, span: Span },
    PrivateFunctionCall { func_name: String, parent: String, span: Span },
    TooManyArguments { func_name: String, parent: String, span: Span },
    TraitMethodMismatch { method: String, trait_name: String, span: Span },
    TraitNotImplemented { ty: Type, trait_name: String, span: Span },
    TypeMismatch { lhs: Type, rhs: Type, span: Span, context: &'static str },
    UnknownEnum { name: String, span: Span },
    UnknownField { class: String, field: String, span: Span },
    UnknownFunction { fn_name: String, span: Span },
    UnknownIdentifier { identifier: String, span: Span },
    UnknownTrait { name: String, span: Span },
    UnknownType { name: String, span: Span },
    UnknownVariant { enum_name: String, variant: String, span: Span },
    UseAfterMove { var: String, span: Span },
    VariantNeedsPayload { variant: String, payload: Type, span: Span },
    VariantWithoutPayload { variant: String, span: Span },
    WrongArgCount { function: String, expected: usize, got: usize, span: Span },
    WrongTypeArgCount { function: String, expected: usize, got: usize, span: Span },
}

impl SemanticError {
    fn throw(self, source: &str) -> ! {
        let (line, col, msg) = match self {
            SemanticError::AmbiguousMethod { method, class, traits, span } =>
                (span.line, span.col, format!("method '{}' of '{}' is ambiguous, it comes from the traits '{}'", method, class, traits.join("', '"))),
            SemanticError::AssignOnUnmutable{ var, span } =>
                (span.line, span.col, format!("Tried reassigning a value to the unmutable Variable '{}'", var)),
            SemanticError::AssignToSelf { span } =>
                (span.line, span.col, "can't assign to 'self' of a class, assign its fields instead".to_string()),
            SemanticError::BuiltinRedeclared { name, span } =>
                (span.line, span.col, format!("'{}' is part of the language, it can't be declared again", name)),
            SemanticError::CannotMoveBorrowed { what, span } =>
                (span.line, span.col, format!("can't move out of '{}', it's only borrowed, use '{}.clone()' for a copy", what, what)),
            SemanticError::CannotInferTypeParam { param, function, span } =>
                (span.line, span.col, format!("can't infer type parameter '{}' of '{}', give it explicitly like {}<...>(...)", param, function, function)),
            SemanticError::CannotPrintUnique { span } =>
                (span.line, span.col, "a Unique, Shared or Buffer can't be printed, print the values inside instead".to_string()),
            SemanticError::CannotNegate { ty, span } =>
                (span.line, span.col, format!("cannot negate a value of type '{}'", ty)),
            SemanticError::DuplicateImpl { trait_name, ty, span } =>
                (span.line, span.col, format!("trait '{}' is already implemented for '{}'", trait_name, ty)),
            SemanticError::GenericMethod { method, span } =>
                (span.line, span.col, format!("method '{}' can't be generic, trait methods and their impls can't have type parameters", method)),
            SemanticError::InvalidConversion { from, to, span } =>
                (span.line, span.col, format!("cannot convert '{}' to '{}'", from, to)),
            SemanticError::InvalidImplTarget { ty, span } =>
                (span.line, span.col, format!("traits can't be implemented for '{}'", ty)),
            SemanticError::InvalidOperands { op, ty, span } =>
                (span.line, span.col, format!("'{}' can't be used on values of type '{}'", op, ty)),
            SemanticError::InvalidReceiver { recv_ty, span, context } =>
                (span.line, span.col, format!("'{}' {}", recv_ty, context)),
            SemanticError::LiteralOutOfRange { value, ty, span } =>
                (span.line, span.col, format!("literal '{}' doesn't fit into '{}'", value, ty)),
            SemanticError::MissingArgument { function, index, span } =>
                (span.line, span.col, format!("missing argument {} for '{}'", index, function)),
            SemanticError::MissingTraitMethod { method, trait_name, ty, span } =>
                (span.line, span.col, format!("'{}' is missing method '{}' of trait '{}'", ty, method, trait_name)),
            SemanticError::MissingTypeArgs { name, span } =>
                (span.line, span.col, format!("generic type '{}' needs type arguments, like {}<...>", name, name)),
            SemanticError::MoveInLoop { var, span } =>
                (span.line, span.col, format!("'{}' is moved inside the loop, it would be gone in the next round", var)),
            SemanticError::MutatingCallOnImmutable { method, var, span } =>
                (span.line, span.col, format!("method '{}' changes 'self', so '{}' has to be mutable", method, var)),
            SemanticError::NotATraitMethod { method, trait_name, span } =>
                (span.line, span.col, format!("method '{}' is not a member of trait '{}'", method, trait_name)),
            SemanticError::NotGeneric { function, span } =>
                (span.line, span.col, format!("'{}' isn't generic, it takes no type arguments", function)),
            SemanticError::BlockWithoutValue { what, span } =>
                (span.line, span.col, format!("this {} gives no value, end it with an expression without ';' or leave with 'return', 'break' or 'panic'", what)),
            SemanticError::IfWithoutElse { span } =>
                (span.line, span.col, "an if giving a value needs an else, which gives one too".to_string()),
            SemanticError::WithoutValue { what, span } =>
                (span.line, span.col, format!("this {} gives no value, but it's used as one", what)),
            SemanticError::NonExhaustiveMatch { span } =>
                (span.line, span.col, "a match giving a value has to handle every case, add the missing variants or a '_' arm".to_string()),
            SemanticError::NotAList { ty, span } =>
                (span.line, span.col, format!("a list pattern like [a, b] only matches an array or Vector, not '{}'", ty)),
            SemanticError::ListPatternLength { items, len, rest, span } =>
                (span.line, span.col, if rest {
                    format!("this pattern needs at least {} items, but the array only has {}", items, len)
                } else {
                    format!("this pattern has {} items, but the array has {}, use '..' for the rest", items, len)
                }),
            SemanticError::NotIndexable { ty, span } =>
                (span.line, span.col, format!("'{}' can't be indexed, it's no array, String or type with a 'get' method", ty)),
            SemanticError::ArrayLength { expected, got, span } =>
                (span.line, span.col, format!("the array needs {} values, got {}", expected, got)),
            SemanticError::ArrayIndexOutOfRange { index, len, span } =>
                (span.line, span.col, format!("index {} is out of bounds for an array of length {}", index, len)),
            SemanticError::EmptyArrayType { span } =>
                (span.line, span.col, "the type of an empty array can't be inferred, give it like `let a: int32[0] = {};`".to_string()),
            SemanticError::ExternType { ty, span } =>
                (span.line, span.col, format!("'{}' can't be passed to a C function, use a number, char, String, extern type, Option of an extern type, or a slice or &Buffer of numbers or chars", ty)),
            SemanticError::ExternReturnType { ty, span } =>
                (span.line, span.col, format!("a C function can't return '{}', only a number, String, extern type, Option of an extern type or nothing", ty)),
            SemanticError::ModuleNotImported { module, span } =>
                (span.line, span.col, format!("module '{}' isn't imported, add 'import std::{};'", module, module)),
            SemanticError::UnknownImport { item, module, span } =>
                (span.line, span.col, format!("module '{}' has no '{}'", module, item)),
            SemanticError::StringImmutable { span } =>
                (span.line, span.col, "a String can't be changed, build a new one instead".to_string()),
            SemanticError::CannotPrintArray { span } =>
                (span.line, span.col, "an array can't be printed, print its values instead".to_string()),
            SemanticError::OutsideLoop { keyword, span } =>
                (span.line, span.col, format!("'{}' can only be used inside a loop", keyword)),
            SemanticError::PrivateFieldAccess { field_name, span } =>
                (span.line, span.col, format!("Tried accessing a private field '{}'", field_name)),
            SemanticError::PrivateFunctionCall { func_name, parent,  span } =>
                (span.line, span.col, format!("Tried calling a private function '{}' in '{}'", func_name, parent)),
            SemanticError::TooManyArguments {  func_name, parent,  span } =>
                (span.line, span.col, format!("Too many arguments provided for function '{}' in '{}'", func_name, parent)),
            SemanticError::TraitMethodMismatch { method, trait_name, span } =>
                (span.line, span.col, format!("method '{}' doesn't match its signature in trait '{}'", method, trait_name)),
            SemanticError::TraitNotImplemented { ty, trait_name, span } => {
                // As written, `Default` for `default::Default`
                let written = match trait_name.split_once("::") {
                    Some((module, rest)) if !module.contains('<') => rest,
                    _ => trait_name.as_str(),
                };
                (span.line, span.col, format!("'{}' doesn't implement trait '{}'", crate::analysis::show_type(&ty), written))
            }
            SemanticError::TypeMismatch { lhs, rhs, span, context } =>
                (span.line, span.col, format!("type '{}' does not match '{}' in {}", lhs, rhs, context)),
            SemanticError::UnknownEnum { name, span } =>
                (span.line, span.col, format!("unknown enum '{}'", name)),
            SemanticError::UnknownField { class, field, span } =>
                (span.line, span.col, format!("class '{}' has no field '{}'", class, field)),
            SemanticError::UnknownFunction { fn_name, span } =>
                (span.line, span.col, format!("unknown function '{}'", fn_name)),
            SemanticError::UnknownIdentifier { identifier, span } =>
                (span.line, span.col, format!("unknown identifier '{}'", identifier)),
            SemanticError::UnknownTrait { name, span } =>
                (span.line, span.col, format!("unknown trait '{}'", name)),
            SemanticError::UnknownType { name, span } =>
                (span.line, span.col, format!("unknown type '{}'", name)),
            SemanticError::UseAfterMove { var, span } =>
                (span.line, span.col, format!("use of moved value '{}', it was moved somewhere else, use '{}.clone()' there to keep a copy", var, var)),
            SemanticError::VariantNeedsPayload { variant, payload, span } =>
                (span.line, span.col, format!("'{}' carries a '{}', create it like {}(value)", variant, payload, variant)),
            SemanticError::VariantWithoutPayload { variant, span } =>
                (span.line, span.col, format!("'{}' carries no value, there's nothing to bind, match it like {}", variant, variant)),
            SemanticError::WrongArgCount { function, expected, got, span } =>
                (span.line, span.col, format!("'{}' takes {} arguments, got {}", function, expected, got)),
            SemanticError::WrongTypeArgCount { function, expected, got, span } =>
                (span.line, span.col, format!("'{}' takes {} type arguments, got {}", function, expected, got)),
            SemanticError::UnknownVariant { enum_name, variant, span } =>
                (span.line, span.col, format!("enum '{}' has no variant '{}'", enum_name, variant)),
        };
        diagnostic::throw(msg, Some(Span { line, col, len: 1 }), source)
    }
}

/// A generic function, method, class or enum together with where it's declared,
/// since its instances are checked in that context.
#[derive(Clone)]
struct GenericDecl {
    decl: Declaration,
    module: String,
    source: String,
    /// What the module declaring it imports, its body uses those names.
    imports: Imports,
    /// The class a generic method belongs to, `None` for everything else.
    class: Option<String>,
    /// Type arguments already fixed by the surroundings, like `T = int32` for a
    /// generic method of `Box<int32>`.
    outer_subst: HashMap<String, Type>,
}

/// The names a module sees from the standard library, see
/// [`Sema::imported_modules`] and [`Sema::aliases`].
#[derive(Clone, Default)]
struct Imports {
    imported_modules: Vec<String>,
    aliases: HashMap<String, String>,
    ambiguous: HashMap<String, Vec<String>>,
}

/// An instance of a generic class or enum whose method bodies still have to
/// be checked. That happens at the end of the module, since instances can be
/// created while the module's own declarations are still being processed.
struct PendingInstance {
    decl: Declaration,
    subst: HashMap<String, Type>,
    module: String,
    source: String,
    imports: Imports,
}

/// A method of an instance of a generic class or enum, like `push` of
/// `Vector<Token>`. It's only checked once something calls it, so an instance
/// only needs what its used methods need, like `T` having a default only for
/// a constructor filling elements with defaults. See [`Sema::use_method`].
struct LazyMethod {
    /// Index of its instance in [`Sema::type_instance_decls`].
    instance: usize,
    method: Declaration,
    /// The instance's module and imports, with the instance as the class.
    template: GenericDecl,
    subst: HashMap<String, Type>,
}

/// What [`Sema::check_type_instance`] passes on to the class or enum being
/// checked: its methods are registered as [`LazyMethod`]s instead of checked.
struct LazyInstance {
    instance: usize,
    template: GenericDecl,
    subst: HashMap<String, Type>,
}

/// The type an impl method returns for a trait method returning `impl Trait`.
/// Whether it implements the trait is checked at the end of the module, when
/// all impls are known, see [`Sema::check_opaque_returns`].
struct OpaqueReturn {
    ty: Type,
    /// Like `Iterator<int32>`.
    trait_name: String,
    method: String,
    span: Span,
    source: String,
}

/// The steps of checking a program, see [`Sema::check_phase`]. Each runs for
/// every module before the next one starts, so every module knows the types
/// and functions of all others, also of the ones importing it back, like an
/// `expression` and a `statement` module using each other's types.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    /// The names of classes, enums and extern types.
    TypeNames,
    /// The types written in declarations, resolved to the ones they mean.
    Signatures,
    Traits,
    GenericImpls,
    /// Impls without type parameters, checked against their trait.
    Impls,
    /// Fields, functions and methods with their signatures.
    Declarations,
    /// Checks on the declared types, like a class containing itself.
    Validation,
    /// Statements and function bodies.
    Bodies,
}

impl Phase {
    pub const ALL: [Phase; 8] = [
        Phase::TypeNames, Phase::Signatures, Phase::Traits, Phase::GenericImpls,
        Phase::Impls, Phase::Declarations, Phase::Validation, Phase::Bodies,
    ];
}

/// Who owns the value a match looks at, so its bindings know whether they
/// can be moved out of.
#[derive(Clone)]
enum Owner {
    /// A temporary like a call's result. Its bindings own their values.
    Temporary,
    /// The variable, moving a binding moves out of it.
    Variable(String),
    /// Something borrowed like `self.kind`, its bindings can only be read.
    Borrowed,
}

pub struct Sema {
    scope: HashMap<String, (Type, bool)>,
    /// Params, return type, visibility and the name of the module the function is declared in.
    functions: HashMap<String, (Vec<Param>, Type, Visibility, String)>,
    classes: HashMap<String, Vec<Field>>,
    enums: HashMap<String, Vec<(String, u32, Type)>>,
    /// Methods of every trait, by trait name. An instance of a generic trait is
    /// named like `Iterator<int32>`.
    traits: HashMap<String, Vec<Declaration>>,
    /// Generic traits like `Iterator<T>` by name, templates for their instances.
    generic_traits: HashMap<String, GenericDecl>,
    /// Type arguments of every instance of a generic trait by instance name. The
    /// default methods copied into an impl are checked with them.
    trait_substs: HashMap<String, HashMap<String, Type>>,
    /// Generic impls like `impl<T> Iterator<T> for VectorIter<T>`, applied to
    /// every instance of their target.
    generic_impls: Vec<GenericDecl>,
    /// Which generic impls were applied to which instance, by index into
    /// [`Sema::generic_impls`] and instance name.
    applied_impls: HashSet<(usize, String)>,
    /// Every `Buffer` type with a known element type seen so far, which the
    /// generic impls for `Buffer<T>` apply to.
    buffer_types: Vec<Type>,
    /// The mangled names of each class's constructors in order, like `Point`
    /// and `Point#2`, by class name. See [`Sema::add_constructor`].
    constructors: HashMap<String, Vec<String>>,
    /// Checked instances of generic classes and enums, like `Vector<Token>`,
    /// with the methods checked so far, for codegen.
    type_instance_decls: Vec<Declaration>,
    /// Methods of instances nothing called yet, by mangled name.
    lazy_methods: HashMap<String, LazyMethod>,
    /// Every function and method a call went to, by mangled name, with where
    /// it was called first and that module's source, for errors.
    used_functions: HashMap<String, (Span, String)>,
    /// Lazy methods something called, waiting to be checked.
    used_methods: Vec<String>,
    /// Set while an instance of a generic class or enum is checked, see [`LazyInstance`].
    lazy_instance: Option<LazyInstance>,
    /// Whether the impls of every module are known, which they are from
    /// [`Phase::Declarations`] on. Before, a missing impl may still come.
    impls_known: bool,
    /// Bounds of generic types that weren't met while impls weren't known
    /// yet: the type, the trait, where, and in which module. Checked again
    /// with that module's bodies.
    deferred_bounds: Vec<(Type, String, Span, String)>,
    /// Generic impls by index and the instance they didn't apply to while
    /// impls weren't known yet, because a bound wasn't met.
    deferred_impls: Vec<(usize, Type)>,
    /// Types returned for `impl Trait` that still have to be checked.
    opaque_returns: Vec<OpaqueReturn>,
    /// Names of the traits each type implements, by type name.
    impls: HashMap<String, Vec<String>>,
    current_class: Option<String>,
    /// Trait of the impl block currently being declared.
    current_trait: Option<String>,
    /// Generic functions by name. They are templates, only their instances get checked.
    generic_functions: HashMap<String, GenericDecl>,
    /// Generic classes and enums by name, templates like the generic functions.
    generic_types: HashMap<String, GenericDecl>,
    /// Template name and type arguments of every instance of a generic class or
    /// enum, by instance name like `Box<int32>`.
    type_instances: HashMap<String, (String, Vec<Type>)>,
    /// Instances of generic classes and enums whose bodies aren't checked yet.
    pending_instances: Vec<PendingInstance>,
    /// Instances of generic functions, classes and enums created while checking
    /// the current module, waiting to be handed to codegen with [`Sema::take_instances`].
    instances: Vec<Declaration>,
    /// Type arguments of the generic instance currently being checked, by type parameter name.
    type_subst: HashMap<String, Type>,
    /// Type the next checked expression is expected to have, see [`Sema::check_expr_expecting`].
    expected_type: Option<Type>,
    /// Variables in scope whose value got moved out, so they can't be used anymore.
    moved: HashSet<String>,
    /// Names that only borrow their value, like `self` or a binding of a match
    /// on `self.kind`. Moving out of them is an error, a `.clone()` copies.
    borrowed: HashSet<String>,
    /// Match bindings by name with the variable owning the matched value.
    /// Moving out of a binding moves out of that variable.
    binding_roots: HashMap<String, String>,
    /// Who owns the value the patterns being checked bind into, see [`Owner`].
    pattern_owner: Owner,
    /// Names bound by `value is pattern` expressions that are still in scope,
    /// the most recent last, with what they meant before.
    is_bindings: Vec<IsBinding>,
    /// Whether the expression checked next is a condition whose `is` bindings
    /// stay in scope afterwards, like the left side of `&&` or the condition of
    /// an `if`. Anywhere else they're gone once the expression is checked.
    keep_is_bindings: bool,
    /// Whether the expression checked next only reads its value instead of
    /// taking it, see [`Sema::check_borrowed`].
    borrowing: bool,
    /// Whether the expression checked next is the object of a field access,
    /// like `a.b` in `a.b.c`, which can have other fields moved out.
    field_object: bool,
    current_return_type: Option<Type>,
    /// Int literals too big for their default type `int32` that haven't gotten
    /// another type from their context yet, by their position.
    oversized_literals: HashMap<(usize, usize), (i64, Span)>,
    /// How many loops are around the statement being checked.
    loop_depth: usize,
    /// Name of the module currently being checked.
    module: String,
    source: String,
    /// `extern func`s by mangled name, the C functions they call.
    externs: HashMap<String, Extern>,
    /// Names of the `extern type`s, see [`Type::Handle`].
    handles: HashSet<String>,
    /// The extern type each one with `extern type Name: Base` is a kind of.
    handle_bases: HashMap<String, String>,
    /// The type the method call being resolved should give, from its type
    /// argument or where its value goes. Picks between impls of one trait for
    /// different types, like `text.try_as<int32>()`, see [`Sema::find_method`].
    method_target: Option<Type>,
    /// Whether [`Sema::method_target`] picked the method, so its type argument was used up.
    method_target_used: Cell<bool>,
    /// Names of the modules the current module imports, `fs` for
    /// `import std::fs;` and `llvm` for `import llvm;`. Their items are used
    /// like `fs::read` or `llvm::Module`.
    imported_modules: Vec<String>,
    /// The top-level names of each of the program's own modules, without the
    /// module's name, by module name. See [`Sema::set_module_names`].
    module_names: HashMap<String, Vec<String>>,
    /// The top-level names of the program's own modules that keep their
    /// names, like the main file, by module name.
    plain_names: HashMap<String, Vec<String>>,
    /// Names several modules the current module imports declare, with those
    /// modules. They have to be written with the module's name, like `llvm::Module`.
    ambiguous: HashMap<String, Vec<String>>,
    /// Items of standard library modules the current module uses without the
    /// module's name, like `read` for `fs::read` after `import std::fs::{read};`.
    aliases: HashMap<String, String>,
    /// The imported items of [`Sema::aliases`] and where they're imported,
    /// checked to exist when the module's check starts.
    imported_items: Vec<(String, String, Span)>,
    /// Errors of the current module so far. Checking goes on after an error
    /// with the next statement, see [`Sema::recover`].
    errors: Vec<CompileError>,
    /// Whether the statement being checked used something whose type is
    /// unknown because of an earlier error. Its errors would only follow from that one.
    used_unknown: Cell<bool>,
}

/// A name bound by `value is pattern`, with what the name meant before, to
/// put back once the binding is out of scope.
struct IsBinding {
    name: String,
    scope: Option<(Type, bool)>,
    /// The moved paths of the earlier variable, like `p` or `p.left`.
    moved: Vec<String>,
    borrowed: bool,
    root: Option<String>,
}

/// What [`Sema::recover`] puts back when checking a statement fails halfway.
struct Snapshot {
    scope: HashMap<String, (Type, bool)>,
    is_bindings: usize,
    method_target: Option<Type>,
    moved: HashSet<String>,
    borrowed: HashSet<String>,
    binding_roots: HashMap<String, String>,
    pattern_owner: Owner,
    borrowing: bool,
    field_object: bool,
    expected_type: Option<Type>,
    current_return_type: Option<Type>,
    loop_depth: usize,
    current_class: Option<String>,
    current_trait: Option<String>,
    type_subst: HashMap<String, Type>,
    module: String,
    source: String,
    imports: Imports,
}

impl Sema {
    /// Creates a sema with the built-in functions already declared.
    ///
    /// Codegen defines the same functions in `CodegenContext::define_builtins`.
    pub fn new() -> Self {
        let mut sema = Self {
            scope: HashMap::new(), functions: HashMap::new(), classes: HashMap::new(), enums: HashMap::new(),
            traits: HashMap::new(), impls: HashMap::new(), current_class: None, current_trait: None,
            generic_traits: HashMap::new(), trait_substs: HashMap::new(), generic_impls: Vec::new(), applied_impls: HashSet::new(), buffer_types: Vec::new(), constructors: HashMap::new(),
            type_instance_decls: Vec::new(), lazy_methods: HashMap::new(), used_functions: HashMap::new(), used_methods: Vec::new(), lazy_instance: None,
            impls_known: false, deferred_bounds: Vec::new(), deferred_impls: Vec::new(),
            opaque_returns: Vec::new(),
            generic_functions: HashMap::new(), generic_types: HashMap::new(), type_instances: HashMap::new(),
            pending_instances: Vec::new(), instances: Vec::new(), type_subst: HashMap::new(), expected_type: None,
            moved: HashSet::new(), borrowed: HashSet::new(), binding_roots: HashMap::new(),
            pattern_owner: Owner::Temporary, is_bindings: Vec::new(), keep_is_bindings: false, borrowing: false, field_object: false,
            source: String::new(), module: String::new(), current_return_type: None, oversized_literals: HashMap::new(), loop_depth: 0,
            externs: HashMap::new(), handles: HashSet::new(), handle_bases: HashMap::new(), method_target: None, method_target_used: Cell::new(false), imported_modules: Vec::new(), module_names: HashMap::new(), plain_names: HashMap::new(), ambiguous: HashMap::new(), aliases: HashMap::new(), imported_items: Vec::new(),
            errors: Vec::new(), used_unknown: Cell::new(false),
        };
        for builtin in runtime::builtins() {
            let params = builtin.params.iter().map(|(name, ty)| Param::builtin(name, ty.clone())).collect();
            sema.push_func(builtin.name.to_string(), params, builtin.ret, Visibility::Public);
        }
        sema
    }

    /// Switches to checking the module `module` with the source code `source`,
    /// which has the imports `imports`.
    ///
    /// Declarations of previously checked modules stay known, local variables are dropped.
    pub fn enter_module(&mut self, module: String, source: String, imports: &[Import]) {
        self.module = module;
        self.source = source;
        self.scope.clear();
        self.moved.clear();
        self.imported_modules.clear();
        self.aliases.clear();
        self.ambiguous.clear();
        self.imported_items.clear();
        // Which of the program's own imported modules declare each name
        let mut declared_by: HashMap<String, Vec<String>> = HashMap::new();
        for import in imports {
            // `fs` for `std::fs`, the program's own modules are named like their file
            let (module, own) = match import.name.strip_prefix("std::") {
                Some(std_name) => (std_name, false),
                None => (import.name.as_str(), true),
            };
            if !self.imported_modules.iter().any(|m| m == module) {
                self.imported_modules.push(module.to_string());
            }
            for (item, span) in &import.items {
                let qualified = format!("{}::{}", module, item);
                self.aliases.insert(item.clone(), qualified.clone());
                self.imported_items.push((qualified, import.name.clone(), span.clone()));
            }
            if let (true, Some(names)) = (own, self.module_names.get(module)) {
                for name in names {
                    let modules = declared_by.entry(name.clone()).or_default();
                    if !modules.iter().any(|m| m == module) {
                        modules.push(module.to_string());
                    }
                }
            }
        }
        // A name only one imported module declares is used without the module's name, like
        // `Lexer` after `import lexer;`. Listed items come first, and in a module whose names
        // stay as they are, like the main file, its own names. The other modules' own names
        // have their module's name already
        let own_names = self.plain_names.get(&self.module).cloned().unwrap_or_default();
        for (name, modules) in declared_by {
            if self.aliases.contains_key(&name) || own_names.contains(&name) {
                continue;
            }
            match modules.as_slice() {
                [module] => { self.aliases.insert(name.clone(), format!("{}::{}", module, name)); }
                _ => { self.ambiguous.insert(name, modules); }
            }
        }
    }

    /// Gives sema the top-level names of the program's own modules by module.
    /// Those of `qualified` get the module's name, like `Module` in `llvm`
    /// becomes `llvm::Module`, so a module importing one can use them without
    /// it, see [`Sema::enter_module`]. Those of `plain`, like the main file,
    /// keep their names.
    pub fn set_module_names(&mut self, qualified: HashMap<String, Vec<String>>, plain: HashMap<String, Vec<String>>) {
        self.module_names = qualified;
        self.plain_names = plain;
    }

    /// Whether the items of the module `name` can be used like `name::item` in
    /// the current module: it's imported, or it's the current module.
    fn sees_module(&self, name: &str) -> bool {
        self.imported_modules.iter().any(|m| m == name)
            || self.module.strip_prefix("std::") == Some(name) || self.module == name
    }

    /// The mangled name of the function `function` of the class, enum or
    /// extern type `owner` that has no `self`, for calling it like
    /// `Owner::function(...)`. `None` if there's none, or `function` is a
    /// variant of the enum `owner`.
    fn static_function(&self, owner: &str, function: &str) -> Option<String> {
        let owner = self.aliases.get(owner).cloned().unwrap_or_else(|| owner.to_string());
        let is_type = self.classes.contains_key(&owner) || self.enums.contains_key(&owner) || self.handles.contains(&owner);
        let is_variant = self.enums.get(&owner).is_some_and(|variants| variants.iter().any(|(v, _, _)| v == function));
        if !is_type || is_variant {
            return None;
        }
        // An extern type also has the functions of the one it's a kind of
        let mut current = owner;
        for _ in 0..64 {
            let mangled = format!("{}_{}", current, function);
            if let Some((params, ..)) = self.functions.get(&mangled) {
                return params.first().is_none_or(|p| p.name != "self").then_some(mangled);
            }
            current = self.handle_bases.get(&current)?.clone();
        }
        None
    }

    /// How the current module writes the item `name`, for errors: `Option` for
    /// `option::Option` after `import std::option::{Option};`, else `name`.
    fn written_name(&self, name: &str) -> String {
        // The module's own item, like `llvm::LLVMModule` in `llvm.lv`, is written without it
        let own = format!("{}::", self.module.strip_prefix("std::").unwrap_or(&self.module));
        if let Some(short) = name.strip_prefix(&own).filter(|short| !short.contains("::")) {
            return short.to_string();
        }
        self.aliases.iter()
            .find(|(_, qualified)| qualified.as_str() == name)
            .map(|(short, _)| short.clone())
            .unwrap_or_else(|| name.to_string())
    }

    /// The enum `name`, `Option` or `Result`, means in the current module, for
    /// `Some(x)`, `None`, `Ok(x)` and `Err(e)` without the enum's name: an own
    /// one, else the one of `std::option` or `std::result` if the module
    /// imports that, like `option::Option`.
    fn variant_enum(&self, name: &str) -> String {
        if let Some(imported) = self.aliases.get(name) {
            return imported.clone();
        }
        let own = self.enums.contains_key(name) || self.generic_types.contains_key(name);
        let module = name.to_lowercase();
        if !own && self.sees_module(&module) {
            return format!("{}::{}", module, name);
        }
        name.to_string()
    }

    /// Whether `name` stands for a standard library module where it's
    /// written like `name::item`, and not for an enum.
    /// Whether `name` is an enum, also a generic one like `Option`.
    fn is_enum_name(&self, name: &str) -> bool {
        self.enums.contains_key(name)
            || self.generic_types.get(name).is_some_and(|generic| matches!(generic.decl.kind, DeclarationKind::Enum { .. }))
    }

    fn is_module_prefix(&self, name: &str) -> bool {
        !self.enums.contains_key(name) && !self.generic_types.contains_key(name)
            && (self.sees_module(name) || module::std_source(name).is_some())
    }

    /// The function or type `name` means in the current module: an item
    /// imported from the standard library with its module's name, like
    /// `fs::read` for `read` after `import std::fs::{read};`. Else `name`.
    ///
    /// # Panics
    ///
    /// Panics if `name` is like `fs::read` and the module isn't imported.
    fn resolve_name(&self, name: &str, span: &Span) -> String {
        // Instance names like `Vector<env::Args>` are no module's items
        let qualified = name.split_once("::").filter(|(prefix, _)| prefix.chars().all(|c| c.is_alphanumeric() || c == '_'));
        if name.contains("::") && qualified.is_none() {
            return name.to_string();
        }
        if let Some((prefix, _)) = qualified {
            if module::std_source(prefix).is_none() && !self.sees_module(prefix) {
                SemanticError::UnknownFunction { fn_name: name.to_string(), span: span.clone() }.throw(&self.source);
            }
            if !self.sees_module(prefix) {
                SemanticError::ModuleNotImported { module: prefix.to_string(), span: span.clone() }.throw(&self.source);
            }
            return name.to_string();
        }
        if let Some(qualified) = self.aliases.get(name) {
            return qualified.clone();
        }
        self.check_unambiguous(name, span);
        name.to_string()
    }

    /// Stops with an error if several imported modules declare `name`, it has
    /// to be written with the module's name then, like `llvm::Module`.
    fn check_unambiguous(&self, name: &str, span: &Span) {
        let Some(modules) = self.ambiguous.get(name) else { return };
        let options: Vec<String> = modules.iter().map(|module| format!("`{}::{}`", module, name)).collect();
        let modules: Vec<String> = modules.iter().map(|module| format!("'{}'", module)).collect();
        diagnostic::throw(
            format!("'{}' is in both {}, write {}", name, modules.join(" and "), options.join(" or ")),
            Some(span.clone()),
            &self.source,
        )
    }

    /// Type of the variable `name`, for using it in an expression.
    ///
    /// # Panics
    ///
    /// Panics if there's no such variable, or its `Unique` got moved out already.
    fn use_variable(&self, name: &str, span: &Span) -> Type {
        let ty = self.use_variable_field(name, span);
        // A value with a field moved out can't be used as a whole anymore
        let prefix = format!("{}.", name);
        if let Some(path) = self.moved.iter().find(|path| path.starts_with(&prefix)) {
            SemanticError::UseAfterMove { var: path.clone(), span: span.clone() }.throw(&self.source);
        }
        ty
    }

    /// Like [`Sema::use_variable`], for reading a field of the variable
    /// `name`, which works if other fields of it were moved out.
    fn use_variable_field(&self, name: &str, span: &Span) -> Type {
        let (ty, _) = self.scope.get(name).cloned()
            .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                identifier: name.to_string(),
                span: span.clone(),
            }.throw(&self.source));
        // Declared by a statement with an error
        if ty == Type::Unknown {
            self.used_unknown.set(true);
        }
        if self.moved.contains(name) {
            SemanticError::UseAfterMove { var: name.to_string(), span: span.clone() }.throw(&self.source);
        }
        ty
    }

    /// Whether values of `ty` own heap memory, so using one as a value moves
    /// it: a `String`, `Unique` or `Buffer`, a class with a field or an enum
    /// with a payload that does, or an array of such values. Copies of them
    /// are made explicitly with `.clone()`.
    fn moves(&self, ty: &Type) -> bool {
        self.moves_within(ty, 0)
    }

    /// [`Sema::moves`], giving up on types nested deeper than any real one,
    /// which can only be a type containing itself.
    fn moves_within(&self, ty: &Type, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match ty {
            Type::Unique(_) | Type::Buffer(_) => true,
            Type::Class(name) => self.classes.get(name)
                .is_some_and(|fields| fields.iter().any(|f| self.moves_within(&f.ty, depth + 1))),
            Type::Enum(name) => self.enums.get(name)
                .is_some_and(|variants| variants.iter().any(|(_, _, payload)| self.moves_within(payload, depth + 1))),
            Type::Array(element, _) => self.moves_within(element, depth + 1),
            Type::Tuple(items) => items.iter().any(|item| self.moves_within(item, depth + 1)),
            _ => false,
        }
    }

    /// Whether values of `ty` own heap memory only through `Shared`s, like a
    /// `Shared` itself or a class with `Shared` fields. Using one as a value
    /// copies it, which only counts one more user of each `Shared`.
    fn shares(&self, ty: &Type) -> bool {
        !self.moves(ty) && self.has_shared(ty, 0)
    }

    /// Whether `ty` is or holds a `Shared`, see [`Sema::shares`].
    fn has_shared(&self, ty: &Type, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match ty {
            Type::Shared(_) => true,
            Type::Class(name) => self.classes.get(name)
                .is_some_and(|fields| fields.iter().any(|f| self.has_shared(&f.ty, depth + 1))),
            Type::Enum(name) => self.enums.get(name)
                .is_some_and(|variants| variants.iter().any(|(_, _, payload)| self.has_shared(payload, depth + 1))),
            Type::Array(element, _) => self.has_shared(element, depth + 1),
            Type::Tuple(items) => items.iter().any(|item| self.has_shared(item, depth + 1)),
            _ => false,
        }
    }

    /// Makes `expr`, a value of type `ty` taken from a variable, field or
    /// element, a copy of it if `ty` [shares](Sema::shares), so the place keeps its value.
    fn copy_shared(&self, expr: &mut Expr, ty: &Type) {
        let is_place = matches!(expr.kind, ExprKind::Identifier(_) | ExprKind::FieldAccess { .. } | ExprKind::Index { .. });
        if !is_place || !self.shares(ty) {
            return;
        }
        let span = expr.span.clone();
        let mut place = std::mem::replace(expr, Expr::new(ExprKind::Boolean(false), span.clone()));
        place.ty = ty.clone();
        *expr = Expr::new(ExprKind::FunctionCall {
            name: "clone".to_string(),
            type_args: vec![],
            params: vec![],
            receiver: Some(Box::new(place)),
            mangled: Some(CLONE.to_string()),
        }, span);
        expr.ty = ty.clone();
    }

    /// Turns `expr`, an `a ?? b`, into `match a { Some(value) => value, None => b }`,
    /// or the same with `Ok` and `Err` for a `Result`. `b` only runs if needed.
    ///
    /// `expected` is the type the whole expression should have. In
    /// `text.try_as() ?? 0` it, or else the default's type, says what `try_as` gives.
    ///
    /// # Panics
    ///
    /// Panics if `a` is neither an `Option` nor a `Result`.
    fn rewrite_default(&mut self, expr: &mut Expr, expected: Option<&Type>) {
        let span = expr.span.clone();
        let ExprKind::BinaryExpr { lhs, rhs, .. } = &mut expr.kind else { return };
        let picks_by_type = match &lhs.kind {
            ExprKind::FunctionCall { name, receiver: Some(receiver), type_args, .. } if type_args.is_empty() =>
                self.is_static_trait_function(name) || self.has_impls_by_type(receiver, name),
            _ => false,
        };
        if let ExprKind::FunctionCall { type_args, .. } = &mut lhs.kind {
            // `?? return x` has no type to pick by
            let leaves = matches!(rhs.kind, ExprKind::Leave(_));
            if picks_by_type {
                let wanted = expected.filter(|ty| **ty != Type::Unknown).cloned()
                    .or_else(|| (!leaves).then(|| self.probe_type(rhs)));
                if let Some(wanted) = wanted {
                    type_args.push(wanted);
                }
            }
        }
        let ty = self.probe_type(lhs);
        let template = match &ty {
            Type::Enum(instance) => self.type_instances.get(instance).map(|(template, _)| template.clone()),
            _ => None,
        };
        let (present, missing) = match template.as_deref() {
            Some(name) if name.ends_with("Option") => ("Some", "None"),
            Some(name) if name.ends_with("Result") => ("Ok", "Err"),
            _ => {
                // `list[i]` stops the program past the end, `list.at(i)` gives an Option instead
                let hint = if matches!(lhs.kind, ExprKind::Index { .. }) { ", use `.at(index)` instead of `[index]`" } else { "" };
                diagnostic::throw(format!("'??' gives the value of an Option or Result, '{}' is neither{}", ty, hint), Some(lhs.span.clone()), &self.source)
            }
        };
        let enum_name = template.expect("checked above");

        // A name no code can write, so it can't hide a variable
        let binding = "?? value".to_string();
        let placeholder = || Expr::new(ExprKind::Boolean(false), span.clone());
        let scrutinee = std::mem::replace(lhs.as_mut(), placeholder());
        let mut default = std::mem::replace(rhs.as_mut(), placeholder());

        // `v.at(5) ?? "none"` gives a `&String`: the default isn't copied either, a
        // variable is referenced and any other value is kept for the reference
        let present_ty = match &ty {
            Type::Enum(instance) => self.type_instances.get(instance).and_then(|(_, args)| args.first().cloned()),
            _ => None,
        };
        if let (Some(Type::Ref(value_ty)), false) = (present_ty, matches!(default.kind, ExprKind::Leave(_))) {
            if !matches!(self.probe_type(&default), Type::Ref(_)) {
                default = if matches!(default.kind, ExprKind::Identifier(_) | ExprKind::FieldAccess { .. }) {
                    Expr::new(ExprKind::Ref(Box::new(default)), span.clone())
                } else {
                    self.check_expr_expecting(&mut default, &value_ty);
                    self.coerce(&mut default, &value_ty);
                    let kept_ty = Type::Ref(Box::new(default.ty.clone()));
                    let default_span = default.span.clone();
                    Expr { kind: ExprKind::Kept(Box::new(default)), span: default_span, ty: kept_ty }
                };
            }
        }
        let arms = vec![
            MatchArm {
                pattern: Pattern::EnumVariant {
                    enum_name: enum_name.clone(),
                    variant: present.to_string(),
                    payload: Some(Box::new(Pattern::name(&binding, span.clone()))),
                },
                block: Block::of_value(Expr::new(ExprKind::Identifier(binding), span.clone())),
                span: span.clone(),
            },
            MatchArm {
                pattern: Pattern::EnumVariant { enum_name, variant: missing.to_string(), payload: None },
                // `?? return x`, `?? break` and `?? continue` leave, like an arm `None => break`
                block: match default.kind {
                    ExprKind::Leave(stmt) => Block { stmts: vec![*stmt], value: None },
                    _ => Block::of_value(default),
                },
                span: span.clone(),
            },
        ];
        expr.kind = ExprKind::Match { scrutinee: Box::new(scrutinee), arms };
    }

    /// Makes `expr`, a `Shared<T>` or `Unique<T>` or a reference to one, a
    /// `&T` to the value it holds, like `expr.get()`.
    fn get_heap_value(&self, expr: &mut Expr) {
        let Some(inner) = heap_inner(&expr.ty) else { return };
        let span = expr.span.clone();
        let mut receiver = std::mem::replace(expr, Expr::new(ExprKind::Boolean(false), span.clone()));
        // `&a` only borrows `a` already
        if let ExprKind::Ref(referenced) = receiver.kind {
            receiver = *referenced;
        }
        *expr = Expr::new(ExprKind::FunctionCall {
            name: "get".to_string(),
            type_args: vec![],
            params: vec![],
            receiver: Some(Box::new(receiver)),
            mangled: Some(GET.to_string()),
        }, span);
        expr.ty = Type::Ref(Box::new(inner));
    }

    /// Whether a value of type `ty` can be passed as a `&[element]`: an array
    /// of `element`s or a `Vector<element>`, also through a reference.
    fn slices_into(&self, ty: &Type, element: &Type) -> bool {
        match ty {
            Type::Ref(inner) => self.slices_into(inner, element),
            Type::Array(item, _) => **item == *element,
            Type::Class(name) => self.type_instances.get(name)
                .is_some_and(|(template, args)| is_vector(template) && args.as_slice() == std::slice::from_ref(element)),
            _ => false,
        }
    }

    /// Checks `expr`, whose value is only read, like the operand of `print` or
    /// the receiver of a method, instead of being taken: a variable keeps its
    /// value. `expected` is the type the context expects, if any.
    fn check_borrowed(&mut self, expr: &mut Expr, expected: Option<&Type>) -> Type {
        self.expected_type = expected.cloned();
        self.borrowing = true;
        self.check_expr(expr)
    }

    /// Moves the value out of the variable `name`, which can't be used
    /// afterwards. A match binding moves out of the variable it binds into.
    ///
    /// # Panics
    ///
    /// Panics if `name` only borrows its value.
    fn move_variable(&mut self, name: &str, span: &Span) {
        if self.borrowed.contains(name) {
            SemanticError::CannotMoveBorrowed { what: name.to_string(), span: span.clone() }.throw(&self.source);
        }
        self.moved.insert(name.to_string());
        if let Some(root) = self.binding_roots.get(name).cloned() {
            self.move_variable(&root, span);
        }
    }

    /// Moves the value of the field or array element `place` out, called
    /// `what` in errors. Out of a temporary like a call's result that's
    /// fine, out of a variable it moves the whole variable.
    ///
    /// # Panics
    ///
    /// Panics if the place is borrowed, like a field of `self`.
    fn move_out_of(&mut self, place: &Expr, what: &str, span: &Span) {
        let Some(root) = root_variable(place) else { return };
        let through_reference = self.scope.get(root).is_some_and(|(ty, _)| matches!(ty, Type::Ref(_) | Type::Slice(_)));
        if through_reference {
            SemanticError::CannotMoveBorrowed { what: what.to_string(), span: span.clone() }.throw(&self.source);
        }
        let root = root.to_string();
        if self.borrowed.contains(&root) {
            SemanticError::CannotMoveBorrowed { what: what.to_string(), span: span.clone() }.throw(&self.source);
        }
        // Fields move one by one, like `p.left` and then `p.right`. Which element of an
        // array moves isn't known while compiling, so the whole array counts as moved
        if what.contains("[..]") {
            self.move_variable(&root, span);
            return;
        }
        self.moved.insert(what.to_string());
        if let Some(owner) = self.binding_roots.get(&root).cloned() {
            self.move_variable(&owner, span);
        }
    }

    /// Who owns the value of `scrutinee`, for the bindings of a match on it.
    fn owner_of(&self, scrutinee: &Expr) -> Owner {
        let Some(root) = root_variable(scrutinee) else { return Owner::Temporary };
        let through_reference = self.scope.get(root).is_some_and(|(ty, _)| matches!(ty, Type::Ref(_) | Type::Slice(_)));
        if through_reference || self.borrowed.contains(root) {
            Owner::Borrowed
        } else {
            Owner::Variable(root.to_string())
        }
    }

    /// Adds the binding `name` of a pattern, `ty` its type, to the scope,
    /// owning or borrowing its value like [`Sema::pattern_owner`] says.
    fn add_binding(&mut self, name: &str, ty: Type) {
        self.scope.insert(name.to_string(), (ty, false));
        self.moved.remove(name);
        self.borrowed.remove(name);
        self.binding_roots.remove(name);
        match self.pattern_owner.clone() {
            Owner::Temporary => {}
            Owner::Variable(root) => { self.binding_roots.insert(name.to_string(), root); }
            Owner::Borrowed => { self.borrowed.insert(name.to_string()); }
        }
    }

    /// Checks `value is pattern` and adds the names of the pattern to the
    /// scope, remembering what they meant before for [`Sema::drop_is_bindings`].
    fn check_is(&mut self, value: &mut Expr, pattern: &mut Pattern, span: &Span) -> Type {
        // Only read like a match's scrutinee, a `&` is matched by the value it refers to
        let value_ty = match self.check_borrowed(value, None) {
            Type::Ref(inner) => *inner,
            other => other,
        };
        for name in pattern.names() {
            let binding = IsBinding {
                scope: self.scope.get(&name).cloned(),
                moved: self.moved.iter().filter(|path| path_root(path) == name).cloned().collect(),
                borrowed: self.borrowed.contains(&name),
                root: self.binding_roots.get(&name).cloned(),
                name,
            };
            self.is_bindings.push(binding);
        }
        self.pattern_owner = self.owner_of(value);
        self.check_pattern(pattern, &value_ty, span);
        Type::Bool
    }

    /// Takes the names bound by `is` since there were `mark` of them out of
    /// the scope again, the names they hid mean what they meant before.
    fn drop_is_bindings(&mut self, mark: usize) {
        while self.is_bindings.len() > mark {
            let binding = self.is_bindings.pop().expect("there are more than `mark`");
            let name = binding.name;
            match binding.scope {
                Some(entry) => { self.scope.insert(name.clone(), entry); }
                None => { self.scope.remove(&name); }
            }
            self.moved.retain(|path| path_root(path) != name);
            self.moved.extend(binding.moved);
            if binding.borrowed { self.borrowed.insert(name.clone()); } else { self.borrowed.remove(&name); }
            match binding.root {
                Some(root) => { self.binding_roots.insert(name, root); }
                None => { self.binding_roots.remove(&name); }
            }
        }
    }

    /// Whether `expr` creates a built-in `Buffer` like `Buffer<int32>(8)`. A user
    /// type or function with that name takes precedence.
    fn is_buffer_call(&self, expr: &Expr) -> bool {
        matches!(&expr.kind, ExprKind::FunctionCall { name, receiver: None, .. }
            if name == "Buffer"
                && !self.functions.contains_key("Buffer")
                && !self.generic_functions.contains_key("Buffer")
                && !self.generic_types.contains_key("Buffer"))
    }

    /// Checks `Buffer<T>(capacity)` and turns it into an [`ExprKind::NewBuffer`].
    ///
    /// `T` comes from the explicit type argument, else from an expected `Buffer<T>`.
    ///
    /// # Panics
    ///
    /// Panics if `T` is unknown or owns memory itself, or the capacity isn't
    /// exactly one `uint64` argument.
    fn check_buffer_call(&mut self, expr: &mut Expr, expected: Option<&Type>) -> Type {
        let span = expr.span.clone();
        let ExprKind::FunctionCall { type_args, params, .. } = &mut expr.kind else {
            unreachable!("only `Buffer` calls get here");
        };
        let element = match (type_args.as_slice(), expected) {
            ([ty], _) => self.canonicalize_type(ty, &span),
            ([], Some(Type::Buffer(element))) => (**element).clone(),
            ([], _) => SemanticError::CannotInferTypeParam {
                param: "T".to_string(),
                function: "Buffer".to_string(),
                span,
            }.throw(&self.source),
            (args, _) => SemanticError::WrongTypeArgCount {
                function: "Buffer".to_string(),
                expected: 1,
                got: args.len(),
                span,
            }.throw(&self.source),
        };
        self.check_type(&element, &span);

        if params.len() != 1 {
            SemanticError::WrongArgCount { function: "Buffer".to_string(), expected: 1, got: params.len(), span }
                .throw(&self.source);
        }
        let mut capacity = params.remove(0);
        self.check_argument(&mut capacity, &Type::UINT64, "Buffer capacity");

        let ty = Type::Buffer(Box::new(element));
        self.see_buffer(&ty);
        expr.kind = ExprKind::NewBuffer(Box::new(capacity));
        ty
    }

    /// Checks a call of the built-in `Buffer` method `method` on `receiver`, a
    /// `Buffer<T>`, and returns its result type:
    ///
    /// ```text
    /// get(index: uint64) -> &T                 the element, it stays in the Buffer
    /// take(mut &self, index: uint64) -> T      the element itself, the slot is left empty
    /// set(mut &self, index: uint64, value: T)
    /// capacity() -> uint64
    /// resize(mut &self, capacity: uint64)
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if there's no such method, the arguments don't match, or a
    /// changing method is called on an immutable variable.
    fn check_buffer_method(&mut self, method: &str, receiver: &Expr, args: &mut Vec<Expr>, span: &Span) -> Type {
        let element = match &receiver.ty {
            Type::Buffer(element) => (**element).clone(),
            Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) => match inner.as_ref() {
                Type::Buffer(element) => (**element).clone(),
                _ => unreachable!("only Buffer receivers get here"),
            },
            _ => unreachable!("only Buffer receivers get here"),
        };
        let (params, result, changes_self) = match method {
            "get" => (vec![Type::UINT64], Type::Ref(Box::new(element)), false),
            "take" => (vec![Type::UINT64], element, true),
            "set" => (vec![Type::UINT64, element], Type::Void, true),
            "capacity" => (vec![], Type::UINT64, false),
            "resize" => (vec![Type::UINT64], Type::Void, true),
            _ => SemanticError::UnknownFunction { fn_name: method.to_string(), span: span.clone() }.throw(&self.source),
        };

        if args.len() != params.len() {
            SemanticError::WrongArgCount { function: method.to_string(), expected: params.len(), got: args.len(), span: span.clone() }
                .throw(&self.source);
        }
        for (arg, ty) in args.iter_mut().zip(&params) {
            self.check_argument(arg, ty, "function argument");
        }

        if changes_self {
            if let Some(variable) = root_variable(receiver) {
                if self.scope.get(variable).is_some_and(|(_, mutable)| !mutable) {
                    SemanticError::MutatingCallOnImmutable {
                        method: method.to_string(),
                        var: variable.to_string(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
        }
        result
    }

    /// Checks `arg` for a parameter of type `ty`, adapting literals and widening
    /// numbers like for every other argument.
    ///
    /// # Panics
    ///
    /// Panics if `arg` doesn't have the type `ty` afterwards.
    fn check_argument(&mut self, arg: &mut Expr, ty: &Type, context: &'static str) {
        self.check_expr_expecting(arg, ty);
        self.coerce(arg, ty);
        if arg.ty != *ty {
            SemanticError::TypeMismatch { lhs: arg.ty.clone(), rhs: ty.clone(), span: arg.span.clone(), context }
                .throw(&self.source);
        }
    }

    /// Whether `expr` calls the built-in `unique` or `shared`, and which. A user
    /// function with that name takes precedence.
    fn heap_call(&self, expr: &Expr) -> Option<&'static str> {
        let ExprKind::FunctionCall { name, receiver: None, .. } = &expr.kind else { return None };
        let builtin = ["unique", "shared"].into_iter().find(|builtin| name == builtin)?;
        let own = self.functions.contains_key(builtin) || self.generic_functions.contains_key(builtin);
        (!own).then_some(builtin)
    }

    /// Checks a call of the built-in `unique(value)` or `shared(value)`, which
    /// `builtin` says, and turns it into an [`ExprKind::Unique`] or
    /// [`ExprKind::Shared`], which moves the value onto the heap.
    ///
    /// The value's type comes from an explicit `unique<T>(...)`, else from an
    /// expected `Unique<T>`, else from the value itself.
    ///
    /// # Panics
    ///
    /// Panics if there isn't exactly one argument or it doesn't have the type `T`.
    fn check_heap_call(&mut self, builtin: &str, expr: &mut Expr, expected: Option<&Type>) -> Type {
        let span = expr.span.clone();
        let ExprKind::FunctionCall { type_args, params, .. } = &mut expr.kind else {
            unreachable!("only calls of `unique` and `shared` get here");
        };
        if params.is_empty() {
            SemanticError::MissingArgument { function: builtin.to_string(), index: 0, span }.throw(&self.source);
        }
        if params.len() > 1 {
            SemanticError::TooManyArguments { func_name: builtin.to_string(), parent: "builtin".to_string(), span }
                .throw(&self.source);
        }
        let target = match (type_args.as_slice(), expected) {
            ([], Some(Type::Unique(inner))) if builtin == "unique" => Some((**inner).clone()),
            ([], Some(Type::Shared(inner))) if builtin == "shared" => Some((**inner).clone()),
            ([], _) => None,
            ([ty], _) => Some(self.canonicalize_type(ty, &span)),
            (args, _) => SemanticError::WrongTypeArgCount {
                function: builtin.to_string(),
                expected: 1,
                got: args.len(),
                span,
            }.throw(&self.source),
        };

        let mut value = params.remove(0);
        match &target {
            Some(ty) => {
                self.check_expr_expecting(&mut value, ty);
                self.coerce(&mut value, ty);
                if value.ty != *ty {
                    SemanticError::TypeMismatch {
                        lhs: value.ty.clone(),
                        rhs: ty.clone(),
                        span: value.span.clone(),
                        context: if builtin == "shared" { "shared value" } else { "unique value" },
                    }.throw(&self.source);
                }
            }
            None => {
                self.check_expr(&mut value);
            }
        }

        let inner = Box::new(value.ty.clone());
        let (ty, kind) = if builtin == "shared" {
            (Type::Shared(inner), ExprKind::Shared(Box::new(value)))
        } else {
            (Type::Unique(inner), ExprKind::Unique(Box::new(value)))
        };
        expr.kind = kind;
        ty
    }

    /// Returns the instances of generic functions, classes and enums created since the last call.
    ///
    /// They are ordinary declarations and have to be declared and generated
    /// like the module's own ones.
    /// The instances of generic functions, classes, enums and impls created so
    /// far. A class's or enum's only have the methods something called, see
    /// [`LazyMethod`], so this is taken once every module is checked.
    pub fn take_instances(&mut self) -> Vec<Declaration> {
        let mut instances = std::mem::take(&mut self.instances);
        instances.append(&mut self.type_instance_decls);
        instances
    }

    pub fn push_class(&mut self, name: String, fields: Vec<Field>) {
        self.classes.insert(name, fields);
    }

    pub fn push_enum(&mut self, name: String, fields: Vec<(String, u32, Type)>) {
        self.enums.insert(name, fields);
    }

    pub fn push_func(&mut self, name: String, params: Vec<Param>, return_type: Type, visibility: Visibility) {
        self.functions.insert(name, (params, return_type, visibility, self.module.clone()));
    }


    /// Runs the step `phase` of checking on the statements of a whole module.
    /// Every step runs for all modules before the next one starts, see
    /// [`Phase`]. Errors are collected, see [`Sema::take_errors`].
    pub fn check_phase(&mut self, phase: Phase, ast: &mut Vec<Stmt>) {
        // Every module's impls are known once declaring starts. Generic impls whose
        // bounds weren't met yet get another try
        if phase == Phase::Declarations && !self.impls_known {
            self.impls_known = true;
            for (index, instance_ty) in std::mem::take(&mut self.deferred_impls) {
                self.recover(|s| s.apply_generic_impl(index, &instance_ty));
            }
        }
        match phase {
            Phase::Bodies => self.check_bodies(ast),
            declaring => self.declare_phase(declaring, ast),
        }
    }

    /// Checks the code of a module whose declarations, and those of every
    /// other module, are known: its statements and function bodies, and the
    /// generic instances they create.
    fn check_bodies(&mut self, ast: &mut Vec<Stmt>) {
        // Bounds of the types this module's declarations use, now that every impl is known
        let module = self.module.clone();
        let (own, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.deferred_bounds).into_iter()
            .partition(|(_, _, _, of)| *of == module);
        self.deferred_bounds = others;
        for (ty, trait_name, span, _) in own {
            if !self.implements(&ty, &trait_name) {
                self.recover(|s| SemanticError::TraitNotImplemented { ty, trait_name, span }.throw(&s.source));
            }
        }
        // Every module's items are declared by now
        for (qualified, module, span) in self.imported_items.clone() {
            let exists = self.functions.contains_key(&qualified) || self.generic_functions.contains_key(&qualified)
                || self.classes.contains_key(&qualified) || self.enums.contains_key(&qualified)
                || self.generic_types.contains_key(&qualified) || self.traits.contains_key(&qualified)
                || self.generic_traits.contains_key(&qualified);
            if !exists {
                let item = qualified.split_once("::").map_or(qualified.clone(), |(_, item)| item.to_string());
                self.recover(|s| SemanticError::UnknownImport { item, module, span }.throw(&s.source));
            }
        }
        for stmt in ast.iter_mut() {
            self.check_stmt(stmt);
        }

        // Checking one instance or method can create more instances and call more methods
        loop {
            if let Some(pending) = self.pending_instances.pop() {
                self.recover(|s| s.check_type_instance(pending));
            } else if let Some(method) = self.used_methods.pop() {
                self.recover(|s| s.check_lazy_method(&method));
            } else {
                break;
            }
        }
        self.check_opaque_returns();

        // Literals that never got a type from their context stay int32
        let first = self.oversized_literals.drain()
            .map(|(_, literal)| literal)
            .min_by_key(|(_, span)| (span.line, span.col));
        if let Some((value, span)) = first {
            self.recover(|s| SemanticError::LiteralOutOfRange { value, ty: Type::INT32, span }.throw(&s.source));
        }
    }

    /// The errors found since the last call, in the order they were found.
    pub fn take_errors(&mut self) -> Vec<CompileError> {
        std::mem::take(&mut self.errors)
    }

    /// Runs `check`, and if it stops with an error, records the error and
    /// puts the state back to before it, so checking can go on after it.
    /// Returns `None` then.
    ///
    /// An error is left out if the failed part used something whose type is
    /// unknown because of an earlier error, it would only follow from that one.
    /// An error inside makes errors of the parts around it follow from it too.
    fn recover<T>(&mut self, check: impl FnOnce(&mut Self) -> T) -> Option<T> {
        let snapshot = Snapshot {
            scope: self.scope.clone(), is_bindings: self.is_bindings.len(), method_target: self.method_target.clone(), moved: self.moved.clone(), borrowed: self.borrowed.clone(),
            binding_roots: self.binding_roots.clone(), pattern_owner: self.pattern_owner.clone(),
            borrowing: self.borrowing, field_object: self.field_object, expected_type: self.expected_type.clone(),
            current_return_type: self.current_return_type.clone(), loop_depth: self.loop_depth,
            current_class: self.current_class.clone(), current_trait: self.current_trait.clone(),
            type_subst: self.type_subst.clone(), module: self.module.clone(), source: self.source.clone(),
            imports: self.imports(),
        };
        let outer_used_unknown = self.used_unknown.replace(false);
        let errors_before = self.errors.len();
        let result = diagnostic::catch(|| check(self));
        let error = match result {
            Ok(value) => {
                self.used_unknown.set(outer_used_unknown || self.used_unknown.get());
                return Some(value);
            }
            Err(error) => error,
        };

        let follows = self.used_unknown.get() || self.errors.len() > errors_before
            || (!self.errors.is_empty() && error.message.contains("Unknown"));
        if !follows {
            self.errors.push(error);
        }
        self.used_unknown.set(true);
        self.scope = snapshot.scope;
        self.is_bindings.truncate(snapshot.is_bindings);
        self.keep_is_bindings = false;
        // A call stopped by the error leaves its wanted type behind, it's not the next call's
        self.method_target = snapshot.method_target;
        self.method_target_used.set(false);
        self.moved = snapshot.moved;
        self.borrowed = snapshot.borrowed;
        self.binding_roots = snapshot.binding_roots;
        self.pattern_owner = snapshot.pattern_owner;
        self.borrowing = snapshot.borrowing;
        self.field_object = snapshot.field_object;
        self.expected_type = snapshot.expected_type;
        self.current_return_type = snapshot.current_return_type;
        self.loop_depth = snapshot.loop_depth;
        self.current_class = snapshot.current_class;
        self.current_trait = snapshot.current_trait;
        self.type_subst = snapshot.type_subst;
        self.module = snapshot.module;
        self.source = snapshot.source;
        self.imported_modules = snapshot.imports.imported_modules;
        self.aliases = snapshot.imports.aliases;
        self.ambiguous = snapshot.imports.ambiguous;
        None
    }

    /// Checks statements with their own declarations, like a function body:
    /// every declaring phase, then the statements.
    fn check_stmts(&mut self, ast: &mut Vec<Stmt>) {
        for phase in Phase::ALL.into_iter().filter(|phase| *phase != Phase::Bodies) {
            self.declare_phase(phase, ast);
        }
        for stmt in ast.iter_mut() {
            self.check_stmt(stmt);
        }
    }

    /// Runs the declaring step `phase` on the declarations of `ast`.
    fn declare_phase(&mut self, phase: Phase, ast: &mut Vec<Stmt>) {
        match phase {
            Phase::TypeNames => { self.recover(|s| s.declare_type_names(ast)); }
            // Types are resolved before declaring, so the declared signatures use the final types
            Phase::Signatures => for stmt in ast.iter_mut() {
                if let StmtKind::Declaration(decl) = &mut stmt.kind {
                    self.recover(|s| s.canonicalize_decl(decl));
                }
            },
            // All traits are known before any impl is checked against them
            Phase::Traits => for stmt in ast.iter() {
                let StmtKind::Declaration(decl) = &stmt.kind else { continue };
                let DeclarationKind::Trait { name, type_params, methods } = &decl.kind else { continue };
                if type_params.is_empty() {
                    self.traits.insert(name.clone(), methods.clone());
                } else {
                    self.generic_traits.insert(name.clone(), self.template(decl));
                }
            },
            // Generic impls apply to the instances of their target, also to those created already
            Phase::GenericImpls => for stmt in ast.iter() {
                if let StmtKind::Declaration(decl) = &stmt.kind {
                    if matches!(&decl.kind, DeclarationKind::Impl { type_params, .. } if !type_params.is_empty()) {
                        self.recover(|s| s.register_generic_impl(decl));
                    }
                }
            },
            Phase::Impls => for stmt in ast.iter_mut() {
                if let StmtKind::Declaration(decl) = &mut stmt.kind {
                    if matches!(&decl.kind, DeclarationKind::Impl { type_params, .. } if type_params.is_empty()) {
                        self.recover(|s| s.complete_impl(decl));
                    }
                }
            },
            Phase::Declarations => for stmt in ast.iter_mut() {
                if let StmtKind::Declaration(decl) = &mut stmt.kind {
                    self.recover(|s| s.declare_decl(decl));
                }
            },
            Phase::Validation => for stmt in ast.iter_mut() {
                if let StmtKind::Declaration(decl) = &stmt.kind {
                    self.recover(|s| s.validate_decl_types(decl, &stmt.span));
                }
            },
            Phase::Bodies => unreachable!("bodies aren't declarations"),
        }
    }

    /// Registers the names of all classes and enums declared in `ast`, so types
    /// referring to them can be resolved before the declarations are processed.
    ///
    /// [`Sema::declare_decl`] fills in their fields and variants later. Generic
    /// classes and enums are registered as templates for their instances.
    fn declare_type_names(&mut self, ast: &[Stmt]) {
        for stmt in ast {
            if let StmtKind::Declaration(decl) = &stmt.kind {
                match &decl.kind {
                    DeclarationKind::Class { name, type_params, .. } | DeclarationKind::Enum { name, type_params, .. }
                        if !type_params.is_empty() =>
                    {
                        self.generic_types.insert(name.clone(), GenericDecl {
                            decl: decl.clone(),
                            module: self.module.clone(),
                            source: self.source.clone(),
                            imports: self.imports(),
                            class: None,
                            outer_subst: HashMap::new(),
                        });
                    }
                    // Literals and the operators on strings use the prelude's `String`
                    DeclarationKind::Class { name, .. } if name == "String" && self.module != "prelude" => {
                        SemanticError::BuiltinRedeclared { name: name.clone(), span: decl.span.clone() }.throw(&self.source);
                    }
                    DeclarationKind::Class { name, .. } => { self.classes.entry(name.clone()).or_default(); }
                    DeclarationKind::Enum { name, .. } => { self.enums.entry(name.clone()).or_default(); }
                    DeclarationKind::Extern(ExternItem::Type { name, base, .. }) => {
                        self.handles.insert(name.clone());
                        if let Some(base) = base {
                            self.handle_bases.insert(name.clone(), base.clone());
                        }
                    }
                    DeclarationKind::Function { .. } | DeclarationKind::Trait { .. } | DeclarationKind::Impl { .. }
                        | DeclarationKind::Extern(ExternItem::Link(_)) => {}
                }
            }
        }
    }

    /// Checks an impl block against its trait and adds the trait's default
    /// methods it doesn't override.
    ///
    /// # Panics
    ///
    /// Panics if the trait is unknown, the target can't have methods, the trait is
    /// already implemented for the target, or the methods don't match the
    /// trait's: a method the trait doesn't have, a different signature, or a
    /// missing method without default body.
    fn complete_impl(&mut self, decl: &mut Declaration) {
        let span = decl.span.clone();
        let DeclarationKind::Impl { trait_name, trait_args, target, methods, .. } = &mut decl.kind else { return };

        // `impl Iterator<int32> for ...` implements the instance `Iterator<int32>`
        *trait_name = self.trait_instance(trait_name, trait_args, &span);
        trait_args.clear();
        let trait_methods = self.traits[trait_name.as_str()].clone();
        let type_name = target.owner_name()
            .unwrap_or_else(|| SemanticError::InvalidImplTarget { ty: target.clone(), span: span.clone() }.throw(&self.source));
        if self.impls.get(&type_name).is_some_and(|traits| traits.contains(trait_name)) {
            SemanticError::DuplicateImpl { trait_name: trait_name.clone(), ty: type_name, span }.throw(&self.source);
        }

        // The parser can't type `self` for built-in targets, so every method gets the target here
        for method in methods.iter_mut() {
            if let DeclarationKind::Function { params, .. } = &mut method.kind {
                for p in params.iter_mut().filter(|p| p.name == "self") {
                    p.ty = target.clone();
                }
            }
        }

        // Every method of the impl has to be declared by the trait, with the same signature
        for method in methods.iter() {
            let method_name = function_name(method);
            let trait_method = trait_methods.iter().find(|m| function_name(m) == method_name)
                .unwrap_or_else(|| SemanticError::NotATraitMethod {
                    method: method_name.to_string(),
                    trait_name: trait_name.clone(),
                    span: method.span.clone(),
                }.throw(&self.source));
            // `-> impl Iterator<T>` in the trait takes any type implementing that trait
            let mut expected = trait_method.kind.clone();
            // `Self` in the trait is this impl's target
            self.fill_in_self(&mut expected, target, &span);
            if let (
                DeclarationKind::Function { return_type: Some(Type::Opaque(bound)), .. },
                DeclarationKind::Function { return_type: Some(returned), .. },
            ) = (&trait_method.kind, &method.kind) {
                let (bound_name, bound_args) = match bound.as_ref() {
                    Type::Generic { name, args } => (name.clone(), args.clone()),
                    Type::Class(name) | Type::Object(name) => (name.clone(), vec![]),
                    other => diagnostic::throw(format!("'impl' needs a trait, '{}' isn't one", other), Some(trait_method.span.clone()), &self.source),
                };
                let bound_trait = self.trait_instance(&bound_name, &bound_args, &trait_method.span);
                self.opaque_returns.push(OpaqueReturn {
                    ty: returned.clone(),
                    trait_name: bound_trait,
                    method: method_name.to_string(),
                    span: method.span.clone(),
                    source: self.source.clone(),
                });
                if let DeclarationKind::Function { return_type, .. } = &mut expected {
                    *return_type = Some(returned.clone());
                }
            }
            if !same_signature(&expected, &method.kind) {
                SemanticError::TraitMethodMismatch {
                    method: method_name.to_string(),
                    trait_name: trait_name.clone(),
                    span: method.span.clone(),
                }.throw(&self.source);
            }
        }

        // Methods the impl doesn't have come from the trait's default bodies
        for trait_method in &trait_methods {
            let method_name = function_name(trait_method);
            if methods.iter().any(|m| function_name(m) == method_name) {
                continue;
            }
            let DeclarationKind::Function { body: Some(_), .. } = &trait_method.kind else {
                SemanticError::MissingTraitMethod {
                    method: method_name.to_string(),
                    trait_name: trait_name.clone(),
                    ty: type_name.clone(),
                    span: span.clone(),
                }.throw(&self.source)
            };

            let mut copy = trait_method.clone();
            copy.visibility = Visibility::Public;
            self.fill_in_self(&mut copy.kind, target, &span);
            if let DeclarationKind::Function { params, parent, visibility, .. } = &mut copy.kind {
                // In the trait `self` stands for any implementing type, here it's the target
                for p in params.iter_mut().filter(|p| p.name == "self") {
                    p.ty = target.clone();
                }
                *parent = Some(type_name.clone());
                *visibility = Visibility::Public;
            }
            methods.push(copy);
        }

        self.impls.entry(type_name).or_default().push(trait_name.clone());
    }

    /// Checks that every type an impl method returns for `impl Trait` implements
    /// that trait. Done once all impls of the module are known, so the order
    /// of the impls doesn't matter.
    fn check_opaque_returns(&mut self) {
        for opaque in std::mem::take(&mut self.opaque_returns) {
            if !self.implements(&opaque.ty, &opaque.trait_name) {
                let ty = opaque.ty.owner_name().unwrap_or_else(|| opaque.ty.to_string());
                self.recover(|_| diagnostic::throw(
                    format!("'{}' returns '{}', which doesn't implement '{}'", opaque.method, ty, opaque.trait_name),
                    Some(opaque.span.clone()),
                    &opaque.source,
                ));
            }
        }
    }

    /// `decl` as a template for its instances, checked later in the current module.
    fn template(&self, decl: &Declaration) -> GenericDecl {
        GenericDecl {
            decl: decl.clone(),
            module: self.module.clone(),
            source: self.source.clone(),
            imports: self.imports(),
            class: None,
            outer_subst: HashMap::new(),
        }
    }

    /// The name of the trait `name` with the type arguments `args`. For a
    /// generic trait that's its instance like `Iterator<int32>`, created the
    /// first time with the type parameters replaced in its method signatures.
    ///
    /// # Panics
    ///
    /// Panics if the trait is unknown, or the number of type arguments is wrong.
    /// Like [`Sema::has_method`], but only a method taking `self` counts. A
    /// function without, like `try_as` of an impl for the receiver's type, doesn't.
    fn has_self_method(&mut self, receiver: &Expr, name: &str, span: &Span) -> bool {
        let ty = peel(self.probe_type(receiver));
        let Some(owner) = ty.owner_name() else { return self.has_method(receiver, name, span) };
        let method = self.find_method(&owner, name, span);
        self.functions.get(&method).is_some_and(|(params, ..)| params.first().is_some_and(|p| p.name == "self"))
            || self.generic_functions.contains_key(&method)
    }

    /// Whether the type of `receiver` has the method `name` from several impls
    /// of one trait, like `try_as` of `TryAs<int32>` and `TryAs<bool>` for
    /// `String`, so the type a call should give picks one.
    fn has_impls_by_type(&mut self, receiver: &Expr, name: &str) -> bool {
        let ty = peel(self.probe_type(receiver));
        let Some(owner) = ty.owner_name() else { return false };
        self.impls.get(&owner).into_iter().flatten()
            .filter(|tr| self.functions.contains_key(&format!("{}_{}_{}", owner, tr, name)))
            .count() > 1
    }

    /// Whether some trait has a function `name` without `self`, like `try_as`
    /// of `TryAs`, which is called on the value it takes: `text.try_as<int32>()`.
    fn is_static_trait_function(&self, name: &str) -> bool {
        self.traits.values().flatten().any(|m| matches!(&m.kind,
            DeclarationKind::Function { name: n, params, .. } if n == name && params.first().is_none_or(|p| p.name != "self")))
    }

    /// The mangled name of the trait function `name` of the impl for `target`
    /// whose first parameter takes a `from`, like `int32_TryAs<String>_try_as`.
    fn static_trait_impl(&self, name: &str, from: &Type, target: &Type) -> Option<String> {
        let owner = target.owner_name()?;
        self.impls.get(&owner)?.iter()
            .map(|tr| format!("{}_{}_{}", owner, tr, name))
            .find(|mangled| self.functions.get(mangled).is_some_and(|(params, ..)| match params.first() {
                Some(p) if p.name != "self" => p.ty == *from || matches!(&p.ty, Type::Ref(inner) if **inner == *from),
                _ => false,
            }))
    }

    /// The instance of the generic enum `template` in `expected`: `expected`
    /// itself, or the value of an `Option` or `Result` around it, like
    /// `Result<int32, String>` in `Option<Result<int32, String>>`.
    fn expected_instance_of(&self, template: &str, expected: Option<&Type>) -> Option<Type> {
        let mut ty = expected?.clone();
        loop {
            let Type::Enum(instance) = &ty else { return None };
            if self.type_instances.get(instance)?.0 == template {
                return Some(ty);
            }
            ty = self.result_types(&ty).map(|(ok, _)| ok).or_else(|| self.option_value_type(&ty))?;
        }
    }

    /// Whether `ty` is an `Option` of an extern type, like `Option<LLVMValueRef>`.
    /// C passes it as a pointer that's null for `None`.
    fn nullable_handle(&self, ty: &Type) -> bool {
        self.option_value_type(ty).is_some_and(|value| matches!(value, Type::Handle(_)))
    }

    /// Whether `&ty` is an out-parameter of a C function: an `Option` of an
    /// extern type C writes a handle into, or an `Option<String>` C writes a
    /// `char *` into, which becomes a copy.
    fn is_out_param(&self, ty: &Type) -> bool {
        self.nullable_handle(ty) || self.option_value_type(ty).is_some_and(|value| value.is_string())
    }

    /// The `T` and `E` of `Result<T, E>`, if `ty` is an instance of `Result`.
    fn result_types(&self, ty: &Type) -> Option<(Type, Type)> {
        let Type::Enum(instance) = ty else { return None };
        let (template, args) = self.type_instances.get(instance)?;
        (template.ends_with("Result") && args.len() == 2).then(|| (args[0].clone(), args[1].clone()))
    }

    /// The `T` of `Option<T>`, if `ty` is an instance of `Option`.
    fn option_value_type(&self, ty: &Type) -> Option<Type> {
        let Type::Enum(instance) = ty else { return None };
        let (template, args) = self.type_instances.get(instance)?;
        (template.ends_with("Option") && args.len() == 1).then(|| args[0].clone())
    }

    /// Replaces `Self` in the signature of the trait function `function` by
    /// `target`, the type of the impl it's checked against or copied into.
    fn fill_in_self(&mut self, function: &mut DeclarationKind, target: &Type, span: &Span) {
        let DeclarationKind::Function { params, return_type, .. } = function else { return };
        for p in params.iter_mut().filter(|p| p.name != "self" && has_self_type(&p.ty)) {
            p.ty = self.canonicalize_type(&replace_self(&p.ty, target), span);
        }
        if let Some(ty) = return_type.as_mut().filter(|ty| has_self_type(ty)) {
            *ty = self.canonicalize_type(&replace_self(ty, target), span);
        }
    }

    fn trait_instance(&mut self, name: &str, args: &[Type], span: &Span) -> String {
        // A trait the module imports is that one, even if another module declares one with its name
        let known = |s: &Self, name: &str| s.traits.contains_key(name) || s.generic_traits.contains_key(name);
        let name = match self.aliases.get(name) {
            Some(imported) => imported.clone(),
            None if known(self, name) => name.to_string(),
            None => self.resolve_name(name, span),
        };
        let Some(generic) = self.generic_traits.get(&name).cloned() else {
            if !self.traits.contains_key(&name) {
                SemanticError::UnknownTrait { name, span: span.clone() }.throw(&self.source);
            }
            if !args.is_empty() {
                SemanticError::WrongTypeArgCount { function: name, expected: 0, got: args.len(), span: span.clone() }.throw(&self.source);
            }
            return name;
        };
        let DeclarationKind::Trait { type_params, methods, .. } = &generic.decl.kind else {
            unreachable!("generic traits are trait declarations");
        };
        if args.len() != type_params.len() {
            SemanticError::WrongTypeArgCount { function: name, expected: type_params.len(), got: args.len(), span: span.clone() }
                .throw(&self.source);
        }

        let args: Vec<Type> = args.iter().map(|arg| self.canonicalize_type(arg, span)).collect();
        let subst: HashMap<String, Type> = type_params.iter().map(|p| p.name.clone()).zip(args).collect();
        let instance = instance_name(&name, type_params, &subst);
        if self.traits.contains_key(&instance) {
            return instance;
        }
        let mut methods = methods.clone();
        self.in_template_context(subst.clone(), &generic, |sema| {
            for method in methods.iter_mut() {
                sema.canonicalize_decl(method);
            }
        });
        self.traits.insert(instance.clone(), methods);
        self.trait_substs.insert(instance.clone(), subst);
        instance
    }

    /// Registers the generic impl `decl`, like `impl<T> Iterator<T> for VectorIter<T>`,
    /// and applies it to the instances of its target that exist already.
    ///
    /// # Panics
    ///
    /// Panics if the target isn't an instance of a generic class or enum.
    fn register_generic_impl(&mut self, decl: &Declaration) {
        let mut decl = decl.clone();
        let DeclarationKind::Impl { target, .. } = &mut decl.kind else { return };
        // The target is written with the type parameters, like `VectorIter<T>`
        let Type::Generic { name, .. } = target else {
            diagnostic::throw(format!("a generic impl needs a generic target like `Box<T>`, '{}' isn't one", target), Some(decl.span.clone()), &self.source)
        };
        // The built-in `Buffer<T>`, like `impl<T> Iterable<T> for Buffer<T>`
        if name == "Buffer" && !self.generic_types.contains_key("Buffer") {
            let index = self.generic_impls.len();
            self.generic_impls.push(self.template(&decl));
            for buffer in self.buffer_types.clone() {
                self.apply_generic_impl(index, &buffer);
            }
            return;
        }
        if !self.generic_types.contains_key(name.as_str()) {
            *name = self.resolve_name(name, &decl.span);
        }
        if !self.generic_types.contains_key(name.as_str()) {
            SemanticError::UnknownType { name: name.clone(), span: decl.span.clone() }.throw(&self.source);
        }
        let template = name.clone();

        let index = self.generic_impls.len();
        self.generic_impls.push(self.template(&decl));
        let existing: Vec<String> = self.type_instances.iter()
            .filter(|(_, (of, _))| *of == template)
            .map(|(instance, _)| instance.clone())
            .collect();
        for instance in existing {
            let instance_ty = self.instance_type(&instance);
            self.apply_generic_impl(index, &instance_ty);
        }
    }

    /// The type of the instance of a generic class or enum called `instance`.
    fn instance_type(&self, instance: &str) -> Type {
        if self.enums.contains_key(instance) { Type::Enum(instance.to_string()) } else { Type::Class(instance.to_string()) }
    }

    /// Applies every generic impl of the template of `instance` to it.
    fn apply_generic_impls(&mut self, instance: &str) {
        let Some((template, _)) = self.type_instances.get(instance).cloned() else { return };
        let instance_ty = self.instance_type(instance);
        self.apply_generic_impls_of(&template, &instance_ty);
    }

    /// Applies every generic impl whose target is written with `template`, like
    /// `Box<T>`, to `instance_ty`, an instance of it.
    fn apply_generic_impls_of(&mut self, template: &str, instance_ty: &Type) {
        let matching: Vec<usize> = self.generic_impls.iter().enumerate()
            .filter(|(_, generic)| matches!(&generic.decl.kind,
                DeclarationKind::Impl { target: Type::Generic { name, .. }, .. } if name == template))
            .map(|(index, _)| index)
            .collect();
        for index in matching {
            self.apply_generic_impl(index, instance_ty);
        }
    }

    /// Notes the `Buffer` type `ty` and applies the generic impls for
    /// `Buffer<T>` to it the first time it shows up. Anything else, or a
    /// `Buffer` whose element type is still a type parameter, is ignored.
    fn see_buffer(&mut self, ty: &Type) {
        let Type::Buffer(element) = ty else { return };
        if contains_type_param(element) || matches!(element.as_ref(), Type::Unknown) || self.buffer_types.contains(ty) {
            return;
        }
        if self.generic_types.contains_key("Buffer") {
            return;
        }
        self.buffer_types.push(ty.clone());
        self.apply_generic_impls_of("Buffer", ty);
    }

    /// Creates the impl of the generic impl at `index` for `instance_ty`, an
    /// instance of a generic class or enum or a `Buffer`, like
    /// `impl Iterator<int32> for VectorIter<int32>`, declares its methods and
    /// checks its bodies at the end of the module.
    ///
    /// An impl whose type parameters have bounds the instance's type arguments
    /// don't meet doesn't apply, so `impl<T: Show> Show for Box<T>` only gives
    /// `Box<int32>` a `show` if `int32` has one.
    ///
    /// # Panics
    ///
    /// Panics if a type parameter of the impl doesn't appear in its target,
    /// or the impl doesn't match its trait.
    fn apply_generic_impl(&mut self, index: usize, instance_ty: &Type) {
        if !self.applied_impls.insert((index, instance_ty.to_string())) {
            return;
        }
        let generic = self.generic_impls[index].clone();
        let mut decl = generic.decl.clone();
        let DeclarationKind::Impl { type_params, target, .. } = &mut decl.kind else { return };

        // `VectorIter<T>` against the instance `VectorIter<int32>` gives `T = int32`
        let mut subst = HashMap::new();
        self.bind_type_params(target, instance_ty, &mut subst);
        for param in type_params.iter() {
            let Some(ty) = subst.get(&param.name) else {
                diagnostic::throw(
                    format!("type parameter '{}' of the impl has to appear in its target", param.name),
                    Some(decl.span.clone()),
                    &generic.source,
                )
            };
            if !param.bounds.iter().all(|bound| self.implements(ty, bound)) {
                // Tried again once every module's impls are known, see `Sema::check_phase`
                if !self.impls_known {
                    self.applied_impls.remove(&(index, instance_ty.to_string()));
                    self.deferred_impls.push((index, instance_ty.clone()));
                }
                return;
            }
        }
        type_params.clear();

        let span = decl.span.clone();
        self.in_template_context(subst.clone(), &generic, |sema| {
            sema.canonicalize_decl(&mut decl);
            sema.complete_impl(&mut decl);
            sema.declare_decl(&mut decl);
            sema.validate_decl_types(&decl, &span);
        });
        self.pending_instances.push(PendingInstance {
            decl,
            subst,
            module: generic.module,
            source: generic.source,
            imports: generic.imports,
        });
    }

    fn canonicalize_type(&mut self, ty: &Type, span: &Span) -> Type {
        match ty {
            // An imported type like `Args` of `import std::env::{Args};` is `env::Args`
            Type::Generic { name, args } if self.aliases.contains_key(name) => {
                let name = self.resolve_name(name, span);
                self.canonicalize_type(&Type::Generic { name, args: args.clone() }, span)
            }
            // The parser can't tell classes and enums apart, both come in as `Class`
            Type::Object(name) | Type::Class(name) => {
                let known = self.classes.contains_key(name) || self.enums.contains_key(name) || self.handles.contains(name);
                // `byte` is another name for `uint8`, unless a type is called like that
                if name == "byte" && !known && !self.generic_types.contains_key(name) {
                    return Type::Int { bits: 8, signed: false };
                }
                // `Self` in a trait is the type implementing it, filled in by its impls
                if name == "Self" && !known {
                    return self.type_subst.get("Self").cloned().unwrap_or_else(|| Type::Param("Self".to_string()));
                }
                // A type the module imports is that one, even if another module declares one with its name
                let name = &match self.aliases.get(name) {
                    Some(imported) => imported.clone(),
                    None if known => name.clone(),
                    None => self.resolve_name(name, span),
                };
                if self.handles.contains(name) {
                    Type::Handle(name.clone())
                } else if self.enums.contains_key(name) {
                    Type::Enum(name.clone())
                } else if self.classes.contains_key(name) {
                    Type::Class(name.clone())
                } else if self.generic_types.contains_key(name) {
                    SemanticError::MissingTypeArgs { name: self.written_name(name), span: span.clone() }.throw(&self.source)
                } else {
                    SemanticError::UnknownType {
                        name: self.written_name(name),
                        span: span.clone(),
                    }.throw(&self.source)
                }
            }
            Type::Ref(inner) => Type::Ref(Box::new(self.canonicalize_type(inner, span))),
            // Inside a generic instance, `T` stands for its type argument
            Type::Param(name) => self.type_subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
            Type::Generic { name, args } => {
                let args: Vec<Type> = args.iter().map(|arg| self.canonicalize_type(arg, span)).collect();
                // The built-in `Unique<T>`, `Shared<T>` and `Buffer<T>`, unless a user type has the same name
                if (name == "Unique" || name == "Shared" || name == "Buffer") && !self.generic_types.contains_key(name) {
                    if args.len() != 1 {
                        SemanticError::WrongTypeArgCount {
                            function: name.clone(),
                            expected: 1,
                            got: args.len(),
                            span: span.clone(),
                        }.throw(&self.source);
                    }
                    let arg = Box::new(args[0].clone());
                    let ty = match name.as_str() {
                        "Unique" => Type::Unique(arg),
                        "Shared" => Type::Shared(arg),
                        _ => Type::Buffer(arg),
                    };
                    self.see_buffer(&ty);
                    return ty;
                }
                // Inside a generic template the arguments can still be type parameters
                if args.iter().any(contains_type_param) {
                    return Type::Generic { name: name.clone(), args };
                }
                self.instantiate_type(name, args, span)
            }
            Type::Unique(inner) => Type::Unique(Box::new(self.canonicalize_type(inner, span))),
            Type::Shared(inner) => Type::Shared(Box::new(self.canonicalize_type(inner, span))),
            Type::Buffer(inner) => {
                let ty = Type::Buffer(Box::new(self.canonicalize_type(inner, span)));
                self.see_buffer(&ty);
                ty
            }
            Type::Array(element, len) => Type::Array(Box::new(self.canonicalize_type(element, span)), *len),
            Type::Slice(element) => Type::Slice(Box::new(self.canonicalize_type(element, span))),
            Type::Tuple(items) => Type::Tuple(items.iter().map(|item| self.canonicalize_type(item, span)).collect()),
            // The trait of `impl Iterator<T>` stays a name, only its type arguments are types
            Type::Opaque(bound) => Type::Opaque(Box::new(match bound.as_ref() {
                Type::Generic { name, args } => Type::Generic {
                    name: name.clone(),
                    args: args.iter().map(|arg| self.canonicalize_type(arg, span)).collect(),
                },
                other => other.clone(),
            })),
            other => other.clone(),
        }
    }

    /// Like [`Sema::canonicalize_type`], but an error is recorded and the type
    /// becomes `Unknown`, so the rest of the declaration is still usable.
    fn canonicalize_or_unknown(&mut self, ty: &Type, span: &Span) -> Type {
        self.recover(|s| s.canonicalize_type(ty, span)).unwrap_or(Type::Unknown)
    }

    fn canonicalize_decl(&mut self, decl: &mut Declaration) {
        match &mut decl.kind {
            // Generic templates are canonicalized per instance, see `instantiate_type`
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => {}
            DeclarationKind::Class { fields, methods, .. } => {
                for f in fields.iter_mut() {
                    let span = f.ty_span.clone();
                    f.ty = self.canonicalize_or_unknown(&f.ty, &span);
                }
                for m in methods.iter_mut() {
                    self.canonicalize_decl(m);
                }
            }
            DeclarationKind::Enum { name, fields, methods, .. } => {
                for (_, _, ty, ty_span) in fields.iter_mut() {
                    *ty = self.canonicalize_or_unknown(ty, ty_span);
                }
                for m in methods.iter_mut() {
                    // The parser can't tell an enum's `self` from a class's
                    if let DeclarationKind::Function { params, .. } = &mut m.kind {
                        for p in params.iter_mut().filter(|p| p.name == "self") {
                            p.ty = Type::Enum(name.clone());
                        }
                    }
                    self.canonicalize_decl(m);
                }
            }
            DeclarationKind::Function { params, return_type, .. } => {
                // `self` is typed by the parser already, in traits it isn't a real type
                for p in params.iter_mut().filter(|p| p.name != "self") {
                    let span = p.ty_span.clone();
                    p.ty = self.canonicalize_or_unknown(&p.ty, &span);
                }
                if let Some(rt) = return_type.as_mut() {
                    let span = decl.span.clone();
                    *rt = self.canonicalize_or_unknown(rt, &span);
                }
            }
            // Generic traits and impls are canonicalized per instance, see
            // `trait_instance` and `apply_generic_impl`
            DeclarationKind::Trait { type_params, .. } | DeclarationKind::Impl { type_params, .. }
                if !type_params.is_empty() => {}
            DeclarationKind::Trait { methods, .. } => {
                for m in methods.iter_mut() {
                    self.canonicalize_decl(m);
                }
            }
            DeclarationKind::Impl { target, methods, .. } => {
                *target = self.canonicalize_type(target, &decl.span);
                // `Self` in the impl's methods is its target
                let outer_self = self.type_subst.insert("Self".to_string(), target.clone());
                for m in methods.iter_mut() {
                    self.canonicalize_decl(m);
                }
                match outer_self {
                    Some(outer) => self.type_subst.insert("Self".to_string(), outer),
                    None => self.type_subst.remove("Self"),
                };
            }
            DeclarationKind::Extern(ExternItem::Type { name, methods, .. }) => {
                for m in methods.iter_mut() {
                    // The parser gives `self` a class type, it's the handle
                    if let DeclarationKind::Function { params, .. } = &mut m.kind {
                        for p in params.iter_mut().filter(|p| p.name == "self") {
                            p.ty = Type::Handle(name.clone());
                        }
                    }
                    self.canonicalize_decl(m);
                }
            }
            DeclarationKind::Extern(ExternItem::Link(_)) => {}
        }
    }

    fn check_type(&self, ty: &Type, span: &Span) {
        match ty {
            Type::Class(name) => {
                if !self.classes.contains_key(name) {
                    SemanticError::UnknownType {
                        name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
            Type::Enum(name) => {
                if !self.enums.contains_key(name) {
                    SemanticError::UnknownEnum {
                        name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
            Type::Object(name) => {
                SemanticError::UnknownType {
                    name: name.clone(),
                    span: span.clone(),
                }.throw(&self.source);
            }
            Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) | Type::Buffer(inner) | Type::Array(inner, _) | Type::Slice(inner) => self.check_type(inner, span),
            Type::Tuple(items) => items.iter().for_each(|item| self.check_type(item, span)),
            _ => {}
        }
    }

    fn validate_decl_types(&self, decl: &Declaration, span: &Span) {
        match &decl.kind {
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => {}
            DeclarationKind::Class { fields, methods, .. } => {
                for f in fields {
                    self.check_type(&f.ty, &f.ty_span);
                }
                for m in methods {
                    self.validate_decl_types(m, span);
                }
            }
            DeclarationKind::Enum { fields, methods, .. } => {
                for (_, _, ty, ty_span) in fields {
                    if *ty != Type::Void {
                        self.check_type(ty, ty_span);
                    }
                }
                for m in methods {
                    self.validate_decl_types(m, span);
                }
            }
            DeclarationKind::Function { params, return_type, .. } => {
                for p in params {
                    self.check_type(&p.ty, &p.ty_span);
                }
                if let Some(rt) = return_type {
                    self.check_type(rt, span);
                }
            }
            // Trait methods are checked through the impls they get copied into
            DeclarationKind::Trait { .. } | DeclarationKind::Extern(ExternItem::Link(_)) => {}
            DeclarationKind::Impl { type_params, .. } if !type_params.is_empty() => {}
            DeclarationKind::Impl { methods, .. } => {
                for m in methods {
                    self.validate_decl_types(m, span);
                }
            }
            DeclarationKind::Extern(ExternItem::Type { name, base, methods }) => {
                // Only an extern type is the same kind of pointer, and the kinds can't go in a circle
                if let Some(base) = base {
                    if !self.handles.contains(base) {
                        diagnostic::throw(format!("'{}' can only be a kind of an extern type, '{}' isn't one", name, base), Some(span.clone()), &self.source);
                    }
                    if self.is_kind_of(base, name) {
                        diagnostic::throw(format!("'{}' and '{}' can't be kinds of each other", name, base), Some(span.clone()), &self.source);
                    }
                }
                for m in methods {
                    self.validate_decl_types(m, span);
                }
            }
        }
    }

    fn declare_decl(&mut self, decl: &mut Declaration) {
        if matches!(&decl.kind, DeclarationKind::Function { type_params, .. } if !type_params.is_empty()) {
            self.declare_generic(decl);
            return;
        }

        match &mut decl.kind {
            // Registered as templates already in `declare_type_names`
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => {}
            DeclarationKind::Class { name, fields, methods, .. } => {
                self.push_class(name.clone(), fields.clone());
                // Its constructors are collected while its methods are declared
                self.constructors.remove(name.as_str());
                let prev_class = self.current_class.take();
                self.current_class = Some(name.clone());
                for method in methods.iter_mut() {
                    self.declare_decl(method);
                }
                self.current_class = prev_class;
            }
            DeclarationKind::Enum { name, fields, methods, .. } => {
                let stripped = fields.iter()
                    .map(|(n, i, t, _)| (n.clone(), *i, t.clone()))
                    .collect();
                self.push_enum(name.clone(), stripped);
                // Methods are mangled under the enum's name, like a class's
                let prev_class = self.current_class.replace(name.clone());
                for method in methods.iter_mut() {
                    self.declare_decl(method);
                }
                self.current_class = prev_class;
            },
            DeclarationKind::Function { name, params, return_type, mangled, visibility, external: Some(external), .. } => {
                // Only values C has too can be passed, a `&Buffer` or slice as the address of its elements
                for p in params.iter() {
                    let passable = match &p.ty {
                        Type::Int { .. } | Type::Float { .. } | Type::Char | Type::Handle(_) => true,
                        ty if ty.is_string() => true,
                        // An `Option` of an extern type is a pointer that's null for `None`
                        ty if self.nullable_handle(ty) => true,
                        // A char is a C `char`, so a `&Buffer<char>` is a `char *`. An `&Option` of an
                        // extern type is an out-parameter, a pointer C writes a handle through
                        Type::Ref(inner) => matches!(inner.as_ref(), Type::Buffer(element) if element.is_number() || **element == Type::Char)
                            || self.is_out_param(inner),
                        Type::Slice(element) => element.is_number() || matches!(element.as_ref(), Type::Char | Type::Handle(_)),
                        _ => false,
                    };
                    if !passable {
                        SemanticError::ExternType { ty: p.ty.clone(), span: p.ty_span.clone() }.throw(&self.source);
                    }
                }
                let ret = return_type.clone().unwrap_or(Type::Void);
                if !(ret.is_number() || ret == Type::Void || ret.is_string() || matches!(ret, Type::Handle(_)) || self.nullable_handle(&ret)) {
                    SemanticError::ExternReturnType { ty: ret, span: decl.span.clone() }.throw(&self.source);
                }
                self.push_func(name.clone(), params.clone(), ret, visibility.clone());
                self.externs.insert(name.clone(), external.clone());
                *mangled = Some(name.clone());
            }
            DeclarationKind::Function { name, params, return_type, mangled, visibility, .. } => {
                let is_ctor = self.current_trait.is_none()
                    && matches!(&self.current_class, Some(cls) if cls == name);
                if is_ctor {
                    *return_type = Some(Type::Class(name.clone()));
                }
                let ret = return_type.clone().unwrap_or(Type::Void);
                // Trait methods include the trait, so two traits can have methods with the same name
                let m = match (&self.current_class, &self.current_trait) {
                    (Some(cls), Some(tr)) => format!("{}_{}_{}", cls, tr, name),
                    (Some(cls), None) if cls != name => format!("{}_{}", cls, name),
                    // A class's first constructor has its name, the others a number after it
                    _ if is_ctor => {
                        let span = decl.span.clone();
                        self.add_constructor(name, params, &span)
                    }
                    _ => name.clone(),
                };
                self.push_func(m.clone(), params.clone(), ret, visibility.clone());
                *mangled = Some(m);
            }
            DeclarationKind::Trait { .. } | DeclarationKind::Extern(ExternItem::Link(_)) => {}
            // A generic impl's methods are declared for each instance, see `apply_generic_impl`
            DeclarationKind::Impl { type_params, .. } if !type_params.is_empty() => {}
            DeclarationKind::Impl { trait_name, target, methods, .. } => {
                let prev_class = self.current_class.take();
                self.current_class = target.owner_name();
                self.current_trait = Some(trait_name.clone());
                for method in methods.iter_mut() {
                    self.declare_decl(method);
                }
                self.current_trait = None;
                self.current_class = prev_class;
            }
            // Mangled under the type's name like a class's methods, `LLVMModuleRef_dump`
            DeclarationKind::Extern(ExternItem::Type { name, methods, .. }) => {
                let prev_class = self.current_class.replace(name.clone());
                for method in methods.iter_mut() {
                    self.declare_decl(method);
                }
                self.current_class = prev_class;
            }
        }
    }

    /// What `owner::default()` calls, with its type: the `default` of the
    /// owner's `impl Default`, or its constructor taking no arguments. `owner`
    /// is a type, like `T` in a generic or `Bucket`. `None` if it's no type,
    /// like a module whose function is called `default`.
    ///
    /// # Panics
    ///
    /// Panics if the type has no default value.
    fn default_call(&self, owner: &str, span: &Span) -> Option<(String, Type)> {
        // A type's own function called `default` is called like any other
        if self.static_function(owner, "default").is_some() {
            return None;
        }
        let ty = match self.type_subst.get(owner) {
            Some(ty) => ty.clone(),
            None => {
                let name = self.aliases.get(owner).cloned().unwrap_or_else(|| owner.to_string());
                if self.enums.contains_key(&name) {
                    Type::Enum(name)
                } else if self.classes.contains_key(&name) {
                    Type::Class(name)
                } else {
                    return None;
                }
            }
        };
        let owner_name = ty.owner_name()?;
        let from_impl = format!("{}_{}_default", owner_name, DEFAULT_TRAIT);
        if let Some((_, ret, ..)) = self.functions.get(&from_impl) {
            return Some((from_impl, ret.clone()));
        }
        if let Some(constructor) = self.default_constructor(&ty) {
            return Some((constructor, ty));
        }
        diagnostic::throw(
            format!("'{}' has no default value, give it a constructor taking no arguments or an `impl Default`",
                crate::analysis::show_type(&ty)),
            Some(span.clone()),
            &self.source,
        )
    }

    /// The constructor of the class `ty` that takes no arguments, if it has one.
    fn default_constructor(&self, ty: &Type) -> Option<String> {
        let Type::Class(class) = ty else { return None };
        let constructors = self.constructors.get(class).cloned().unwrap_or_else(|| vec![class.clone()]);
        constructors.into_iter().find(|ctor| self.functions.get(ctor).is_some_and(|(params, ..)| argument_range(params).0 == 0))
    }

    /// Adds a constructor with `params` to the class `class` and returns its
    /// mangled name: the class's name for the first one, like `Point`, and
    /// `Point#2`, `Point#3` for the others.
    ///
    /// # Panics
    ///
    /// Panics if another constructor of the class takes the same number of
    /// arguments, a call couldn't tell them apart.
    fn add_constructor(&mut self, class: &str, params: &[Param], span: &Span) -> String {
        let range = argument_range(params);
        let existing = self.constructors.get(class).cloned().unwrap_or_default();
        for other in &existing {
            let other_range = self.functions.get(other).map(|(params, ..)| argument_range(params)).unwrap_or((0, 0));
            if range.0 <= other_range.1 && other_range.0 <= range.1 {
                let count = range.0.max(other_range.0);
                diagnostic::throw(
                    format!("'{}' has two constructors taking {} argument{}, a call couldn't tell them apart",
                        self.written_name(class), count, if count == 1 { "" } else { "s" }),
                    Some(span.clone()),
                    &self.source,
                );
            }
        }
        let mangled = match existing.len() {
            0 => class.to_string(),
            n => format!("{}#{}", class, n + 1),
        };
        self.constructors.entry(class.to_string()).or_default().push(mangled.clone());
        mangled
    }

    /// The constructor of the class `class` a call with `count` arguments
    /// goes to, if the class has several. `None` if it has one or none, the
    /// call is checked against that as usual.
    ///
    /// # Panics
    ///
    /// Panics if none of its constructors takes `count` arguments.
    fn pick_constructor(&self, class: &str, count: usize, span: &Span) -> Option<String> {
        let constructors = self.constructors.get(class).filter(|constructors| constructors.len() > 1)?;
        let fits = |ctor: &&String| self.functions.get(*ctor).is_some_and(|(params, ..)| {
            let (min, max) = argument_range(params);
            min <= count && count <= max
        });
        match constructors.iter().find(fits) {
            Some(ctor) => Some(ctor.clone()),
            None => diagnostic::throw(
                format!("'{}' has no constructor taking {} argument{}", self.written_name(class), count, if count == 1 { "" } else { "s" }),
                Some(span.clone()),
                &self.source,
            ),
        }
    }

    /// Registers the generic function or method `decl` as a template for its instances.
    ///
    /// Methods are registered under their mangled name like `Box<int32>_convert`,
    /// together with the class and its type arguments.
    ///
    /// # Panics
    ///
    /// Panics if it's a method of an impl block, which can't be generic, or a
    /// bound isn't a known trait.
    fn declare_generic(&mut self, decl: &Declaration) {
        // An imported trait like `AsPath` of `import std::path::{AsPath};` is `path::AsPath`
        let mut decl = decl.clone();
        if let DeclarationKind::Function { type_params, .. } = &mut decl.kind {
            for bound in type_params.iter_mut().flat_map(|p| p.bounds.iter_mut()) {
                *bound = self.resolve_name(bound, &decl.span);
            }
        }
        let decl = &decl;
        let DeclarationKind::Function { name, type_params, .. } = &decl.kind else { return };
        // A trait method can't be generic, an impl couldn't match all its instances
        if self.current_trait.is_some() {
            SemanticError::GenericMethod { method: name.clone(), span: decl.span.clone() }.throw(&self.source);
        }
        for bound in type_params.iter().flat_map(|p| &p.bounds) {
            if !self.traits.contains_key(bound) {
                SemanticError::UnknownTrait { name: bound.clone(), span: decl.span.clone() }.throw(&self.source);
            }
        }

        // Generic methods are registered under their mangled name, like `Box<int32>_convert`
        let key = match &self.current_class {
            Some(class) => format!("{}_{}", class, name),
            None => name.clone(),
        };
        self.generic_functions.insert(key, GenericDecl {
            decl: decl.clone(),
            module: self.module.clone(),
            source: self.source.clone(),
            imports: self.imports(),
            class: self.current_class.clone(),
            // Inside a generic class instance, its type arguments are already fixed
            outer_subst: self.type_subst.clone(),
        });
    }

    /// Returns the instance of the generic function `name` for a call with the
    /// arguments `args`, creating and checking it the first time.
    ///
    /// The type arguments are `type_args` if given explicitly. Otherwise they are
    /// inferred from the types of typed arguments first, then from the type
    /// `expected` for the call's result, and last from literal arguments, so
    /// `max(x, 7)` with `x: int64` gives `int64` instead of the literal's `int32`.
    ///
    /// The instance's body is checked in the module that declares the generic
    /// function, with every type parameter replaced by its type argument.
    ///
    /// # Panics
    ///
    /// Panics if the explicit type arguments have the wrong count, a type
    /// parameter can't be inferred, a type argument doesn't implement a bound,
    /// or the instance's body contains an error.
    /// Checks the arguments of a call of a generic function or constructor
    /// before its type arguments are inferred from them. Like for other calls,
    /// an argument for a `&` or slice parameter of `declared` is only borrowed,
    /// the others are taken.
    fn check_generic_args(&mut self, args: &mut [Expr], declared: &[Type]) {
        for (i, arg) in args.iter_mut().enumerate() {
            if matches!(declared.get(i), Some(Type::Ref(_) | Type::Slice(_))) {
                self.check_borrowed(arg, None);
            } else {
                self.check_expr(arg);
            }
        }
    }

    fn instantiate(&mut self, name: &str, type_args: &[Type], args: &mut [Expr], expected: Option<&Type>, span: &Span) -> String {
        let generic = self.generic_functions.get(name).cloned().unwrap();
        let DeclarationKind::Function { type_params, params, return_type, .. } = &generic.decl.kind else {
            unreachable!("generic functions are function declarations");
        };

        let declared: Vec<Type> = params.iter().filter(|p| p.name != "self").map(|p| p.ty.clone()).collect();
        self.check_generic_args(args, &declared);
        let bindings: Vec<(Type, &Expr)> = params.iter()
            .filter(|p| p.name != "self")
            .map(|p| p.ty.clone())
            .zip(args.iter())
            .collect();
        let result = return_type.as_ref().zip(expected);
        let subst = self.infer_type_args(name, type_params, type_args, &bindings, result, span);

        let instance = instance_name(name, type_params, &subst);
        if self.functions.contains_key(&instance) {
            return instance;
        }

        let mut decl = generic.decl.clone();
        let module = generic.module.clone();
        self.in_template_context(subst, &generic, |sema| {
            let DeclarationKind::Function { type_params, params, return_type, mangled, visibility, .. } = &mut decl.kind else {
                unreachable!("generic functions are function declarations");
            };
            type_params.clear();
            // Resolves `T`, and instantiates types like `Box<T>` in the signature
            for p in params.iter_mut() {
                let span = p.ty_span.clone();
                p.ty = sema.canonicalize_type(&p.ty, &span);
            }
            if let Some(ty) = return_type.as_mut() {
                *ty = sema.canonicalize_type(ty, span);
            }
            *mangled = Some(instance.clone());

            // Registered before checking the body, so the instance can call itself
            let ret = return_type.clone().unwrap_or(Type::Void);
            sema.functions.insert(instance.clone(), (params.clone(), ret, visibility.clone(), module));

            sema.check_decl(&mut decl);
        });

        self.instances.push(decl);
        instance
    }

    /// Infers the type arguments of the generic `owner` with the type parameters
    /// `type_params`, and checks their bounds.
    ///
    /// The type arguments are `explicit` if given. Otherwise they are inferred
    /// from `bindings`, pairs of a declared type like `T` or `Box<T>` and an
    /// already checked expression for it: typed expressions first, then the
    /// declared result type against the expected one in `result`, and last
    /// literals, so `max(x, 7)` with `x: int64` gives `int64` instead of the
    /// literal's `int32`.
    ///
    /// # Panics
    ///
    /// Panics if the explicit type arguments have the wrong count, a type
    /// parameter can't be inferred or a type argument doesn't implement a bound.
    fn infer_type_args(
        &mut self,
        owner: &str,
        type_params: &[TypeParam],
        explicit: &[Type],
        bindings: &[(Type, &Expr)],
        result: Option<(&Type, &Type)>,
        span: &Span,
    ) -> HashMap<String, Type> {
        let mut subst: HashMap<String, Type> = HashMap::new();
        if !explicit.is_empty() {
            // `impl Trait` parameters can't be given explicitly
            let named: Vec<&TypeParam> = type_params.iter().filter(|p| !p.is_hidden()).collect();
            if named.len() != explicit.len() {
                SemanticError::WrongTypeArgCount {
                    function: owner.to_string(),
                    expected: named.len(),
                    got: explicit.len(),
                    span: span.clone(),
                }.throw(&self.source);
            }
            for (param, arg) in named.iter().zip(explicit) {
                let ty = self.canonicalize_type(arg, span);
                self.check_type(&ty, span);
                subst.insert(param.name.clone(), ty);
            }
        }

        for (declared, arg) in bindings {
            if !is_number_literal(arg) {
                self.bind_type_params(declared, &arg.ty, &mut subst);
            }
        }
        if let Some((declared, expected)) = result {
            self.bind_type_params(declared, expected, &mut subst);
        }
        for (declared, arg) in bindings {
            if is_number_literal(arg) {
                self.bind_type_params(declared, &arg.ty, &mut subst);
            }
        }

        for param in type_params {
            let Some(ty) = subst.get(&param.name) else {
                SemanticError::CannotInferTypeParam {
                    param: param.name.clone(),
                    function: self.written_name(owner),
                    span: span.clone(),
                }.throw(&self.source)
            };
            for bound in &param.bounds {
                if !self.implements(ty, bound) {
                    SemanticError::TraitNotImplemented {
                        ty: ty.clone(),
                        trait_name: bound.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
        }
        subst
    }

    /// Binds the type parameters in the declared type `pattern` to the matching
    /// part of the actual type `actual`. Earlier bindings are kept.
    fn bind_type_params(&self, pattern: &Type, actual: &Type, subst: &mut HashMap<String, Type>) {
        match (pattern, actual) {
            (_, Type::Unknown | Type::Void) => {}
            (Type::Param(name), _) => {
                subst.entry(name.clone()).or_insert_with(|| actual.clone());
            }
            (Type::Ref(pattern), Type::Ref(actual)) => self.bind_type_params(pattern, actual, subst),
            (Type::Unique(pattern), Type::Unique(actual)) | (Type::Shared(pattern), Type::Shared(actual))
                | (Type::Buffer(pattern), Type::Buffer(actual)) => {
                self.bind_type_params(pattern, actual, subst);
            }
            (Type::Array(pattern, _), Type::Array(actual, _)) => self.bind_type_params(pattern, actual, subst),
            // `Unique<T>` or `Buffer<T>` in a template, which stays generic until `T` is known
            (Type::Generic { name, args }, Type::Unique(actual)) if name == "Unique" && args.len() == 1 => {
                self.bind_type_params(&args[0], actual, subst);
            }
            (Type::Generic { name, args }, Type::Shared(actual)) if name == "Shared" && args.len() == 1 => {
                self.bind_type_params(&args[0], actual, subst);
            }
            (Type::Generic { name, args }, Type::Buffer(actual)) if name == "Buffer" && args.len() == 1 => {
                self.bind_type_params(&args[0], actual, subst);
            }
            // Arguments for `&T` parameters are referenced automatically
            (Type::Ref(pattern), _) => self.bind_type_params(pattern, actual, subst),
            // `Box<T>` against the instance `Box<int32>` binds `T = int32`
            (Type::Generic { name, args }, Type::Class(instance) | Type::Enum(instance)) => {
                if let Some((template, instance_args)) = self.type_instances.get(instance) {
                    if template == name {
                        for (pattern, actual) in args.iter().zip(instance_args) {
                            self.bind_type_params(pattern, actual, subst);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Runs `f` like inside the declaration of the generic `template`: with
    /// fresh local variables, the type parameters replaced by `subst` and the
    /// ones fixed by its surroundings, inside its class for a generic method,
    /// and with the module and source of the template, so its private helpers
    /// are visible and errors point at its code. Everything is restored afterwards.
    fn in_template_context<R>(&mut self, subst: HashMap<String, Type>, template: &GenericDecl, f: impl FnOnce(&mut Self) -> R) -> R {
        let mut full_subst = template.outer_subst.clone();
        full_subst.extend(subst);

        let scope = std::mem::take(&mut self.scope);
        let moved = std::mem::take(&mut self.moved);
        let outer_subst = std::mem::replace(&mut self.type_subst, full_subst);
        let class = std::mem::replace(&mut self.current_class, template.class.clone());
        let trait_name = self.current_trait.take();
        let return_ty = self.current_return_type.take();
        let module = std::mem::replace(&mut self.module, template.module.clone());
        let source = std::mem::replace(&mut self.source, template.source.clone());
        // Names in the template mean what they mean in its own module
        let imported_modules = std::mem::replace(&mut self.imported_modules, template.imports.imported_modules.clone());
        let aliases = std::mem::replace(&mut self.aliases, template.imports.aliases.clone());
        let ambiguous = std::mem::replace(&mut self.ambiguous, template.imports.ambiguous.clone());

        let result = f(self);

        self.scope = scope;
        self.moved = moved;
        self.type_subst = outer_subst;
        self.current_class = class;
        self.current_trait = trait_name;
        self.current_return_type = return_ty;
        self.module = module;
        self.source = source;
        self.imported_modules = imported_modules;
        self.aliases = aliases;
        self.ambiguous = ambiguous;
        result
    }

    /// What the current module imports, for checking its generics' instances later.
    fn imports(&self) -> Imports {
        Imports { imported_modules: self.imported_modules.clone(), aliases: self.aliases.clone(), ambiguous: self.ambiguous.clone() }
    }

    /// Returns the instance of the generic class or enum `name` for the type
    /// arguments `args`, like `Box<int32>` for `Box` and `int32`.
    ///
    /// The first time, the instance is created as an ordinary class or enum
    /// named `Box<int32>`, with the type parameters replaced, and declared right
    /// away. Its method bodies are checked at the end of the module.
    ///
    /// # Panics
    ///
    /// Panics if `name` isn't a generic class or enum, the number of type
    /// arguments is wrong, or one doesn't implement a bound.
    fn instantiate_type(&mut self, name: &str, args: Vec<Type>, span: &Span) -> Type {
        let generic = self.generic_types.get(name).cloned()
            .unwrap_or_else(|| SemanticError::UnknownType { name: name.to_string(), span: span.clone() }.throw(&self.source));
        let (is_enum, type_params) = match &generic.decl.kind {
            DeclarationKind::Class { type_params, .. } => (false, type_params.clone()),
            DeclarationKind::Enum { type_params, .. } => (true, type_params.clone()),
            _ => unreachable!("generic types are classes or enums"),
        };
        if args.len() != type_params.len() {
            SemanticError::WrongTypeArgCount {
                function: name.to_string(),
                expected: type_params.len(),
                got: args.len(),
                span: span.clone(),
            }.throw(&self.source);
        }
        let subst: HashMap<String, Type> = type_params.iter().map(|p| p.name.clone()).zip(args.iter().cloned()).collect();
        for param in &type_params {
            for bound in &param.bounds {
                if !self.implements(&subst[&param.name], bound) {
                    // Another module's impl can still come, like `impl Hash for String`
                    // for a `HashMap<String, V>` in a signature
                    if !self.impls_known {
                        self.deferred_bounds.push((subst[&param.name].clone(), bound.clone(), span.clone(), self.module.clone()));
                        continue;
                    }
                    SemanticError::TraitNotImplemented {
                        ty: subst[&param.name].clone(),
                        trait_name: bound.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
        }

        let instance = instance_name(name, &type_params, &subst);
        let ty = if is_enum { Type::Enum(instance.clone()) } else { Type::Class(instance.clone()) };
        if self.type_instances.contains_key(&instance) {
            return ty;
        }

        let mut decl = generic.decl.clone();
        match &mut decl.kind {
            DeclarationKind::Class { name: class_name, type_params, methods, .. } => {
                *class_name = instance.clone();
                type_params.clear();
                for method in methods.iter_mut() {
                    if let DeclarationKind::Function { name: method_name, params, parent, .. } = &mut method.kind {
                        // The constructor is named after the class
                        if method_name == name {
                            *method_name = instance.clone();
                        }
                        if parent.is_some() {
                            *parent = Some(instance.clone());
                        }
                        for p in params.iter_mut().filter(|p| p.name == "self") {
                            p.ty = ty.clone();
                        }
                    }
                }
                self.classes.entry(instance.clone()).or_default();
            }
            DeclarationKind::Enum { name: enum_name, type_params, methods, .. } => {
                *enum_name = instance.clone();
                type_params.clear();
                for method in methods.iter_mut() {
                    if let DeclarationKind::Function { params, parent, .. } = &mut method.kind {
                        *parent = Some(instance.clone());
                        for p in params.iter_mut().filter(|p| p.name == "self") {
                            p.ty = ty.clone();
                        }
                    }
                }
                self.enums.entry(instance.clone()).or_default();
            }
            _ => unreachable!("generic types are classes or enums"),
        }
        // Registered before resolving its types, so the instance can refer to itself
        self.type_instances.insert(instance.clone(), (name.to_string(), args));

        self.in_template_context(subst.clone(), &generic, |sema| {
            sema.canonicalize_decl(&mut decl);
            sema.declare_decl(&mut decl);
            sema.validate_decl_types(&decl, span);
        });
        self.pending_instances.push(PendingInstance {
            decl,
            subst,
            module: generic.module.clone(),
            source: generic.source.clone(),
            imports: generic.imports.clone(),
        });
        // Like `impl<T> Iterator<T> for VectorIter<T>` for `VectorIter<int32>`
        self.apply_generic_impls(&instance);
        ty
    }

    /// Checks the method bodies of an instance of a generic class or enum and
    /// hands it to codegen.
    ///
    /// # Panics
    ///
    /// Panics if a method body contains an error.
    fn check_type_instance(&mut self, pending: PendingInstance) {
        let PendingInstance { mut decl, subst, module, source, imports } = pending;
        let template = GenericDecl { decl: decl.clone(), module, source, imports, class: None, outer_subst: HashMap::new() };
        // A class's or enum's methods are only checked once something calls them, impls fully
        let name = match &decl.kind {
            DeclarationKind::Class { name, .. } | DeclarationKind::Enum { name, .. } => Some(name.clone()),
            _ => None,
        };
        let Some(name) = name else {
            self.in_template_context(subst, &template, |sema| {
                sema.check_decl(&mut decl);
            });
            self.instances.push(decl);
            return;
        };
        let instance = self.type_instance_decls.len();
        let methods_template = GenericDecl { class: Some(name), ..template.clone() };
        self.lazy_instance = Some(LazyInstance { instance, template: methods_template, subst: subst.clone() });
        self.in_template_context(subst, &template, |sema| {
            sema.check_decl(&mut decl);
        });
        self.lazy_instance = None;
        self.type_instance_decls.push(decl);
    }

    /// Registers the methods of the instance being checked, see [`LazyInstance`],
    /// to be checked once they're called. `methods` keeps the generic ones,
    /// which are templates.
    fn register_lazy_methods(&mut self, methods: &mut Vec<Declaration>, lazy: LazyInstance) {
        for method in std::mem::take(methods) {
            let mangled = match &method.kind {
                DeclarationKind::Function { type_params, mangled: Some(mangled), .. } if type_params.is_empty() => mangled.clone(),
                _ => {
                    methods.push(method);
                    continue;
                }
            };
            // Codegen calls `equals` itself for `==`, sema never sees that call
            if self.used_functions.contains_key(&mangled) || function_name(&method) == "equals" {
                self.used_methods.push(mangled.clone());
            }
            self.lazy_methods.insert(mangled, LazyMethod {
                instance: lazy.instance,
                method,
                template: lazy.template.clone(),
                subst: lazy.subst.clone(),
            });
        }
    }

    /// Notes that a call goes to the function or method `mangled`. A method of
    /// an instance of a generic class or enum gets checked then, see [`LazyMethod`].
    fn use_method(&mut self, mangled: &str, span: &Span) {
        if self.used_functions.contains_key(mangled) {
            return;
        }
        self.used_functions.insert(mangled.to_string(), (span.clone(), self.source.clone()));
        if self.lazy_methods.contains_key(mangled) {
            self.used_methods.push(mangled.to_string());
        }
    }

    /// Checks the lazy method `mangled` something called and adds it to its instance.
    ///
    /// If its code doesn't work for the instance's types, like `Vector<Statement>(3)`
    /// needing a default `Statement`, the call needing it gets an error too, next
    /// to the one in the method.
    fn check_lazy_method(&mut self, mangled: &str) {
        let Some(LazyMethod { instance, mut method, template, subst }) = self.lazy_methods.remove(mangled) else { return };
        let errors_before = self.errors.len();
        let result = diagnostic::catch(|| self.in_template_context(subst, &template, |sema| {
            sema.check_decl(&mut method);
        }));
        let failed = result.is_err() || self.errors.len() > errors_before;
        if let Err(error) = result {
            self.errors.push(error);
        }
        if failed {
            if let Some((span, source)) = self.used_functions.get(mangled).cloned() {
                let class = template.class.clone().unwrap_or_default();
                let what = match function_name(&method) {
                    name if name == class => "this constructor".to_string(),
                    name => format!("'{}'", name),
                };
                self.errors.push(CompileError {
                    message: format!("{} of '{}' can't be used, its code doesn't work for these types, see the error in it",
                        what, crate::analysis::show_type(&Type::Class(class))),
                    span: Some(span),
                    source,
                    file: None,
                });
            }
            return;
        }
        if let DeclarationKind::Class { methods, .. } | DeclarationKind::Enum { methods, .. } = &mut self.type_instance_decls[instance].kind {
            methods.push(method);
        }
    }

    /// The generic result type of a generic class or enum's constructors, like
    /// `Box<T>` for `class Box<T>`, to infer type arguments from an expected type.
    fn generic_self_type(name: &str, type_params: &[TypeParam]) -> Type {
        Type::Generic {
            name: name.to_string(),
            args: type_params.iter().map(|p| Type::Param(p.name.clone())).collect(),
        }
    }

    /// Returns the instance of the generic class `name` constructed with `args`,
    /// like `Box<int32>` for `Box(5)` or `Box<int32>(5)`. Its constructor has
    /// the instance's name.
    ///
    /// The type arguments are inferred like for generic functions, from the
    /// constructor arguments and the `expected` type.
    ///
    /// # Panics
    ///
    /// Panics if the type arguments can't be inferred or are invalid.
    fn instantiate_constructor(&mut self, name: &str, type_args: &[Type], args: &mut [Expr], expected: Option<&Type>, span: &Span) -> String {
        let generic = self.generic_types.get(name).cloned().unwrap();
        let DeclarationKind::Class { type_params, methods, .. } = &generic.decl.kind else {
            // `Option<T>()`: an enum's values are made with its variants
            diagnostic::throw(
                format!("'{}' is an enum, it has no constructor: make one with a variant, like `{}::{}`",
                    self.written_name(name), self.written_name(name), first_variant(&generic.decl).unwrap_or("Variant")),
                Some(span.clone()),
                &self.source,
            )
        };
        let constructor_params = constructor_params(name, methods, args.len());

        let declared: Vec<Type> = constructor_params.iter().map(|p| p.ty.clone()).collect();
        self.check_generic_args(args, &declared);
        let bindings: Vec<(Type, &Expr)> = constructor_params.iter().map(|p| p.ty.clone()).zip(args.iter()).collect();
        let self_type = Self::generic_self_type(name, type_params);
        let result = expected.map(|expected| (&self_type, expected));
        let subst = self.infer_type_args(name, type_params, type_args, &bindings, result, span);

        let type_args = type_params.iter().map(|p| subst[&p.name].clone()).collect();
        self.instantiate_type(name, type_args, span).owner_name().unwrap()
    }

    /// Returns the instance of the generic class `class` for a brace initializer
    /// with `fields`, like `Box<int32>` for `Box { value: 5 }`.
    ///
    /// The type arguments are inferred like for constructor calls. The field
    /// values get checked for that, the caller mustn't check them again.
    ///
    /// # Panics
    ///
    /// Panics if the type arguments can't be inferred or are invalid.
    fn instantiate_struct_init(&mut self, class: &str, type_args: &[Type], fields: &mut [(String, Expr)], expected: Option<&Type>, span: &Span) -> String {
        let generic = self.generic_types.get(class).cloned().unwrap();
        let DeclarationKind::Class { type_params, methods, .. } = &generic.decl.kind else {
            SemanticError::UnknownFunction { fn_name: class.to_string(), span: span.clone() }.throw(&self.source)
        };
        let names: Vec<String> = fields.iter().map(|(field, _)| field.clone()).collect();
        let constructor_params = constructor_params_named(class, methods, &names);

        for (_, value) in fields.iter_mut() {
            self.check_expr(value);
        }
        // Unknown field names are reported later, when the fields are matched to the constructor
        let bindings: Vec<(Type, &Expr)> = fields.iter()
            .filter_map(|(field, value)| {
                constructor_params.iter().find(|p| p.name == *field).map(|p| (p.ty.clone(), value))
            })
            .collect();
        let self_type = Self::generic_self_type(class, type_params);
        let result = expected.map(|expected| (&self_type, expected));
        let subst = self.infer_type_args(class, type_params, type_args, &bindings, result, span);

        let type_args = type_params.iter().map(|p| subst[&p.name].clone()).collect();
        self.instantiate_type(class, type_args, span).owner_name().unwrap()
    }

    /// Returns the instance of the generic enum `name` for constructing its
    /// variant `tag` with `value`, like `Option<int32>` for `Option::Some(5)`.
    ///
    /// The type arguments are inferred from the payload, unless `check_value`
    /// is false, and the `expected` type. If the payload got checked, the
    /// caller mustn't check it again.
    ///
    /// # Panics
    ///
    /// Panics if the variant doesn't exist or the type arguments can't be inferred.
    fn instantiate_enum_init(&mut self, name: &str, type_args: &[Type], tag: &str, value: &mut Expr, check_value: bool, expected: Option<&Type>, span: &Span) -> String {
        let generic = self.generic_types.get(name).cloned().unwrap();
        let DeclarationKind::Enum { type_params, fields, .. } = &generic.decl.kind else {
            SemanticError::UnknownEnum { name: name.to_string(), span: span.clone() }.throw(&self.source)
        };
        let payload = fields.iter().find(|(variant, _, _, _)| variant == tag)
            .map(|(_, _, ty, _)| ty.clone())
            .unwrap_or_else(|| SemanticError::UnknownVariant {
                enum_name: name.to_string(),
                variant: tag.to_string(),
                span: span.clone(),
            }.throw(&self.source));

        let mut bindings: Vec<(Type, &Expr)> = Vec::new();
        if check_value {
            self.check_expr(value);
        }
        // Where an `Option` or `Result` around it is expected, like `Ok(n)` returned as an
        // `Option<Result<int32, String>>`, it's the instance inside, which becomes `Some(...)` by itself
        let inner = self.expected_instance_of(name, expected);
        let expected = inner.as_ref().or(expected);
        // Where an instance of this enum is expected, it's that one, like `Ok(value)` with
        // `value: &int64` returned as a `Result<int64, String>` reads the number
        if let (true, Some(Type::Enum(instance))) = (type_args.is_empty(), expected) {
            if self.type_instances.get(instance).is_some_and(|(template, _)| template == name) {
                return instance.clone();
            }
        }
        if check_value {
            bindings.push((payload, value));
        }
        let self_type = Self::generic_self_type(name, type_params);
        let result = expected.map(|expected| (&self_type, expected));
        let subst = self.infer_type_args(name, type_params, type_args, &bindings, result, span);

        let type_args = type_params.iter().map(|p| subst[&p.name].clone()).collect();
        self.instantiate_type(name, type_args, span).owner_name().unwrap()
    }

    /// Whether `ty` implements the trait `trait_name`.
    fn implements(&self, ty: &Type, trait_name: &str) -> bool {
        // A reference has the methods of the value it refers to
        let ty = match ty {
            Type::Ref(inner) => inner,
            other => other,
        };
        // A class with a constructor taking no arguments has a default without an impl
        if trait_name == DEFAULT_TRAIT && self.default_constructor(ty).is_some() {
            return true;
        }
        ty.owner_name()
            .and_then(|owner| self.impls.get(&owner))
            .is_some_and(|traits| traits.iter().any(|t| t == trait_name))
    }

    /// Checks `stmt`. After an error in it, checking goes on with the next
    /// statement, the variables it declares have an unknown type.
    fn check_stmt(&mut self, stmt: &mut Stmt) {
        if self.recover(|s| s.check_stmt_inner(stmt)).is_some() {
            return;
        }
        match &stmt.kind {
            StmtKind::Instantiation { name, mutable, .. } => {
                self.scope.insert(name.clone(), (Type::Unknown, *mutable));
                self.moved.remove(name.as_str());
            }
            StmtKind::Destructure { names, mutable, .. } => {
                for name in names.iter().filter(|name| *name != "_") {
                    self.scope.insert(name.clone(), (Type::Unknown, *mutable));
                    self.moved.remove(name.as_str());
                }
            }
            _ => {}
        }
    }

    fn check_stmt_inner(&mut self, stmt: &mut Stmt) {
        let stmt_span = stmt.span.clone();
        let ty = match &mut stmt.kind {
            StmtKind::Return(expr) => {
                // At the top level, `return` ends the program with an exit code
                let cur = self.current_return_type.clone().unwrap_or(Type::INT32);
                self.check_expr_expecting(expr, &cur);
                self.coerce(expr, &cur);
                let ret = expr.ty.clone();
                if ret != cur {
                    SemanticError::TypeMismatch { lhs: ret, rhs: cur , span: expr.span.clone(), context: "function return type" }.throw(&self.source);
                }
                ret
            },
            StmtKind::Declaration(decl) => self.check_decl(decl),
            StmtKind::Instantiation { name, typedef, value, mutable } => {
                // The hidden iterator of a `for` loop, see `Parser::parse_for`
                if name.starts_with("for$") {
                    self.for_iterator(value);
                }
                let declared = typedef.as_ref().map(|td| {
                    let ty = self.canonicalize_type(td, &stmt_span);
                    self.check_type(&ty, &stmt_span);
                    ty
                });
                let value_ty = match &declared {
                    Some(ty) => self.check_expr_expecting(value, ty),
                    None => self.check_expr(value),
                };
                let ty = match declared {
                    Some(ty) => {
                        self.coerce(value, &ty);
                        if value.ty != ty {
                            SemanticError::TypeMismatch {
                                lhs: value.ty.clone(),
                                rhs: ty,
                                span: value.span.clone(),
                                context: "variable declaration",
                            }.throw(&self.source);
                        }
                        ty
                    }
                    None => value_ty,
                };
                self.check_type(&ty, &stmt_span);
                self.scope.insert(name.clone(), (ty.clone(), *mutable));
                // A new variable, even if an old one with the same name got moved
                self.moved.remove(name.as_str());
                *typedef = Some(ty.clone());
                ty
            }
            StmtKind::Destructure { names, value, mutable } => {
                let ty = self.check_expr(value);
                let types = match &ty {
                    Type::Tuple(types) if types.len() == names.len() => types.clone(),
                    _ => diagnostic::throw(format!("can't take {} values out of '{}'", names.len(), ty), Some(stmt_span.clone()), &self.source),
                };
                for (name, ty) in names.iter().zip(types) {
                    if name != "_" {
                        self.scope.insert(name.clone(), (ty, *mutable));
                        self.moved.remove(name.as_str());
                    }
                }
                Type::Void
            }
            StmtKind::Import(expr) => self.check_expr(expr),
            StmtKind::Expression(expr) => self.check_unused(expr),
            StmtKind::While { condition, body } => {
                // Names an `is` in the condition binds exist in the body only
                let is_mark = self.is_bindings.len();
                self.keep_is_bindings = true;
                self.check_condition(condition, "while condition");
                let moved_before = self.moved.clone();
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.drop_is_bindings(is_mark);
                // The body can run several times, a value moved in one round would be gone in the next
                if let Some(var) = self.moved.iter().find(|var| !moved_before.contains(*var)) {
                    SemanticError::MoveInLoop { var: var.clone(), span: stmt_span.clone() }.throw(&self.source);
                }
                Type::Void
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.loop_depth == 0 {
                    let keyword = if matches!(stmt.kind, StmtKind::Break) { "break" } else { "continue" };
                    SemanticError::OutsideLoop { keyword, span: stmt_span.clone() }.throw(&self.source);
                }
                Type::Void
            }
        };

        stmt.ty = ty.clone();
    }

    /// Checks the condition of an `if` or `while`, which has to be a `bool`.
    ///
    /// # Panics
    ///
    /// Panics if the condition has another type.
    fn check_condition(&mut self, condition: &mut Expr, context: &'static str) {
        let ty = self.check_expr_expecting(condition, &Type::Bool);
        if ty != Type::Bool {
            SemanticError::TypeMismatch {
                lhs: ty,
                rhs: Type::Bool,
                span: condition.span.clone(),
                context,
            }.throw(&self.source);
        }
    }

    /// Checks the statements of a block like the body of an `if` or `while`.
    /// Variables declared in it are gone afterwards.
    fn check_block(&mut self, body: &mut Vec<Stmt>) {
        let prev_scope = self.scope.clone();
        for stmt in body.iter_mut() {
            self.check_stmt(stmt);
        }
        self.moved.retain(|path| prev_scope.contains_key(path_root(path)));
        self.scope = prev_scope;
    }

    /// Checks the arms of a match on a value of type `scrutinee_ty`.
    ///
    /// `result` is `None` for a match statement, whose arm values are unused.
    /// For a match used as a value it holds the expected type, if known, and
    /// the match's position. Then every arm has to give a value of the same
    /// type or leave with `return`, `break`, `continue`, `panic` or `exit`, and
    /// the arms have to cover every case. Returns the type of the values.
    ///
    /// # Panics
    ///
    /// Panics if a pattern doesn't fit the scrutinee, or a match used as a
    /// value breaks the rules above.
    fn check_match_arms(&mut self, scrutinee_ty: &Type, arms: &mut Vec<MatchArm>, result: Option<(Option<Type>, Span)>) -> Type {
        // Every arm starts with what was moved before the match. Afterwards a
        // variable counts as moved if any arm moved it, since that arm might have run.
        let moved_before = self.moved.clone();
        let mut moved_after = moved_before.clone();
        let mut result_ty = result.as_ref().and_then(|(expected, _)| expected.clone());
        // A match inside an arm changes it
        let owner = self.pattern_owner.clone();

        for arm in arms.iter_mut() {
            let prev_scope = self.scope.clone();
            let prev_borrowed = self.borrowed.clone();
            let prev_roots = self.binding_roots.clone();
            self.moved = moved_before.clone();
            self.pattern_owner = owner.clone();
            self.check_pattern(&mut arm.pattern, scrutinee_ty, &arm.span);
            let value_ty = result.is_some().then_some(&mut result_ty);
            self.check_value_block(&mut arm.block, value_ty, "arm", &arm.span);
            // Variables declared in the arm are gone afterwards, and an arm leaving the function never gets there
            if !block_returns(&arm.block) {
                moved_after.extend(self.moved.iter().filter(|path| prev_scope.contains_key(path_root(path))).cloned());
            }
            self.scope = prev_scope;
            self.borrowed = prev_borrowed;
            self.binding_roots = prev_roots;
        }
        self.moved = moved_after;

        let Some((_, span)) = result else { return Type::Void };
        if !self.covers_every_case(scrutinee_ty, arms) {
            SemanticError::NonExhaustiveMatch { span: span.clone() }.throw(&self.source);
        }
        match result_ty {
            Some(ty) if ty != Type::Void => ty,
            _ => SemanticError::WithoutValue { what: "match", span }.throw(&self.source),
        }
    }

    /// Checks the statements and value of `block`, a match arm or a branch of
    /// an `if`, called `what` in errors. The caller handles the scope.
    ///
    /// `result` is `None` if the block's value is unused. Otherwise it holds
    /// the type every block of the match or if gives, which the first value
    /// decides if it isn't known. A block without value then has to leave
    /// with `return`, `break`, `continue`, `panic` or `exit`.
    ///
    /// # Panics
    ///
    /// Panics if the block contains an error or breaks the rules above.
    fn check_value_block(&mut self, block: &mut Block, result: Option<&mut Option<Type>>, what: &'static str, span: &Span) {
        for stmt in block.stmts.iter_mut() {
            self.check_stmt(stmt);
        }
        match (&mut block.value, result) {
            // The value of a block used as a statement is unused, it's only run
            (Some(value), None) => {
                self.check_unused(value);
            }
            // `panic(...)`, `exit(...)` or a match whose arms all leave never come
            // back, there's no value to give
            (Some(value), Some(_)) if leaves(value) => {
                self.check_unused(value);
            }
            (Some(value), Some(result_ty)) => match result_ty {
                Some(ty) => {
                    let ty = ty.clone();
                    let context = if what == "arm" { "match arm value" } else { "if branch value" };
                    self.check_argument(value, &ty, context);
                }
                // The first value decides the type of the others
                None => *result_ty = Some(self.check_expr(value)),
            },
            (None, Some(_)) if !diverges(&block.stmts) => {
                SemanticError::BlockWithoutValue { what, span: span.clone() }.throw(&self.source);
            }
            (None, _) => {}
        }
    }

    /// Checks `expr`, whose value is unused, like an expression statement. A
    /// match or if then only runs its blocks and needs no value.
    fn check_unused(&mut self, expr: &mut Expr) -> Type {
        match &mut expr.kind {
            ExprKind::Match { scrutinee, arms } => {
                // Only read, its bindings own or borrow their values like it. A `&` is
                // matched by the value it refers to
                let scrutinee_ty = match self.check_borrowed(scrutinee, None) {
                    Type::Ref(inner) => *inner,
                    other => other,
                };
                self.pattern_owner = self.owner_of(scrutinee);
                self.check_match_arms(&scrutinee_ty, arms, None);
            }
            ExprKind::If { condition, then_block, else_block } => {
                self.check_if(condition, then_block, else_block, None);
            }
            // Like an `if`, `ready ? start() : wait();` only runs one of them
            ExprKind::Ternary { condition, then_value, else_value } => {
                self.check_ternary(condition, then_value, else_value, None);
            }
            _ => return self.check_expr(expr),
        }
        expr.ty = Type::Void;
        Type::Void
    }

    /// Checks `condition ? then_value : else_value` like an `if` whose branches
    /// are only those values, see [`Sema::check_if`] for `result`.
    fn check_ternary(&mut self, condition: &mut Expr, then_value: &mut Box<Expr>, else_value: &mut Box<Expr>, result: Option<(Option<Type>, Span)>) -> Type {
        let span = then_value.span.clone();
        let placeholder = || Box::new(Expr::new(ExprKind::Boolean(false), span.clone()));
        let mut then_block = Block::of_value(*std::mem::replace(then_value, placeholder()));
        let mut else_block = Some(Block::of_value(*std::mem::replace(else_value, placeholder())));
        let ty = self.check_if(condition, &mut then_block, &mut else_block, result);
        *then_value = then_block.value.expect("a branch of `? :` is only its value");
        *else_value = else_block.and_then(|block| block.value).expect("a branch of `? :` is only its value");
        ty
    }

    /// Checks an `if` with the blocks `then_block` and `else_block`. `result`
    /// is like for [`Sema::check_match_arms`]: `None` for an `if` statement,
    /// the expected type and the position for an `if` used as a value, which
    /// then needs an `else`. Returns the type of the value.
    ///
    /// # Panics
    ///
    /// Panics if the condition isn't a `bool`, a block contains an error, or
    /// an `if` used as a value breaks the rules of [`Sema::check_value_block`].
    fn check_if(&mut self, condition: &mut Expr, then_block: &mut Block, else_block: &mut Option<Block>, result: Option<(Option<Type>, Span)>) -> Type {
        // Names an `is` in the condition binds exist in the then branch only
        let is_mark = self.is_bindings.len();
        self.keep_is_bindings = true;
        self.check_condition(condition, "if condition");
        let as_value = result.is_some();
        let (expected, span) = result.unwrap_or((None, condition.span.clone()));
        let mut result_ty = expected;

        // Like match arms: each branch starts from the same moves, afterwards
        // a variable counts as moved if a branch moved it. A branch leaving the
        // function, like with `return`, never gets there
        let moved_before = self.moved.clone();
        self.check_branch(then_block, as_value.then_some(&mut result_ty), &span);
        self.drop_is_bindings(is_mark);
        let mut moved_after = std::mem::replace(&mut self.moved, moved_before.clone());
        if block_returns(then_block) {
            moved_after = moved_before;
        }
        match else_block {
            Some(else_block) => {
                self.check_branch(else_block, as_value.then_some(&mut result_ty), &span);
                if !block_returns(else_block) {
                    moved_after.extend(self.moved.drain());
                }
            }
            None if as_value => SemanticError::IfWithoutElse { span: span.clone() }.throw(&self.source),
            None => moved_after.extend(self.moved.drain()),
        }
        self.moved = moved_after;

        if !as_value {
            return Type::Void;
        }
        match result_ty {
            Some(ty) if ty != Type::Void => ty,
            _ => SemanticError::WithoutValue { what: "if", span }.throw(&self.source),
        }
    }

    /// Checks a branch of an `if`, which is its own scope, see [`Sema::check_value_block`].
    fn check_branch(&mut self, block: &mut Block, result: Option<&mut Option<Type>>, span: &Span) {
        let prev_scope = self.scope.clone();
        self.check_value_block(block, result, "branch", span);
        self.moved.retain(|path| prev_scope.contains_key(path_root(path)));
        self.scope = prev_scope;
    }

    /// Checks `pattern` against a value of type `ty` and adds its names to the
    /// scope. `span` is the position of the arm, for errors.
    ///
    /// # Panics
    ///
    /// Panics if the pattern can't match a value of that type.
    fn check_pattern(&mut self, pattern: &mut Pattern, ty: &Type, span: &Span) {
        match pattern {
            Pattern::EnumVariant { enum_name, variant, payload } => {
                if let Some(qualified) = self.aliases.get(enum_name.as_str()) {
                    *enum_name = qualified.clone();
                } else if enum_name == "Option" || enum_name == "Result" {
                    // `Some(x)` and the like, from `std::option` or `std::result`
                    *enum_name = self.variant_enum(enum_name);
                }
                let scrut_enum = match ty {
                    Type::Enum(n) => n.clone(),
                    other => SemanticError::InvalidReceiver {
                        recv_ty: other.clone(),
                        span: span.clone(),
                        context: "cannot be matched against an enum variant pattern",
                    }.throw(&self.source),
                };
                // `Option::Some(x)` matches any instance of the generic enum `Option`,
                // codegen needs the exact one like `Option<int32>`
                // `Ok(text)` on what `fs::read` gives is `result::Result`'s, also without importing it
                let is_instance_of_pattern = self.type_instances.get(&scrut_enum)
                    .is_some_and(|(template, _)| template == enum_name
                        || ((enum_name == "Option" || enum_name == "Result") && *template == format!("{}::{}", enum_name.to_lowercase(), enum_name)));
                // `Type::Boolean` on a `type::Type` is its variant, also where `Type` alone could
                // mean another type, like `llvm::Type`: the matched value says which one
                let template = self.type_instances.get(&scrut_enum).map(|(template, _)| template.clone())
                    .unwrap_or_else(|| scrut_enum.clone());
                let written_like_matched = crate::analysis::written_name(&template) == enum_name.as_str();
                if is_instance_of_pattern || written_like_matched {
                    *enum_name = scrut_enum.clone();
                }
                if enum_name != &scrut_enum {
                    SemanticError::TypeMismatch {
                        lhs: Type::Enum(enum_name.clone()),
                        rhs: Type::Enum(scrut_enum.clone()),
                        span: span.clone(),
                        context: "match arm pattern",
                    }.throw(&self.source);
                }
                let variants = self.enums.get(&scrut_enum).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownEnum {
                        name: scrut_enum.clone(),
                        span: span.clone(),
                    }.throw(&self.source));
                let payload_ty = variants.iter().find(|(v, _, _)| v == variant)
                    .map(|(_, _, t)| t.clone())
                    .unwrap_or_else(|| SemanticError::UnknownVariant {
                        enum_name: scrut_enum.clone(),
                        variant: variant.clone(),
                        span: span.clone(),
                    }.throw(&self.source));
                // The payload matches its own pattern, like `x` or `Ok(x)` of `Some(Ok(x))`
                if let Some(payload) = payload {
                    if payload_ty == Type::Void {
                        SemanticError::VariantWithoutPayload {
                            variant: format!("{}::{}", enum_name, variant),
                            span: span.clone(),
                        }.throw(&self.source);
                    }
                    self.check_pattern(payload, &payload_ty, span);
                }
            }
            // A name matches anything and gets the value
            Pattern::Literal(expr) if matches!(expr.kind, ExprKind::Identifier(_)) => {
                self.add_binding(&expr.kind.to_string(), ty.clone());
                expr.ty = ty.clone();
            }
            Pattern::Literal(expr) => {
                self.check_expr(expr);
                self.coerce_literal(expr, ty);
                let lit_ty = expr.ty.clone();
                if &lit_ty != ty {
                    SemanticError::TypeMismatch {
                        lhs: lit_ty,
                        rhs: ty.clone(),
                        span: expr.span.clone(),
                        context: "match arm pattern",
                    }.throw(&self.source);
                }
            }
            Pattern::Wildcard => {}
            Pattern::List { before, rest, after } => {
                let Some((element, len)) = self.list_element(ty) else {
                    SemanticError::NotAList { ty: ty.clone(), span: span.clone() }.throw(&self.source)
                };
                // The length of an array is known, a pattern of another length never matches
                let items = (before.len() + after.len()) as u64;
                if let Some(len) = len {
                    let fits = if rest.is_some() { items <= len } else { items == len };
                    if !fits {
                        SemanticError::ListPatternLength { items, len, rest: rest.is_some(), span: span.clone() }
                            .throw(&self.source);
                    }
                }
                for item in before.iter_mut().chain(after.iter_mut()) {
                    self.check_pattern(item, &element, span);
                }
                if let Some(rest) = rest {
                    // The items in between are a smaller array, or a `Vector` looking at them
                    rest.ty = match len {
                        Some(len) => Type::Array(Box::new(element), len - items),
                        None => ty.clone(),
                    };
                    if let Some(name) = &rest.binding {
                        self.add_binding(&name.clone(), rest.ty.clone());
                        // The rest of a Vector only looks at its items, it can't own them
                        if len.is_none() {
                            self.borrowed.insert(name.clone());
                        }
                    }
                }
            }
            Pattern::Tuple(items) => {
                let types = match ty {
                    Type::Tuple(types) if types.len() == items.len() => types.clone(),
                    _ => diagnostic::throw(format!("a tuple pattern of {} values can't match '{}'", items.len(), ty), Some(span.clone()), &self.source),
                };
                for (item, ty) in items.iter_mut().zip(&types) {
                    self.check_pattern(item, ty, span);
                }
            }
        }
    }

    /// The item type of a value of type `ty` a list pattern can match, and its
    /// length if it's an array. `None` if it's no array or `Vector`.
    fn list_element(&self, ty: &Type) -> Option<(Type, Option<u64>)> {
        match ty {
            Type::Array(element, len) => Some(((**element).clone(), Some(*len))),
            Type::Class(name) => match self.type_instances.get(name) {
                Some((template, args)) if is_vector(template) && args.len() == 1 => Some((args[0].clone(), None)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Whether `pattern` matches every value of type `ty`.
    fn irrefutable(&self, pattern: &Pattern, ty: &Type) -> bool {
        match pattern {
            Pattern::Wildcard => true,
            Pattern::Literal(expr) => matches!(expr.kind, ExprKind::Identifier(_)),
            Pattern::EnumVariant { .. } => false,
            Pattern::List { before, rest, after } => match self.list_element(ty) {
                // An array pattern has the right length, sema checked it
                Some((element, Some(_))) => before.iter().chain(after).all(|item| self.irrefutable(item, &element)),
                // Only `[..]` matches a `Vector` of any length
                Some((_, None)) => before.is_empty() && after.is_empty() && rest.is_some(),
                None => false,
            },
            Pattern::Tuple(items) => match ty {
                Type::Tuple(types) => items.iter().zip(types).all(|(item, ty)| self.irrefutable(item, ty)),
                _ => false,
            },
        }
    }

    /// Whether the list patterns of `arms` match a `Vector` of items of type
    /// `element` of any length, like `[]` and `[first, ..rest]` together.
    ///
    /// Only patterns whose items always match count. A pattern with `..`
    /// covers every length from its number of items on, the lengths below
    /// that need a pattern of exactly that length.
    fn lists_cover_every_length(&self, element: &Type, patterns: &[&Pattern]) -> bool {
        let mut exact = Vec::new();
        let mut from = None::<usize>;
        for pattern in patterns {
            let Pattern::List { before, rest, after } = pattern else { continue };
            if !before.iter().chain(after).all(|item| self.irrefutable(item, element)) {
                continue;
            }
            let items = before.len() + after.len();
            match rest {
                Some(_) => from = Some(from.map_or(items, |f| f.min(items))),
                None => exact.push(items),
            }
        }
        from.is_some_and(|from| (0..from).all(|len| exact.contains(&len)))
    }

    /// Whether `arms` match every value of type `scrutinee_ty`: one of them
    /// matches anything, or they name every variant of an enum or both bools.
    fn covers_every_case(&self, scrutinee_ty: &Type, arms: &[MatchArm]) -> bool {
        let patterns: Vec<&Pattern> = arms.iter().map(|arm| &arm.pattern).collect();
        self.patterns_cover(scrutinee_ty, &patterns)
    }

    /// Whether `patterns` together match every value of type `ty`, also
    /// looking into payloads: `Some(Ok(x))`, `Some(Err(e))` and `None` match
    /// every `Option<Result<T, E>>`.
    fn patterns_cover(&self, ty: &Type, patterns: &[&Pattern]) -> bool {
        let rows: Vec<Vec<Option<&Pattern>>> = patterns.iter().map(|pattern| vec![Some(*pattern)]).collect();
        self.rows_cover(std::slice::from_ref(ty), &rows)
    }

    /// Whether `rows` match every combination of values of the types `types`.
    /// Each row has a pattern per value, `None` for one matching anything.
    ///
    /// Looks at the first value: for each of its cases, like each variant of
    /// an enum or `true` and `false`, the rows that can match it have to cover
    /// the rest, with the case's payload in front. The parts of a tuple are
    /// values of their own. Numbers and strings have too many cases, there
    /// only the rows matching anything count.
    fn rows_cover<'p>(&self, types: &[Type], rows: &[Vec<Option<&'p Pattern>>]) -> bool {
        let Some((first, rest)) = types.split_first() else { return !rows.is_empty() };
        // A pattern matching anything at the front is like none
        let head = |row: &Vec<Option<&'p Pattern>>| -> Option<&'p Pattern> { row[0].filter(|pattern| !self.irrefutable(pattern, first)) };
        let tail = |row: &[Option<&'p Pattern>]| -> Vec<Option<&'p Pattern>> { row[1..].to_vec() };

        match first {
            Type::Enum(name) => {
                let Some(variants) = self.enums.get(name) else { return false };
                variants.iter().all(|(variant, _, payload_ty)| {
                    let has_payload = *payload_ty != Type::Void;
                    let mut types = Vec::new();
                    if has_payload {
                        types.push(payload_ty.clone());
                    }
                    types.extend(rest.iter().cloned());
                    let rows: Vec<Vec<Option<&Pattern>>> = rows.iter().filter_map(|row| {
                        // `None` stands for the payload of a row matching anything, or written without one
                        let payload = match head(row) {
                            None => None,
                            Some(Pattern::EnumVariant { variant: v, payload, .. }) if v == variant => payload.as_deref(),
                            Some(_) => return None,
                        };
                        let mut specialized = Vec::new();
                        if has_payload {
                            specialized.push(payload);
                        }
                        specialized.extend(tail(row));
                        Some(specialized)
                    }).collect();
                    self.rows_cover(&types, &rows)
                })
            }
            Type::Bool => [true, false].iter().all(|b| {
                let rows: Vec<Vec<Option<&Pattern>>> = rows.iter().filter(|row| match head(row) {
                    None => true,
                    Some(Pattern::Literal(expr)) => matches!(expr.kind, ExprKind::Boolean(v) if v == *b),
                    Some(_) => false,
                }).map(|row| tail(row)).collect();
                self.rows_cover(rest, &rows)
            }),
            // The values of a tuple come in front of the rest
            Type::Tuple(items) => {
                let mut types = items.clone();
                types.extend(rest.iter().cloned());
                let rows: Vec<Vec<Option<&Pattern>>> = rows.iter().filter_map(|row| {
                    let mut flat: Vec<Option<&Pattern>> = match head(row) {
                        None => vec![None; items.len()],
                        Some(Pattern::Tuple(parts)) if parts.len() == items.len() => parts.iter().map(Some).collect(),
                        Some(_) => return None,
                    };
                    flat.extend(tail(row));
                    Some(flat)
                }).collect();
                self.rows_cover(&types, &rows)
            }
            _ => {
                let any: Vec<Vec<Option<&Pattern>>> = rows.iter().filter(|row| head(row).is_none()).map(|row| tail(row)).collect();
                if self.rows_cover(rest, &any) {
                    return true;
                }
                // List patterns of every length, like `[]` and `[first, ..rest]`, with rows matching anything after them
                let Some((element, None)) = self.list_element(first) else { return false };
                let tail_matches_anything = |row: &Vec<Option<&Pattern>>| row[1..].iter().zip(rest)
                    .all(|(pattern, ty)| pattern.is_none_or(|pattern| self.irrefutable(pattern, ty)));
                let lists: Vec<&Pattern> = rows.iter().filter(|row| tail_matches_anything(row)).filter_map(|row| row[0]).collect();
                self.lists_cover_every_length(&element, &lists)
            }
        }
    }

    /// Checks `expr` knowing it should have the type `expected`. A generic call
    /// uses that to infer its type arguments, like `T = int64` in
    /// `let x: int64 = max(3, 7);`.
    fn check_expr_expecting(&mut self, expr: &mut Expr, expected: &Type) -> Type {
        self.expected_type = Some(expected.clone());
        self.check_expr(expr)
    }

    /// Checks `expr` and returns its type. Names bound by an `is` in it are
    /// gone afterwards, unless it's a condition keeping them, see
    /// [`Sema::keep_is_bindings`].
    fn check_expr(&mut self, expr: &mut Expr) -> Type {
        let keep = std::mem::take(&mut self.keep_is_bindings);
        let mark = self.is_bindings.len();
        let ty = self.check_expr_inner(expr);
        if !keep {
            self.drop_is_bindings(mark);
        }
        ty
    }

    fn check_expr_inner(&mut self, expr: &mut Expr) -> Type {
        let span = expr.span.clone();
        // Only meant for this expression, not for the ones nested in it
        let expected = self.expected_type.take();
        let borrowing = std::mem::take(&mut self.borrowing);
        let field_object = std::mem::take(&mut self.field_object);

        if let Some(builtin) = self.heap_call(expr) {
            let ty = self.check_heap_call(builtin, expr, expected.as_ref());
            expr.ty = ty.clone();
            return ty;
        }
        if self.is_buffer_call(expr) {
            let ty = self.check_buffer_call(expr, expected.as_ref());
            expr.ty = ty.clone();
            return ty;
        }
        if matches!(expr.kind, ExprKind::Index { .. }) {
            let ty = self.check_index(expr);
            // An array element used as a value is moved out of the array, other
            // indexing is a method call like `get`, which gives a copy
            if !borrowing && matches!(expr.kind, ExprKind::Index { .. }) && self.moves(&ty) {
                self.move_out_of(expr, &place_name(expr), &span);
            }
            expr.ty = ty.clone();
            if !borrowing {
                self.copy_shared(expr, &ty);
            }
            return ty;
        }
        if let ExprKind::Assignment { lhs, .. } = &expr.kind {
            if matches!(lhs.kind, ExprKind::Index { .. }) {
                if let Some(ty) = self.check_index_assignment(expr) {
                    expr.ty = ty.clone();
                    return ty;
                }
            }
        }
        // `Shape::Rect(w, h)` gives the variant the tuple `(w, h)`, like `Shape::Rect((w, h))`.
        // The parser sees a call with several arguments, like `fs::write(path, text)`
        if let ExprKind::FunctionCall { name, receiver: None, params, type_args, .. } = &mut expr.kind {
            let variant = name.rsplit_once("::").filter(|_| params.len() > 1).and_then(|(owner, tag)| {
                let resolved = self.aliases.get(owner).cloned().unwrap_or_else(|| owner.to_string());
                self.is_enum_name(&resolved).then(|| (owner.to_string(), tag.to_string()))
            });
            if let Some((owner, tag)) = variant {
                let payload = Expr::new(ExprKind::Tuple(std::mem::take(params)), span.clone());
                expr.kind = ExprKind::EnumInit { name: owner, type_args: std::mem::take(type_args), tag, value: Box::new(payload) };
            }
        }
        // An imported enum like `ExitCode` of `import std::process::{ExitCode};` is `process::ExitCode`,
        // an imported class like `Box` in `Box { value: 4 }` after `import util;` is `util::Box`
        if let ExprKind::EnumInit { name, .. } | ExprKind::ColonAccess { name, .. } | ExprKind::StructInit { class: name, .. } = &mut expr.kind {
            match self.aliases.get(name.as_str()) {
                Some(qualified) => *name = qualified.clone(),
                // `Type::Boolean` where two imported modules have a `Type`
                None => self.check_unambiguous(name, &span),
            }
        }

        // `fs::read(path)` parses like the enum variant `Name::Variant(payload)`,
        // `math::pi` like one without payload, but they're function calls
        let module_item = match &mut expr.kind {
            ExprKind::EnumInit { name, type_args, tag, value } if self.is_module_prefix(name) => {
                let value = std::mem::replace(value.as_mut(), Expr::new(ExprKind::Boolean(false), span.clone()));
                Some((format!("{}::{}", name, tag), std::mem::take(type_args), vec![value]))
            }
            ExprKind::ColonAccess { name, tag, .. } if self.is_module_prefix(name) => {
                let qualified = self.resolve_name(&format!("{}::{}", name, tag), &span);
                SemanticError::UnknownIdentifier { identifier: qualified, span: span.clone() }.throw(&self.source)
            }
            _ => None,
        };
        if let Some((name, type_args, params)) = module_item {
            expr.kind = ExprKind::FunctionCall { name, type_args, params, receiver: None, mangled: None };
        }

        // `T::default()` is the default value of `T`, see `std::default`: its impl's
        // `default`, or the constructor taking no arguments
        if let ExprKind::FunctionCall { name, receiver: None, params, type_args, mangled } = &mut expr.kind {
            if let (Some(owner), true) = (name.strip_suffix("::default"), params.is_empty() && type_args.is_empty()) {
                if let Some((target, ty)) = self.default_call(&owner.to_string(), &span) {
                    *name = target.clone();
                    *mangled = Some(target.clone());
                    self.use_method(&target, &span);
                    expr.ty = ty.clone();
                    return ty;
                }
            }
        }

        // `Block::create(name)` calls the function `create` of the type `Block`
        // that has no `self`, mangled like its methods as `Block_create`
        let static_call = match &mut expr.kind {
            ExprKind::EnumInit { name, tag, value, type_args } => self.static_function(name, tag).map(|function| {
                let value = std::mem::replace(value.as_mut(), Expr::new(ExprKind::Boolean(false), span.clone()));
                (function, std::mem::take(type_args), vec![value])
            }),
            ExprKind::FunctionCall { name, receiver: None, params, type_args, .. } => name.rsplit_once("::")
                .and_then(|(owner, function)| self.static_function(owner, function))
                .map(|function| (function, std::mem::take(type_args), std::mem::take(params))),
            _ => None,
        };
        if let Some((name, type_args, params)) = static_call {
            expr.kind = ExprKind::FunctionCall { name, type_args, params, receiver: None, mangled: None };
        }

        // `Ok(x)`, `Err(e)` and `Some(x)` are variants of `Result` and `Option`,
        // `None` too, unless a function or variable has that name
        let variant = match &mut expr.kind {
            ExprKind::FunctionCall { name, receiver: None, params, type_args, .. }
                if !params.is_empty() && !self.functions.contains_key(name.as_str()) && !self.generic_functions.contains_key(name.as_str()) =>
            {
                prelude_variant_enum(name).map(|enum_name| {
                    // `Ok(a, b)` holds the tuple `(a, b)`
                    let value = match params.len() {
                        1 => params.remove(0),
                        _ => Expr::new(ExprKind::Tuple(std::mem::take(params)), span.clone()),
                    };
                    let enum_name = self.variant_enum(enum_name);
                    ExprKind::EnumInit { name: enum_name, type_args: std::mem::take(type_args), tag: name.clone(), value: Box::new(value) }
                })
            }
            ExprKind::Identifier(name) if name == "None" && !self.scope.contains_key(name.as_str()) => {
                let enum_name = self.variant_enum("Option");
                Some(ExprKind::ColonAccess { name: enum_name, type_args: vec![], tag: name.clone() })
            }
            _ => None,
        };
        if let Some(kind) = variant {
            expr.kind = kind;
        }

        // `a ?? b` is the value in `a`, or `b` if there's none
        if matches!(&expr.kind, ExprKind::BinaryExpr { op: Token::QuestionQuestion, .. }) {
            self.rewrite_default(expr, expected.as_ref());
        }

        // A reference where one is wanted, like a `T` that's `&String` in
        // `Option<&String>`, is copied as the reference
        let copies_ref = match &expr.kind {
            ExprKind::FunctionCall { name, receiver: Some(receiver), params, type_args, .. }
                if name == "clone" && params.is_empty() && type_args.is_empty() => match self.probe_type(receiver) {
                    ty @ Type::Ref(_) => expected.as_ref() == Some(&ty),
                    _ => false,
                },
            _ => false,
        };
        if copies_ref {
            let ExprKind::FunctionCall { receiver: Some(receiver), .. } = &mut expr.kind else { unreachable!() };
            let mut receiver = std::mem::replace(receiver.as_mut(), Expr::new(ExprKind::Boolean(false), span.clone()));
            let ty = self.check_receiver(&mut receiver);
            *expr = receiver;
            return ty;
        }

        // `x.clone()` copies any value with everything it owns, unless its type has an own `clone`
        let builtin_clone = match &expr.kind {
            ExprKind::FunctionCall { name, receiver: Some(receiver), params, type_args, .. }
                if name == "clone" && params.is_empty() && type_args.is_empty() => !self.has_method(receiver, "clone", &span),
            _ => false,
        };
        if builtin_clone {
            let ExprKind::FunctionCall { receiver: Some(receiver), mangled, .. } = &mut expr.kind else { unreachable!() };
            let ty = match self.check_receiver(receiver) {
                Type::Ref(inner) => *inner,
                other => other,
            };
            *mangled = Some(CLONE.to_string());
            expr.ty = ty.clone();
            return ty;
        }

        // `a.get()` of a `Shared` or `Unique` refers to the value it holds
        let heap_get = match &expr.kind {
            ExprKind::FunctionCall { name, receiver: Some(receiver), params, type_args, .. }
                if name == "get" && params.is_empty() && type_args.is_empty() => heap_inner(&self.probe_type(receiver)).is_some(),
            _ => false,
        };
        if heap_get {
            let ExprKind::FunctionCall { receiver: Some(receiver), mangled, .. } = &mut expr.kind else { unreachable!() };
            let inner = heap_inner(&self.check_receiver(receiver)).expect("probed as a Shared or Unique");
            *mangled = Some(GET.to_string());
            let ty = Type::Ref(Box::new(inner));
            expr.ty = ty.clone();
            return ty;
        }

        // A slice knows its length
        let slice_len = match &expr.kind {
            ExprKind::FunctionCall { name, receiver: Some(receiver), params, .. } if name == "len" && params.is_empty() =>
                matches!(peel(self.probe_type(receiver)), Type::Slice(_)),
            _ => false,
        };
        if slice_len {
            let ExprKind::FunctionCall { receiver: Some(receiver), mangled, .. } = &mut expr.kind else { unreachable!() };
            self.check_receiver(receiver);
            *mangled = Some(SLICE_LEN.to_string());
            expr.ty = Type::UINT64;
            return Type::UINT64;
        }

        // The length of an array is known while compiling
        if let ExprKind::FunctionCall { name, receiver: Some(receiver), params, .. } = &expr.kind {
            if name == "len" && params.is_empty() {
                if let Type::Array(_, len) = peel(self.probe_type(receiver)) {
                    expr.kind = ExprKind::IntLiteral(len as i64);
                    expr.ty = Type::UINT64;
                    return Type::UINT64;
                }
            }
        }

        // An operator on strings, which becomes a call of a `String` method once its sides are checked
        let mut string_operation = None;
        let ty = match &mut expr.kind {
            ExprKind::Boolean(_) => Type::Bool,
            ExprKind::Unique(inner) => {
                self.check_expr(inner);
                Type::Unique(Box::new(inner.ty.clone()))
            }
            ExprKind::Shared(inner) => {
                self.check_expr(inner);
                Type::Shared(Box::new(inner.ty.clone()))
            }
            // Made from a checked reference, see `read_through`
            ExprKind::Deref(_) => expr.ty.clone(),
            // Made from a checked default of `??`, see `rewrite_default`
            ExprKind::Kept(_) => expr.ty.clone(),
            ExprKind::NewBuffer(capacity) => {
                self.check_expr(capacity);
                expr.ty.clone()
            }
            ExprKind::IntLiteral(value) => {
                // Reported at the end of the module unless the context gives it a bigger type
                if !int_fits(*value, 32, true) {
                    self.oversized_literals.insert((span.line, span.col), (*value, span.clone()));
                }
                Type::INT32
            }
            ExprKind::FloatLiteral(_) => Type::FLOAT64,
            ExprKind::StringLiteral(_) => Type::string(),
            ExprKind::Identifier(name) => {
                let ty = self.use_variable(name, &span);
                // Using a value that owns memory moves it out of the variable, unless it's only read
                if !borrowing && self.moves(&ty) {
                    self.move_variable(&name.clone(), &span);
                }
                ty
            }
            ExprKind::ModuleImport { .. } => Type::Void,
            ExprKind::FieldAccess { object, field } => {
                // Other fields of the object may have been moved out already
                let recv_ty = match &object.kind {
                    ExprKind::Identifier(name) => {
                        let ty = self.use_variable_field(name, &object.span);
                        object.ty = ty.clone();
                        ty
                    }
                    _ => {
                        self.field_object = true;
                        self.check_borrowed(object, None)
                    }
                };
                // This field itself can't be moved out, and only be used as a whole if nothing was moved out of it
                if root_variable(object).is_some() {
                    let path = format!("{}.{}", place_name(object), field);
                    let prefix = format!("{}.", path);
                    let moved = self.moved.iter()
                        .find(|moved| **moved == path || (!field_object && moved.starts_with(&prefix)));
                    if let Some(moved) = moved {
                        SemanticError::UseAfterMove { var: moved.clone(), span: span.clone() }.throw(&self.source);
                    }
                }

                // Through a reference to a `Unique` or `Shared`, like `b.get(0).x`, the fields
                // are those of the value it holds
                let mut recv_ty = recv_ty;
                if matches!(&recv_ty, Type::Ref(inner) if matches!(inner.as_ref(), Type::Unique(_) | Type::Shared(_))) {
                    self.get_heap_value(object);
                    recv_ty = object.ty.clone();
                }
                // Fields of a referenced value or one in a `Unique` or `Shared` are accessed directly
                let peeled = match &recv_ty {
                    Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) => (**inner).clone(),
                    other => other.clone(),
                };
                let field_name = field.clone();

                // `t.0` is the first value of a tuple
                if let Type::Tuple(items) = &peeled {
                    let item_ty = field_name.parse::<usize>().ok().and_then(|i| items.get(i).cloned())
                        .unwrap_or_else(|| diagnostic::throw(format!("'{}' has no value '{}'", peeled, field_name), Some(span.clone()), &self.source));
                    if !borrowing && self.moves(&item_ty) {
                        let what = format!("{}.{}", place_name(object), field_name);
                        self.move_out_of(object, &what, &span);
                    }
                    expr.ty = item_ty.clone();
                    if !borrowing {
                        self.copy_shared(expr, &item_ty);
                    }
                    return item_ty;
                }

                if let Type::Enum(enum_name) = &peeled {
                    if field_name == "tag" {
                        expr.ty = Type::INT32;
                        return Type::INT32;
                    }

                    SemanticError::UnknownField {
                        class: enum_name.clone(),
                        field: field_name,
                        span: span.clone(),
                    }.throw(&self.source);
                }

                let class_name = match peeled {
                    Type::Class(c) => c,
                    other => SemanticError::InvalidReceiver {
                        recv_ty: other,
                        span: span.clone(),
                        context: "is not an object",
                    }.throw(&self.source),
                };

                let (field_ty, visibility) = self.classes.get(&class_name)
                    .and_then(|fields| fields.iter().find(|p| p.name == field_name))
                    .map(|p| (p.ty.clone(), p.visibility.clone()))
                    .unwrap_or_else(|| SemanticError::UnknownField {
                        class: class_name.clone(),
                        field: field_name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                let inside_self = match self.current_class.clone() {
                    Some(name) => {
                        class_name == name
                    }
                    None => false,
                };

                if matches!(visibility, Visibility::Private) && !inside_self {
                    SemanticError::PrivateFieldAccess{ field_name, span  }.throw(&self.source);
                }

                // A field used as a value is moved out of its object
                if !borrowing && self.moves(&field_ty) {
                    let what = format!("{}.{}", place_name(object), field_name);
                    self.move_out_of(object, &what, &span);
                }
                field_ty
            },
            ExprKind::Assignment { lhs, rhs } => {
                if let ExprKind::Identifier(name) = &lhs.kind {
                    let (_, mutable) = self.scope.get(name).cloned()
                        .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                            identifier: name.clone(),
                            span: span.clone(),
                        }.throw(&self.source));
                    if !mutable {
                        SemanticError::AssignOnUnmutable {
                            var: name.clone(),
                            span: span.clone(),
                        }.throw(&self.source)
                    }
                }

                // `a.b.c = value` changes the variable `a`
                if let (ExprKind::FieldAccess { .. } | ExprKind::Index { .. }, Some(name)) = (&lhs.kind, root_variable(lhs)) {
                    let (_, mutable) = self.scope.get(name).cloned()
                        .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                            identifier: name.to_string(),
                            span: span.clone(),
                        }.throw(&self.source));
                    if !mutable {
                        SemanticError::AssignOnUnmutable {
                            var: name.to_string(),
                            span: span.clone(),
                        }.throw(&self.source)
                    }
                }

                // Assigning to a variable doesn't use its old value, a moved `Unique` can get a new one
                let lhs_ty = match &lhs.kind {
                    ExprKind::Identifier(name) => {
                        let ty = self.scope.get(name).map(|(ty, _)| ty.clone()).unwrap();
                        // In class methods `self` only points to the object, replacing it
                        // wouldn't change the object. An enum's or built-in type's `self`
                        // is the caller's value itself, `mut &self` can replace it
                        if name == "self" && matches!(ty, Type::Class(_)) {
                            SemanticError::AssignToSelf { span: span.clone() }.throw(&self.source);
                        }
                        lhs.ty = ty.clone();
                        ty
                    }
                    // A field or element gets a new value, its old one isn't taken. A field
                    // moved out before has a value again
                    _ => {
                        let path = place_name(lhs);
                        let prefix = format!("{}.", path);
                        self.moved.retain(|moved| *moved != path && !moved.starts_with(&prefix));
                        self.check_borrowed(lhs, None)
                    }
                };
                self.check_expr_expecting(rhs, &lhs_ty);
                self.coerce(rhs, &lhs_ty);
                let rhs_ty = rhs.ty.clone();
                if lhs_ty != rhs_ty {
                    SemanticError::TypeMismatch {
                        lhs: lhs_ty,
                        rhs: rhs_ty,
                        span: span.clone(),
                        context: "assignment",
                    }.throw(&self.source);
                }
                if let ExprKind::Identifier(name) = &lhs.kind {
                    self.moved.remove(name);
                }

                rhs_ty
            },
            // Made from a checked argument, see `slices_into`
            ExprKind::SliceOf(_) => expr.ty.clone(),
            // Only an argument, see the checks of calls. Elsewhere it would outlive its values
            ExprKind::SliceLiteral(_) => diagnostic::throw(
                "`&[...]` can only be passed straight to a function, like `f(&[a, b])`",
                Some(span.clone()), &self.source),
            ExprKind::Convert { value, target } => {
                // Resolves type parameters, so `x as T` works in generic functions
                *target = self.canonicalize_type(target, &span);
                self.check_expr(value);
                // `n as uint8` with `n: &int64` converts the number `n` refers to
                read_through(value);
                // A literal takes the target type directly, so `3000000000 as int64` works
                if target.is_number() {
                    self.coerce_literal(value, target);
                }

                // Numbers convert to any number type, bools to numbers as 0 or 1, a char to and
                // from the number of its byte
                let allowed = (target.is_number() && (value.ty.is_number() || value.ty == Type::Bool || value.ty == Type::Char))
                    || (*target == Type::Char && (value.ty.is_number() || value.ty == Type::Char));
                if !allowed {
                    SemanticError::InvalidConversion {
                        from: value.ty.clone(),
                        to: target.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
                target.clone()
            }
            ExprKind::Negation(inner) => {
                let inner_ty = self.check_expr(inner);
                if !matches!(inner_ty, Type::Int { signed: true, .. } | Type::Float { .. }) {
                    SemanticError::CannotNegate { ty: inner_ty, span: span.clone() }.throw(&self.source);
                }
                inner_ty
            }
            ExprKind::Ref(inner) => {
                if !matches!(inner.kind, ExprKind::Identifier(_) | ExprKind::FieldAccess { .. }) {
                    diagnostic::throw("'&' only refers to a variable or field, like '&x' or '&x.field'", Some(span.clone()), &self.source);
                }
                // Referencing a variable borrows it, a `Unique` doesn't get moved by that
                let inner_ty = match &inner.kind {
                    ExprKind::Identifier(name) => {
                        let ty = self.use_variable(name, &span);
                        inner.ty = ty.clone();
                        ty
                    }
                    _ => self.check_borrowed(inner, None),
                };
                Type::Ref(Box::new(inner_ty))
            },
            ExprKind::BinaryExpr { lhs, rhs, op } => {
                // Operators only read their operands, `a + b` on strings makes a new one.
                // The right side of `&&` only runs if the left one is true, so it has the
                // names an `is` on the left binds. The `&&` itself keeps them if its context does
                let and = matches!(op, Token::AndAnd);
                self.keep_is_bindings = and;
                self.check_borrowed(lhs, None);
                self.keep_is_bindings = and;
                self.check_borrowed(rhs, None);
                // `x + 1` with `x: &int32` uses the number `x` refers to
                read_through(lhs);
                read_through(rhs);

                // A literal takes the type of the other side, like `1` in `x + 1` with `x: uint8`
                if is_number_literal(lhs) && !is_number_literal(rhs) {
                    self.coerce_literal(lhs, &rhs.ty);
                } else if is_number_literal(rhs) && !is_number_literal(lhs) {
                    self.coerce_literal(rhs, &lhs.ty);
                }

                // The narrower side widens to the other, like `int32 + int64` to `int64`
                if lhs.ty.widens_to(&rhs.ty) {
                    let target = rhs.ty.clone();
                    self.coerce(lhs, &target);
                } else if rhs.ty.widens_to(&lhs.ty) {
                    let target = lhs.ty.clone();
                    self.coerce(rhs, &target);
                }

                // Strings, also through references, become calls of `String` methods
                let is_text = |ty: &Type| ty.is_string() || matches!(ty, Type::Ref(inner) if inner.is_string());
                let string_op = matches!(op, Token::Plus | Token::EqEq | Token::NotEq | Token::Lt | Token::LtEq | Token::Gt | Token::GtEq);
                // `text + c` adds the char `c`
                let adds_char = matches!(op, Token::Plus) && is_text(&lhs.ty) && rhs.ty == Type::Char;
                if adds_char || (string_op && is_text(&lhs.ty) && is_text(&rhs.ty)) {
                    let placeholder = || Expr::new(ExprKind::Boolean(false), span.clone());
                    let lhs = std::mem::replace(lhs.as_mut(), placeholder());
                    let rhs = std::mem::replace(rhs.as_mut(), placeholder());
                    string_operation = Some(string_operator(lhs, op, rhs, &span));
                    if op.is_comparison() { Type::Bool } else { Type::string() }
                } else if matches!(op, Token::EqEq | Token::NotEq) && compares_contents(&lhs.ty, &rhs.ty) {
                    // Anything else compares what it holds, also through references
                    Type::Bool
                } else {
                    let (lhs_ty, rhs_ty) = (lhs.ty.clone(), rhs.ty.clone());
                    if lhs_ty != rhs_ty {
                        SemanticError::TypeMismatch {
                            lhs: lhs_ty,
                            rhs: rhs_ty,
                            span: span.clone(),
                            context: "binary expression",
                        }.throw(&self.source);
                    }

                    // `==` and `!=` work on numbers and bools, ordering on numbers, `&&` and
                    // `||` only take bools
                    let valid = match op {
                        Token::EqEq | Token::NotEq => lhs_ty.is_number() || lhs_ty == Type::Bool || lhs_ty == Type::Char,
                        // Chars sort by their byte
                        Token::Lt | Token::LtEq | Token::Gt | Token::GtEq => lhs_ty.is_number() || lhs_ty == Type::Char,
                        Token::AndAnd | Token::OrOr => lhs_ty == Type::Bool,
                        // `^` flips bits of integers, of bools it's "exactly one is true"
                        Token::Caret => matches!(lhs_ty, Type::Int { .. } | Type::Bool),
                        _ => lhs_ty.is_number(),
                    };
                    if !valid {
                        SemanticError::InvalidOperands { op: op.clone(), ty: lhs_ty, span: span.clone() }.throw(&self.source);
                    }
                    if op.is_comparison() || op.is_logical() { Type::Bool } else { lhs_ty }
                }
            }
            ExprKind::CharLiteral(_) => Type::Char,
            ExprKind::ArrayLiteral(values) => {
                // The element type comes from the expected array type, else from the first value
                let declared = match &expected {
                    Some(Type::Array(element, len)) => {
                        if values.len() as u64 != *len {
                            SemanticError::ArrayLength { expected: *len, got: values.len(), span: span.clone() }
                                .throw(&self.source);
                        }
                        Some((**element).clone())
                    }
                    _ => None,
                };
                let (element, unchecked) = match declared {
                    Some(element) => (element, &mut values[..]),
                    None => {
                        let Some((first, rest)) = values.split_first_mut() else {
                            SemanticError::EmptyArrayType { span: span.clone() }.throw(&self.source)
                        };
                        (self.check_expr(first), rest)
                    }
                };
                for value in unchecked.iter_mut() {
                    self.check_argument(value, &element, "array value");
                }
                Type::Array(Box::new(element), values.len() as u64)
            }
            // The values' types, an expected tuple type adapts them like `(1, 2)` for `(int64, uint8)`
            ExprKind::Tuple(values) => {
                let items: Vec<Type> = match &expected {
                    Some(Type::Tuple(types)) if types.len() == values.len() => {
                        for (value, ty) in values.iter_mut().zip(types) {
                            self.check_argument(value, ty, "tuple value");
                        }
                        types.clone()
                    }
                    _ => values.iter_mut().map(|value| self.check_expr(value)).collect(),
                };
                Type::Tuple(items)
            }
            ExprKind::Index { .. } => unreachable!("indexing is checked by check_index"),
            // Used as a value, every arm gives one, see `check_match_arms`
            ExprKind::Match { scrutinee, arms } => {
                // Only read, its bindings own or borrow their values like it. A `&` is
                // matched by the value it refers to
                let scrutinee_ty = match self.check_borrowed(scrutinee, None) {
                    Type::Ref(inner) => *inner,
                    other => other,
                };
                self.pattern_owner = self.owner_of(scrutinee);
                self.check_match_arms(&scrutinee_ty, arms, Some((expected.clone(), span.clone())))
            }
            ExprKind::Is { value, pattern } => self.check_is(value, pattern, &span),
            // The parser only makes it after `??`, which becomes a match before getting here
            ExprKind::Leave(_) => diagnostic::throw("'return', 'break' and 'continue' only give no value after '??'", Some(span.clone()), &self.source),
            // Used as a value, both branches give one, see `check_if`
            // `if` only runs code, values are chosen with `? :`
            ExprKind::If { .. } => diagnostic::throw(
                "an 'if' doesn't give a value, choose one with `condition ? value : other`, or use `match`",
                Some(span.clone()), &self.source),
            ExprKind::Ternary { condition, then_value, else_value } => {
                self.check_ternary(condition, then_value, else_value, Some((expected.clone(), span.clone())))
            }
            ExprKind::Not(inner) => {
                self.check_condition(inner, "'!' operand");
                Type::Bool
            }
            ExprKind::ColonAccess { name, type_args, tag } => {
                // A generic enum's instance comes from the type arguments or the expected
                // type, like `Option<int32>` in `let x: Option<int32> = Option::None;`
                let enum_name = if self.generic_types.contains_key(name.as_str()) {
                    let mut no_value = Expr::new(ExprKind::Boolean(false), span.clone());
                    self.instantiate_enum_init(&name.clone(), type_args, tag, &mut no_value, false, expected.as_ref(), &span)
                } else {
                    if !type_args.is_empty() {
                        SemanticError::NotGeneric { function: name.clone(), span: span.clone() }.throw(&self.source);
                    }
                    name.clone()
                };
                let variants = self.enums.get(&enum_name).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownEnum {
                        name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                match variants.iter().find(|(v, _, _)| v == tag) {
                    Some((_, _, Type::Void)) => {}
                    Some((_, _, payload)) => SemanticError::VariantNeedsPayload {
                        variant: format!("{}::{}", name, tag),
                        payload: payload.clone(),
                        span: span.clone(),
                    }.throw(&self.source),
                    None => SemanticError::UnknownVariant {
                        enum_name: name.clone(),
                        variant: tag.clone(),
                        span: span.clone(),
                    }.throw(&self.source),
                }

                // Built by the variant's constructor, like one with a payload
                let constructor = format!("{}__{}", enum_name, tag);
                expr.kind = ExprKind::FunctionCall {
                    name: enum_name.clone(),
                    type_args: vec![],
                    params: vec![],
                    receiver: None,
                    mangled: Some(constructor),
                };
                Type::Enum(enum_name)
            }
            ExprKind::FunctionCall { name, type_args, params, receiver, mangled } => {
                // `read` after `import std::fs::{read};` calls `fs::read`, unless this
                // module declares a `read` itself
                if receiver.is_none() {
                    let own = self.functions.get(name.as_str()).is_some_and(|f| f.3 == self.module)
                        || self.generic_functions.get(name.as_str()).is_some_and(|g| g.module == self.module);
                    if !own {
                        *name = self.resolve_name(name, &span);
                    }
                }
                // `x.f(args)` without a method `f` calls a free function `f(x, args)`,
                // so functions like `unwrap(o: Option<T>)` can be called as `o.unwrap()`
                if let Some(recv) = receiver.as_deref() {
                    // The module's own one, which has the module's name like `lexer::contains_word`,
                    // or an imported one, like `trim` of `import std::string::{trim};`
                    let is_function = |s: &Self, name: &str| s.functions.contains_key(name) || s.generic_functions.contains_key(name);
                    let own = format!("{}::{}", self.module.strip_prefix("std::").unwrap_or(&self.module), name);
                    let resolved = if is_function(self, &own) {
                        own
                    } else {
                        self.aliases.get(name.as_str()).cloned().unwrap_or_else(|| name.clone())
                    };
                    let free_function = is_function(self, &resolved);
                    if free_function && !self.has_method(recv, name, &span) {
                        let recv = receiver.take().unwrap();
                        params.insert(0, *recv);
                        *name = resolved;
                    }
                }
                // `text.try_as<int32>()` calls the trait function `try_as` of the impl for
                // `int32` that takes a text, like `impl TryAs<String> for int32`
                if let Some(recv) = receiver.as_deref() {
                    if self.is_static_trait_function(name) && !self.has_self_method(recv, name, &span) {
                        let target = match type_args.first() {
                            Some(ty) => self.canonicalize_type(ty, &span),
                            // `let n: Option<int32> = text.try_as()` wants an `int32`
                            None => expected.as_ref().and_then(|ty| self.option_value_type(ty)).unwrap_or_else(|| diagnostic::throw(
                                format!("can't tell which type '{}' should give, write it like `{}<int32>()`", name, name),
                                Some(span.clone()), &self.source)),
                        };
                        let from = peel(self.probe_type(recv));
                        let function = self.static_trait_impl(name, &from, &target).unwrap_or_else(|| {
                            let (from, target) = (crate::analysis::show_type(&from), crate::analysis::show_type(&target));
                            diagnostic::throw(
                                format!("there's no '{}' from '{}' to '{}', it needs an impl like `impl TryAs<{}> for {}`", name, from, target, from, target),
                                Some(span.clone()), &self.source)
                        });
                        let recv = receiver.take().unwrap();
                        params.insert(0, *recv);
                        *name = function;
                        type_args.clear();
                    }
                }
                // `text.try_as<int32>()`, or `try_as()` where an `int32` is expected, picks
                // the impl of `TryAs<int32>` if `String` has several impls of `TryAs`
                let target = match (receiver.is_some(), type_args.as_slice()) {
                    (true, [ty]) => Some(self.canonicalize_type(&ty.clone(), &span)),
                    (true, []) => expected.clone().filter(|ty| *ty != Type::Unknown),
                    _ => None,
                };
                // A receiver that's a call itself is resolved in between, with its own target
                let outer_target = std::mem::replace(&mut self.method_target, target);
                let outer_used = self.method_target_used.replace(false);
                let (mut m, recv_class) = self.resolve_call(name, receiver, &span);
                self.method_target = outer_target;
                if self.method_target_used.replace(outer_used) {
                    type_args.clear();
                }
                if m.starts_with("Buffer.") {
                    let receiver = receiver.as_deref().expect("Buffer methods have a receiver");
                    let ty = self.check_buffer_method(name, receiver, params, &span);
                    *mangled = Some(m);
                    expr.ty = ty.clone();
                    return ty;
                }
                // Inferring type arguments checks the arguments already. Checking them
                // again would fail, sema rewrites some of them, like struct initializers
                let mut args_checked = false;
                if !self.functions.contains_key(&m) {
                    if self.generic_functions.contains_key(&m) {
                        // A generic method like `Box<int32>_convert`, or a generic function
                        m = self.instantiate(&m.clone(), type_args, params, expected.as_ref(), &span);
                        args_checked = true;
                    } else if recv_class.is_none() && self.generic_types.contains_key(name.as_str()) {
                        // Constructor call of a generic class, like `Box(5)`
                        m = self.instantiate_constructor(&name.clone(), type_args, params, expected.as_ref(), &span);
                        args_checked = true;
                    }
                }
                // A class with several constructors: the one taking this many arguments
                if receiver.is_none() {
                    if let Some(constructor) = self.pick_constructor(&m, params.len(), &span) {
                        m = constructor;
                    }
                }
                // Unknown names fall through to the unknown function error below
                if !args_checked && !type_args.is_empty() && self.functions.contains_key(&m) {
                    SemanticError::NotGeneric { function: name.clone(), span: span.clone() }.throw(&self.source);
                }

                // Built-ins only the prelude and standard library may use don't exist for programs
                let hidden = runtime::is_internal(&m) && self.module != "prelude" && !self.module.starts_with("std::");
                // A class without a `Constructor` can't be called like one
                if receiver.is_none() && !self.functions.contains_key(&m) && self.classes.contains_key(&m) {
                    diagnostic::throw(format!("class '{}' has no Constructor", name), Some(span.clone()), &self.source);
                }
                let (defs, ret, visibility, module) = self.functions.get(&m).filter(|_| !hidden).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownFunction {
                        fn_name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                // A C function like `printf` can take more arguments than it declares
                let is_extern = self.externs.contains_key(&m);
                let variadic = self.externs.get(&m).is_some_and(|e| e.variadic);
                if params.len() > defs.len() && !variadic {
                    SemanticError::TooManyArguments{ func_name: name.clone(), parent: module, span: span.clone() }
                        .throw(&self.source);
                }

                // A method changing `self` needs a receiver that may be changed
                if defs.first().is_some_and(|p| p.name == "self" && p.mutable) {
                    // `a.b.change()` changes the variable `a`. A temporary receiver like
                    // the result of a call has no variable, changing it is fine
                    if let Some(variable) = receiver.as_deref().and_then(root_variable) {
                        if self.scope.get(variable).is_some_and(|(_, mutable)| !mutable) {
                            SemanticError::MutatingCallOnImmutable {
                                method: name.clone(),
                                var: variable.to_string(),
                                span: span.clone(),
                            }.throw(&self.source);
                        }
                    }
                }

                // Free functions from other modules need to be public.
                if recv_class.is_none() && matches!(visibility, Visibility::Private) && module != self.module {
                    SemanticError::PrivateFunctionCall { func_name: self.written_name(name), parent: module, span: span.clone() }
                        .throw(&self.source);
                }

                // Private methods can only be called from inside their own class.
                if let Some(cls) = recv_class {
                    if matches!(visibility, Visibility::Private) && self.current_class.as_ref() != Some(&cls) {
                        SemanticError::PrivateFunctionCall { func_name: name.clone(), parent: cls, span: span.clone() }
                            .throw(&self.source);
                    }
                }

                *mangled = Some(m.clone());
                self.use_method(&m, &span);

                let arg_defs: Vec<_> = defs.iter().filter(|d| d.name != "self").collect();
                // C writes a handle through an out-parameter like `&target`, which needs a
                // `mut` variable or a field of one to go into
                if is_extern {
                    for (def, arg) in arg_defs.iter().zip(params.iter()) {
                        let Type::Ref(inner) = &def.ty else { continue };
                        if !self.is_out_param(inner) {
                            continue;
                        }
                        // `&target`, or a reference to one passed on, like a parameter `mut error: &Option<...>`
                        let passes_reference = matches!(&arg.kind, ExprKind::Identifier(variable)
                            if self.scope.get(variable).is_some_and(|(ty, _)| matches!(ty, Type::Ref(_))));
                        let variable = match &arg.kind {
                            ExprKind::Ref(place) => root_variable(place),
                            _ if passes_reference => root_variable(arg),
                            _ => None,
                        };
                        match variable {
                            None => diagnostic::throw(
                                format!("'{}' writes into this argument, pass a variable like `&target`", self.written_name(name)),
                                Some(arg.span.clone()), &self.source),
                            Some(variable) if self.scope.get(variable).is_some_and(|(_, mutable)| !mutable) => diagnostic::throw(
                                format!("'{}' writes into '{}', declare it with `mut`", self.written_name(name), variable),
                                Some(arg.span.clone()), &self.source),
                            Some(_) => {}
                        }
                    }
                }
                for (i, def) in arg_defs.iter().enumerate() {
                    match params.get_mut(i) {
                        Some(arg) if matches!(arg.kind, ExprKind::SliceLiteral(_)) => {
                            // `&[a, b]` for a `&[T]` parameter: an array of the values, lent for the call
                            let Type::Slice(element) = &def.ty else {
                                diagnostic::throw(format!("`&[...]` is a slice, but this parameter takes '{}'", def.ty), Some(arg.span.clone()), &self.source)
                            };
                            let element = (**element).clone();
                            let ExprKind::SliceLiteral(values) = &mut arg.kind else { unreachable!() };
                            for value in values.iter_mut() {
                                self.check_argument(value, &element, "slice value");
                            }
                            let values = std::mem::take(values);
                            let len = values.len() as u64;
                            let array = Expr { kind: ExprKind::ArrayLiteral(values), span: arg.span.clone(), ty: Type::Array(Box::new(element), len) };
                            arg.kind = ExprKind::SliceOf(Box::new(array));
                            arg.ty = def.ty.clone();
                        }
                        Some(arg) => {
                            if !args_checked {
                                // A `&` parameter only borrows the argument, and so does C, which gets a
                                // copy of a string. Every other parameter takes it
                                let borrowed = matches!(def.ty, Type::Ref(_) | Type::Slice(_)) || (is_extern && def.ty.is_string());
                                if borrowed {
                                    self.check_borrowed(arg, Some(&def.ty));
                                } else {
                                    self.check_expr_expecting(arg, &def.ty);
                                }
                            }
                            // An array or Vector passed as a slice, which looks at its elements
                            if let Type::Slice(element) = &def.ty {
                                if self.slices_into(&arg.ty, element) {
                                    let span = arg.span.clone();
                                    let value = std::mem::replace(arg, Expr::new(ExprKind::Boolean(false), span.clone()));
                                    *arg = Expr { kind: ExprKind::SliceOf(Box::new(value)), span, ty: def.ty.clone() };
                                }
                            }
                            self.coerce(arg, &def.ty);
                            let arg_ty = arg.ty.clone();
                            let matches_ty = arg_ty == def.ty
                                || matches!(&def.ty, Type::Ref(inner) if **inner == arg_ty);
                            if !matches_ty {
                                SemanticError::TypeMismatch {
                                    lhs: arg_ty,
                                    rhs: def.ty.clone(),
                                    span: arg.span.clone(),
                                    context: "function argument",
                                }.throw(&self.source);
                            }
                        }
                        None if def.value.is_none() => {
                            SemanticError::MissingArgument {
                                function: m.clone(),
                                index: i,
                                span: span.clone(),
                            }.throw(&self.source);
                        }
                        None => {}
                    }
                }
                // The extra arguments of a C function get C's types, strings are only read
                for arg in params.iter_mut().skip(arg_defs.len()) {
                    let ty = self.check_borrowed(arg, None);
                    if !(ty.is_number() || ty.is_string() || ty == Type::Bool) {
                        SemanticError::ExternType { ty, span: arg.span.clone() }.throw(&self.source);
                    }
                }

                ret
            }
            ExprKind::Print(inner) => {
                // A type with an `impl Print` of `std::string` prints its `to_string()`,
                // numbers, bools, chars and strings are printed directly
                let printed = peel(self.probe_type(inner));
                let builtin = printed.is_number() || matches!(printed, Type::Bool | Type::Char) || printed.is_string();
                if !builtin && self.implements(&printed, PRINT_TRAIT) {
                    let receiver = std::mem::replace(inner.as_mut(), Expr::new(ExprKind::Boolean(false), span.clone()));
                    **inner = Expr::new(ExprKind::FunctionCall {
                        name: "to_string".to_string(),
                        type_args: vec![],
                        params: vec![],
                        receiver: Some(Box::new(receiver)),
                        mangled: None,
                    }, span.clone());
                } else if matches!(printed, Type::Class(_)) && !builtin {
                    diagnostic::throw(
                        format!("'{}' can't be printed, give it an `impl Print` with a `to_string` (see `std::string`)",
                            written_type(&crate::analysis::show_type(&printed))),
                        Some(span.clone()),
                        &self.source,
                    );
                }
                self.check_borrowed(inner, None);
                // A reference to a number prints the number, not where it is
                read_through(inner);
                let inner_ty = inner.ty.clone();
                if inner_ty.is_move_only() || matches!(inner_ty, Type::Shared(_)) {
                    SemanticError::CannotPrintUnique { span: span.clone() }.throw(&self.source);
                }
                if matches!(inner_ty, Type::Array(..)) {
                    SemanticError::CannotPrintArray { span: span.clone() }.throw(&self.source);
                }
                Type::Void
            }
            ExprKind::EnumInit { .. } => self.check_struct_init(expr, expected.as_ref()),
            ExprKind::StructInit { .. } => self.check_struct_init(expr, expected.as_ref())
        };
        if let Some(kind) = string_operation {
            expr.kind = kind;
        }

        expr.ty = ty.clone();
        if !borrowing {
            self.copy_shared(expr, &ty);
        }
        ty
    }

    /// Makes `expr` have the type `target` where that's possible without losing
    /// information.
    ///
    /// Number literals take the type directly (see [`Sema::coerce_literal`]),
    /// other values get wrapped in an [`ExprKind::Convert`] if their type
    /// [widens](Type::widens_to) to `target`. Anything else is left unchanged,
    /// so the caller's type check reports the mismatch.
    ///
    /// # Panics
    ///
    /// Panics if an integer literal doesn't fit into `target`.
    fn coerce(&mut self, expr: &mut Expr, target: &Type) {
        // A `T` where a `Result<T, E>` is expected is its `Ok`, like `return 5;` in a
        // function giving a `Result<int32, String>`, and where an `Option<T>` is
        // expected its `Some`. An error is always written `Err(...)`
        let wrapper = match self.result_types(target) {
            Some((ok_ty, err_ty)) => Some(("Ok", ok_ty, Some(err_ty))),
            None => self.option_value_type(target).map(|value_ty| ("Some", value_ty, None)),
        };
        if let (Some((variant, value_ty, err_ty)), Type::Enum(instance)) = (wrapper, target) {
            if expr.ty == *target || expr.ty == Type::Unknown {
                return;
            }
            self.coerce(expr, &value_ty);
            if expr.ty == value_ty {
                let span = expr.span.clone();
                let value = std::mem::replace(expr, Expr::new(ExprKind::Boolean(false), span.clone()));
                *expr = Expr {
                    kind: ExprKind::FunctionCall {
                        name: instance.clone(),
                        type_args: vec![],
                        params: vec![value],
                        receiver: None,
                        mangled: Some(format!("{}__{}", instance, variant)),
                    },
                    span,
                    ty: target.clone(),
                };
            } else if err_ty.is_some_and(|err_ty| expr.ty == err_ty) {
                diagnostic::throw(
                    format!("an error has to be written `Err(...)`, only values of type '{}' become `Ok(...)` by themselves", value_ty),
                    Some(expr.span.clone()), &self.source);
            }
            return;
        }
        // `&T` from a `Shared<T>` or `Unique<T>`, a `T` from a reference to a number
        if let Type::Ref(wanted) = target {
            if heap_inner(&expr.ty).is_some_and(|inner| inner == **wanted || same_bytes(&inner, wanted)) {
                self.get_heap_value(expr);
            }
        }
        // A `char` is one byte: a Buffer of chars is one of bytes and the other way round
        let byte_buffers = match (&expr.ty, target) {
            (Type::Ref(inner), Type::Ref(wanted)) => same_bytes(inner, wanted),
            // Lent to a `&Buffer<uint8>` parameter as it is
            (ty, Type::Ref(wanted)) => same_bytes(ty, wanted),
            (ty, wanted) => same_bytes(ty, wanted),
        };
        if byte_buffers {
            expr.ty = match (&expr.ty, target) {
                (Type::Ref(_), _) => target.clone(),
                (_, Type::Ref(wanted)) => (**wanted).clone(),
                _ => target.clone(),
            };
        }
        if matches!(&expr.ty, Type::Ref(inner) if **inner == *target) {
            read_through(expr);
        }
        // An `IntType` where its base `Type` is expected: the same pointer, only seen as the base
        let kind_of = |ty: &Type, target: &Type| match (ty, target) {
            (Type::Handle(handle), Type::Handle(base)) => handle != base && self.is_kind_of(handle, base),
            _ => false,
        };
        // A slice can't change its elements, so `&[IntType]` is also safe to see as `&[Type]`
        let upcast = match (&expr.ty, target) {
            (Type::Ref(inner), Type::Ref(base)) | (Type::Slice(inner), Type::Slice(base)) => kind_of(inner, base),
            (ty, target) => kind_of(ty, target),
        };
        if upcast {
            expr.ty = target.clone();
        }

        self.coerce_literal(expr, target);
        if !expr.ty.widens_to(target) {
            return;
        }

        let span = expr.span.clone();
        let inner = std::mem::replace(expr, Expr::new(ExprKind::IntLiteral(0), span.clone()));
        *expr = Expr { kind: ExprKind::Convert { value: Box::new(inner), target: target.clone() }, span, ty: target.clone() };
    }

    /// Gives a number literal the type `target`, if it can take it.
    ///
    /// Without context, integer literals are `int32` and float literals `float64`.
    /// Where the context expects another number type, like in `let x: uint8 = 5;`,
    /// the literal takes that type instead. This also works for expressions made
    /// only of literals, like `2 + 3`. Anything else is left unchanged, so the
    /// caller's type check reports the mismatch.
    ///
    /// # Panics
    ///
    /// Panics if an integer literal doesn't fit into `target`.
    fn coerce_literal(&mut self, expr: &mut Expr, target: &Type) {
        // Also runs when the type already matches, so the range still gets checked
        if !is_number_literal(expr) {
            return;
        }

        match (&mut expr.kind, target) {
            (ExprKind::IntLiteral(value), Type::Int { bits, signed }) => {
                if !int_fits(*value, *bits, *signed) {
                    SemanticError::LiteralOutOfRange {
                        value: *value,
                        ty: target.clone(),
                        span: expr.span.clone(),
                    }.throw(&self.source);
                }
                self.oversized_literals.remove(&(expr.span.line, expr.span.col));
            }
            (ExprKind::FloatLiteral(_), Type::Float { .. }) => {}
            // `'a'` is also the byte of the letter, for code working with bytes
            (ExprKind::CharLiteral(_), Type::Int { bits: 8, signed: false }) => {}
            (ExprKind::BinaryExpr { lhs, rhs, .. }, _) => {
                self.coerce_literal(lhs, target);
                self.coerce_literal(rhs, target);
                if lhs.ty != *target || rhs.ty != *target {
                    return;
                }
            }
            _ => return,
        }

        expr.ty = target.clone();
    }

    /// Checks the object of a field access or method call, like `a.b` in
    /// `a.b.get()`, and returns its type.
    ///
    /// Unlike using a variable as a value, this doesn't move a `Unique` out
    /// of it, the object stays where it is.
    ///
    /// # Panics
    ///
    /// Panics if the object contains an error or a variable in it got moved.
    fn check_receiver(&mut self, object: &mut Expr) -> Type {
        let ty = match &object.kind {
            ExprKind::Identifier(name) => self.use_variable(name, &object.span),
            // Like `a.b` in `a.b.c` or `a.b.len()`, only read
            _ => return self.check_borrowed(object, None),
        };
        object.ty = ty.clone();
        ty
    }

    /// Resolves which function a call refers to.
    ///
    /// Returns the mangled name and, for method calls like `obj.method()`, the
    /// [owner name](Type::owner_name) of the receiver's type.
    ///
    /// # Panics
    ///
    /// Panics if the receiver is unknown or its type can't have methods.
    fn resolve_call(&mut self, name: &str, receiver: &mut Option<Box<Expr>>, span: &Span) -> (String, Option<String>) {
        let Some(receiver) = receiver else {
            // Without a receiver a call goes to a free function. A method of the
            // surrounding class has to be called as `self.f()` or `Class::f()`
            if let Some(cls) = self.current_class.clone() {
                let free = self.functions.contains_key(name) || self.generic_functions.contains_key(name);
                if cls != name && !free {
                    let method = self.find_method(&cls, name, span);
                    let takes_self = match (self.functions.get(&method), self.generic_functions.get(&method)) {
                        (Some((defs, ..)), _) => Some(defs.first().is_some_and(|p| p.name == "self")),
                        (None, Some(generic)) => Some(matches!(&generic.decl.kind,
                            DeclarationKind::Function { params, .. } if params.first().is_some_and(|p| p.name == "self"))),
                        (None, None) => None,
                    };
                    if let Some(takes_self) = takes_self {
                        let fix = if takes_self { format!("self.{}(...)", name) } else { format!("{}::{}(...)", cls, name) };
                        diagnostic::throw(
                            format!("'{}' is a method of '{}', call it as `{}`", name, cls, fix),
                            Some(span.clone()),
                            &self.source,
                        );
                    }
                }
            }
            return (name.to_string(), None);
        };

        let recv_ty = match self.check_receiver(receiver) {
            // Methods of a referenced value or one in a `Unique` or `Shared` are called directly on it
            Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) => *inner,
            other => other,
        };

        // `Buffer` methods are built in, marked like `Buffer.get` for `check_buffer_method`.
        // Others come from impls for the Buffer type
        if matches!(recv_ty, Type::Buffer(_)) {
            if is_buffer_method(name) {
                return (format!("Buffer.{}", name), Some("Buffer".to_string()));
            }
            self.see_buffer(&recv_ty);
        }

        match recv_ty.owner_name() {
            Some(owner) => (self.find_method(&owner, name, span), Some(owner)),
            None => SemanticError::InvalidReceiver {
                recv_ty,
                span: span.clone(),
                context: "has no methods",
            }.throw(&self.source),
        }
    }

    /// The type of `receiver` used as the object of a method call or field
    /// access, found by checking a copy of it, so checking the real one later
    /// doesn't count its moves twice.
    fn probe_type(&mut self, receiver: &Expr) -> Type {
        let moved = self.moved.clone();
        let mut probe = receiver.clone();
        let ty = self.check_receiver(&mut probe);
        self.moved = moved;
        ty
    }

    /// Checks `object[index]`. Arrays are indexed directly, a `String` gives its
    /// byte and anything else, like a `Vector` or `Buffer`, calls its `get`.
    /// The latter two turn `expr` into that method call.
    ///
    /// # Panics
    ///
    /// Panics if the object can't be indexed or the index isn't an integer.
    fn check_index(&mut self, expr: &mut Expr) -> Type {
        let span = expr.span.clone();
        let ExprKind::Index { object, index } = &mut expr.kind else { unreachable!("only indexing gets here") };
        let object_ty = peel(self.check_receiver(object));
        if let Type::Array(element, len) = object_ty {
            self.check_array_index(index, len);
            return *element;
        }
        // A slice's length is only known while running, it's checked then
        if let Type::Slice(element) = object_ty {
            self.index_as_uint64(index);
            return *element;
        }

        let method = if object_ty.is_string() { "char_at" } else { "get" };
        if !object_ty.is_string() && !self.has_method(object, "get", &span) {
            SemanticError::NotIndexable { ty: object_ty, span }.throw(&self.source);
        }
        self.index_as_uint64(index);
        let placeholder = || Box::new(Expr::new(ExprKind::Boolean(false), span.clone()));
        let object = std::mem::replace(object, placeholder());
        let index = std::mem::replace(index, placeholder());
        expr.kind = ExprKind::FunctionCall {
            name: method.to_string(),
            type_args: vec![],
            params: vec![*index],
            receiver: Some(object),
            mangled: None,
        };
        self.check_expr(expr)
    }

    /// Checks `object[index] = value` for anything but an array, which turns
    /// into a call of the object's `set`. Returns `None` for arrays, they're
    /// assigned like any other place.
    ///
    /// # Panics
    ///
    /// Panics if the object is a `String`, which can't be changed, or has no
    /// `set` method.
    fn check_index_assignment(&mut self, expr: &mut Expr) -> Option<Type> {
        let span = expr.span.clone();
        let ExprKind::Assignment { lhs, rhs } = &mut expr.kind else { unreachable!("only assignments get here") };
        let ExprKind::Index { object, index } = &mut lhs.kind else { unreachable!("only index assignments get here") };
        match peel(self.probe_type(object)) {
            Type::Array(..) => return None,
            ty if ty.is_string() => SemanticError::StringImmutable { span }.throw(&self.source),
            Type::Slice(_) => diagnostic::throw("a slice only looks at the elements, they can't be changed through it", Some(span), &self.source),
            other => {
                if !self.has_method(object, "set", &span) {
                    SemanticError::NotIndexable { ty: other, span: span.clone() }.throw(&self.source);
                }
            }
        }
        self.index_as_uint64(index);
        let placeholder = || Box::new(Expr::new(ExprKind::Boolean(false), span.clone()));
        let object = std::mem::replace(object, placeholder());
        let index = std::mem::replace(index, placeholder());
        let value = std::mem::replace(rhs, placeholder());
        expr.kind = ExprKind::FunctionCall {
            name: "set".to_string(),
            type_args: vec![],
            params: vec![*index, *value],
            receiver: Some(object),
            mangled: None,
        };
        Some(self.check_expr(expr))
    }

    /// Checks the index of an array of length `len`: any integer, and a
    /// constant one has to be inside the array.
    fn check_array_index(&mut self, index: &mut Expr, len: u64) {
        let ty = self.check_expr(index);
        if !matches!(ty, Type::Int { .. }) {
            SemanticError::TypeMismatch { lhs: ty, rhs: Type::UINT64, span: index.span.clone(), context: "array index" }
                .throw(&self.source);
        }
        if let ExprKind::IntLiteral(value) = index.kind {
            if value < 0 || value as u64 >= len {
                SemanticError::ArrayIndexOutOfRange { index: value, len, span: index.span.clone() }.throw(&self.source);
            }
        }
    }

    /// Checks `index` for a `get` or `set` taking a `uint64`. Any other
    /// integer is converted, so `v[i]` works with an `int32` `i` too.
    fn index_as_uint64(&mut self, index: &mut Box<Expr>) {
        let ty = self.check_expr_expecting(index, &Type::UINT64);
        if is_number_literal(index) {
            self.coerce_literal(index, &Type::UINT64);
        } else if matches!(ty, Type::Int { .. }) && ty != Type::UINT64 {
            let span = index.span.clone();
            let value = std::mem::replace(index, Box::new(Expr::new(ExprKind::Boolean(false), span.clone())));
            let mut converted = Expr::new(ExprKind::Convert { value, target: Type::UINT64 }, span);
            converted.ty = Type::UINT64;
            **index = converted;
        }
    }

    /// Makes `values` of `for x in values` an iterator: an iterator, with a
    /// `next` method, is used as it is, anything else with an `iter` method,
    /// like an `Iterable`, becomes `values.iter()`.
    ///
    /// # Panics
    ///
    /// Panics if `values` has neither method.
    fn for_iterator(&mut self, values: &mut Expr) {
        let span = values.span.clone();
        if self.has_method(values, "next", &span) {
            // `for x in self` in an iterator's method goes on with `self`, not a copy of it
            if matches!(&values.kind, ExprKind::Identifier(name) if name == "self") {
                let receiver = std::mem::replace(values, Expr::new(ExprKind::Boolean(false), span.clone()));
                values.kind = ExprKind::Ref(Box::new(receiver));
            }
            return;
        }
        if !self.has_method(values, "iter", &span) {
            let ty = peel(self.probe_type(values));
            let ty = ty.owner_name().unwrap_or_else(|| ty.to_string());
            diagnostic::throw(
                format!("a for loop can't go through '{}', it has no 'next' or 'iter' method", ty),
                Some(span),
                &self.source,
            );
        }
        let placeholder = Expr::new(ExprKind::Boolean(false), span.clone());
        let receiver = std::mem::replace(values, placeholder);
        values.kind = ExprKind::FunctionCall {
            name: "iter".to_string(),
            type_args: vec![],
            params: vec![],
            receiver: Some(Box::new(receiver)),
            mangled: None,
        };
    }

    /// Whether the value of `receiver` has a method `name`.
    fn has_method(&mut self, receiver: &Expr, name: &str, span: &Span) -> bool {
        let ty = peel(self.probe_type(receiver));
        if matches!(ty, Type::Buffer(_)) {
            if is_buffer_method(name) {
                return true;
            }
            self.see_buffer(&ty);
        }
        ty.owner_name().is_some_and(|owner| {
            let method = self.find_method(&owner, name, span);
            self.functions.contains_key(&method) || self.generic_functions.contains_key(&method)
        })
    }

    /// Mangled name of the method `name` called on a value whose type has the
    /// [owner name](Type::owner_name) `class`.
    ///
    /// The class's own methods come first, then the methods of the traits the
    /// type implements. If there's none, the name of a missing own method is
    /// returned, so the caller reports it as unknown.
    ///
    /// # Panics
    ///
    /// Panics if more than one implemented trait has a method called `name`.
    fn find_method(&self, class: &str, name: &str, span: &Span) -> String {
        let own = format!("{}_{}", class, name);
        if self.functions.contains_key(&own) {
            return own;
        }

        let traits: Vec<&String> = self.impls.get(class).into_iter().flatten()
            .filter(|tr| self.functions.contains_key(&format!("{}_{}_{}", class, tr, name)))
            .collect();
        match traits.as_slice() {
            // An extern type has the methods of the one it's a kind of
            [] => match self.inherited_method(class, name, span) {
                Some(inherited) => inherited,
                None => {
                    // `true.try_as<int32>()` without an `impl TryAs<int32> for bool`
                    if let Some(target) = &self.method_target {
                        let generic_trait = self.trait_substs.keys()
                            .find(|tr| self.traits[tr.as_str()].iter().any(|m| function_name(m) == name))
                            .map(|tr| tr.split('<').next().unwrap_or(tr).rsplit("::").next().unwrap_or(tr).to_string());
                        if let Some(tr) = generic_trait {
                            let target = crate::analysis::show_type(target);
                            diagnostic::throw(format!("'{}' has no '{}' giving '{}', it needs an `impl {}<{}> for {}`",
                                class, name, target, tr, target, class), Some(span.clone()), &self.source);
                        }
                    }
                    own
                }
            },
            [tr] => format!("{}_{}_{}", class, tr, name),
            _ => {
                // Impls of one trait for different types, like `TryAs<int32>` and
                // `TryAs<float64>` for `String`: the one giving the wanted type
                let gives_target = |tr: &&&String| self.method_target.as_ref().is_some_and(|target| {
                    self.functions.get(&format!("{}_{}_{}", class, tr, name))
                        .is_some_and(|(_, ret, ..)| ret == target || self.option_value_type(ret).as_ref() == Some(target))
                });
                match traits.iter().filter(gives_target).collect::<Vec<_>>().as_slice() {
                    [tr] => {
                        self.method_target_used.set(true);
                        format!("{}_{}_{}", class, tr, name)
                    }
                    // Impls of one generic trait, the call just doesn't say which type it wants
                    [] if self.method_target.is_none() && traits.iter().all(|tr| self.trait_substs.contains_key(tr.as_str())) =>
                        diagnostic::throw(format!("'{}' of '{}' can give several types, write which one like `{}<int32>()`", name, class, name),
                            Some(span.clone()), &self.source),
                    _ => SemanticError::AmbiguousMethod {
                        method: name.to_string(),
                        class: class.to_string(),
                        traits: traits.iter().map(|tr| tr.to_string()).collect(),
                        span: span.clone(),
                    }.throw(&self.source),
                }
            }
        }
    }

    /// The method `name` of the extern type `handle` inherits from the one it's
    /// a kind of, or from that one's, see [`Sema::handle_bases`]. `None` if there's none.
    fn inherited_method(&self, handle: &str, name: &str, span: &Span) -> Option<String> {
        let mut current = handle.to_string();
        // A loop like `A: B` and `B: A` is reported where the types are declared
        for _ in 0..64 {
            current = self.handle_bases.get(&current)?.clone();
            let method = self.find_own_method(&current, name, span);
            if self.functions.contains_key(&method) {
                return Some(method);
            }
        }
        None
    }

    /// Like [`Sema::find_method`] without looking into the bases.
    fn find_own_method(&self, class: &str, name: &str, span: &Span) -> String {
        let own = format!("{}_{}", class, name);
        if self.functions.contains_key(&own) {
            return own;
        }
        let traits: Vec<&String> = self.impls.get(class).into_iter().flatten()
            .filter(|tr| self.functions.contains_key(&format!("{}_{}_{}", class, tr, name)))
            .collect();
        match traits.as_slice() {
            [tr] => format!("{}_{}_{}", class, tr, name),
            [_, _, ..] => SemanticError::AmbiguousMethod {
                method: name.to_string(),
                class: class.to_string(),
                traits: traits.iter().map(|tr| tr.to_string()).collect(),
                span: span.clone(),
            }.throw(&self.source),
            [] => own,
        }
    }

    /// Whether the extern type `handle` is `base` or a kind of it, directly
    /// or through others, see [`Sema::handle_bases`].
    fn is_kind_of(&self, handle: &str, base: &str) -> bool {
        let mut current = handle;
        for _ in 0..64 {
            if current == base {
                return true;
            }
            match self.handle_bases.get(current) {
                Some(next) => current = next,
                None => return false,
            }
        }
        false
    }

    /// Checks a brace initializer or enum variant construction and lowers it to
    /// a constructor call. `expected` helps inferring the type arguments of a
    /// generic class or enum.
    ///
    /// # Panics
    ///
    /// Panics if the class, enum, variant or a field is unknown, or a value has the wrong type.
    fn check_struct_init(&mut self, expr: &mut Expr, expected: Option<&Type>) -> Type {
        let span = expr.span.clone();
        let is_enum = matches!(expr.kind, ExprKind::EnumInit { .. });

        // Generic classes and enums are first replaced by their instance, like `Box<int32>`.
        // Inferring the type arguments checks the values, they mustn't be checked again.
        let mut values_checked = false;
        match &mut expr.kind {
            ExprKind::StructInit { class, type_args, fields } if self.generic_types.contains_key(class.as_str()) => {
                *class = self.instantiate_struct_init(&class.clone(), type_args, fields, expected, &span);
                values_checked = true;
            }
            ExprKind::EnumInit { name, type_args, tag, value } if self.generic_types.contains_key(name.as_str()) => {
                // A brace initializer payload only knows its class once the instance is known
                let check_value = !matches!(value.kind, ExprKind::StructInit { .. });
                *name = self.instantiate_enum_init(&name.clone(), type_args, tag, value, check_value, expected, &span);
                values_checked = check_value;
            }
            ExprKind::StructInit { class: name, type_args, .. } | ExprKind::EnumInit { name, type_args, .. }
                if !type_args.is_empty() =>
            {
                if !self.classes.contains_key(name.as_str()) && !self.enums.contains_key(name.as_str()) {
                    SemanticError::UnknownType { name: name.clone(), span: span.clone() }.throw(&self.source);
                }
                SemanticError::NotGeneric { function: name.clone(), span: span.clone() }.throw(&self.source);
            }
            _ => {}
        }

        // For enum initializers, first canonicalize the payload value:
        // if it's a struct-init, rewrite its class name to match the variant's
        // declared payload type, then check the value expression so that any
        // StructInit inside gets lowered to a constructor FunctionCall.
        if let ExprKind::EnumInit { name, tag, value, .. } = &mut expr.kind {
            let variants = self.enums.get(name).cloned()
                .unwrap_or_else(|| SemanticError::UnknownEnum {
                    name: name.clone(),
                    span: span.clone(),
                }.throw(&self.source));
            let payload_ty = variants.iter().find(|(v, _, _)| v == tag)
                .map(|(_, _, t)| t.clone())
                .unwrap_or_else(|| SemanticError::UnknownVariant {
                    enum_name: name.clone(),
                    variant: tag.clone(),
                    span: span.clone(),
                }.throw(&self.source));

            if let ExprKind::StructInit { class, .. } = &mut value.kind {
                if let Type::Class(payload_class) = &payload_ty {
                    *class = payload_class.clone();
                }
            }

            if !values_checked {
                self.check_expr_expecting(value, &payload_ty);
            }
            self.coerce(value, &payload_ty);
            let value_ty = value.ty.clone();
            if value_ty != payload_ty {
                SemanticError::TypeMismatch {
                    lhs: value_ty,
                    rhs: payload_ty,
                    span: value.span.clone(),
                    context: "enum variant payload",
                }.throw(&self.source);
            }
        }

        let (struct_name, params, constructor) = match &expr.kind {
            ExprKind::EnumInit { name, tag, value, .. } => {
                let constructor = format!("{}__{}", name, tag);
                (name.to_string(), vec![*value.clone()], constructor)
            }
            ExprKind::StructInit { class, fields, .. } => {
                // With several constructors, the one taking exactly the fields given
                let names: Vec<String> = fields.iter().map(|(field, _)| field.clone()).collect();
                let constructor = self.constructors.get(class.as_str())
                    .and_then(|constructors| constructors.iter()
                        .find(|ctor| self.functions.get(*ctor).is_some_and(|(params, ..)| fits_names(params, &names))))
                    .cloned()
                    .unwrap_or_else(|| class.clone());
                let (defs, _ret, _visibility, _module) = self.functions.get(&constructor).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownFunction {
                        fn_name: class.to_string(),
                        span: span.clone(),
                    }.throw(&self.source));

                let arg_defs: Vec<Param> = defs.into_iter().filter(|d| d.name != "self").collect();

                let mut slots: Vec<Option<Expr>> = (0..arg_defs.len()).map(|_| None).collect();
                for (fname, fexpr) in fields {
                    let idx = arg_defs.iter().position(|d| d.name == *fname)
                        .unwrap_or_else(|| SemanticError::UnknownField {
                            class: class.to_string(),
                            field: fname.clone(),
                            span: fexpr.span.clone(),
                        }.throw(&self.source));
                    slots[idx] = Some(fexpr.clone());
                }

                let mut ordered: Vec<Expr> = Vec::with_capacity(arg_defs.len());
                for (i, slot) in slots.into_iter().enumerate() {
                    let (mut arg, given) = match slot {
                        Some(e) => (e, true),
                        None => match arg_defs[i].value.as_ref() {
                            Some(default) => ((**default).clone(), false),
                            None => SemanticError::MissingArgument {
                                function: class.to_string(),
                                index: i,
                                span: span.clone(),
                            }.throw(&self.source),
                        },
                    };
                    let def_ty = &arg_defs[i].ty;
                    if !(values_checked && given) {
                        self.check_expr_expecting(&mut arg, def_ty);
                    }
                    self.coerce(&mut arg, def_ty);
                    let arg_ty = arg.ty.clone();
                    let matches_ty = &arg_ty == def_ty
                        || matches!(def_ty, Type::Ref(inner) if **inner == arg_ty);
                    if !matches_ty {
                        SemanticError::TypeMismatch {
                            lhs: arg_ty,
                            rhs: def_ty.clone(),
                            span: arg.span.clone(),
                            context: "struct initializer field",
                        }.throw(&self.source);
                    }
                    ordered.push(arg);
                }
                (class.to_string(), ordered, constructor)
            }
            _ => unreachable!(),
        };

        let ty = if is_enum {
            Type::Enum(struct_name.clone())
        } else {
            Type::Class(struct_name.clone())
        };
        self.use_method(&constructor, &span);
        expr.kind = ExprKind::FunctionCall {
            name: struct_name.clone(),
            type_args: vec![],
            params: params,
            receiver: None,
            mangled: Some(constructor),
        };
        expr.ty = ty.clone();
        ty
    }

    fn check_decl(&mut self, decl: &mut Declaration) -> Type {
        let ty = match &mut decl.kind {
            // Generic templates are checked per instance
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => Type::Void,
            DeclarationKind::Class { name, fields, methods, .. } => {
                // Field defaults are evaluated in the constructor, so they can use its parameters
                let prev_scope = self.scope.clone();
                let prev_moved = self.moved.clone();
                for method in methods.iter() {
                    if let DeclarationKind::Function { name: fn_name, params, .. } = &method.kind {
                        if fn_name == name {
                            for p in params {
                                self.scope.insert(p.name.clone(), (p.ty.clone(), false));
                            }
                        }
                    }
                }
                for field in fields.iter_mut() {
                    if let Some(default) = &mut field.value {
                        self.check_default(default, &field.ty);
                    }
                }
                self.scope = prev_scope.clone();
                self.moved = prev_moved.clone();

                // Every constructor runs the defaults, so with several each has to have the
                // parameters they use. Copies are checked, the defaults are checked already
                let constructors: Vec<Vec<Param>> = methods.iter().filter_map(|method| match &method.kind {
                    DeclarationKind::Function { name: fn_name, params, .. } if fn_name == name => Some(params.clone()),
                    _ => None,
                }).collect();
                if constructors.len() > 1 {
                    for params in &constructors {
                        for p in params {
                            self.scope.insert(p.name.clone(), (p.ty.clone(), false));
                        }
                        for field in fields.iter() {
                            let Some(default) = &field.value else { continue };
                            let mut copy = default.clone();
                            let ty = field.ty.clone();
                            if let Err(error) = diagnostic::catch(|| self.check_default(&mut copy, &ty)) {
                                let taken: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
                                diagnostic::throw(
                                    format!("the default of field '{}' runs in every constructor, but it fails in `Constructor({})`: {}",
                                        field.name, taken.join(", "), error.message),
                                    Some(default.span.clone()),
                                    &self.source,
                                );
                            }
                        }
                        self.scope = prev_scope.clone();
                        self.moved = prev_moved.clone();
                    }
                }

                // An instance of a generic class checks its methods once they're called
                if let Some(lazy) = self.lazy_instance.take() {
                    self.register_lazy_methods(methods, lazy);
                    return Type::Class(name.clone());
                }
                let prev_class = self.current_class.take();
                self.current_class = Some(name.clone());
                for method in methods.iter_mut() {
                    self.check_decl(method);
                }
                self.current_class = prev_class;
                Type::Class(name.clone())
            }
            DeclarationKind::Enum { name, methods, .. } => {
                if let Some(lazy) = self.lazy_instance.take() {
                    self.register_lazy_methods(methods, lazy);
                    return Type::Enum(name.clone());
                }
                let prev_class = self.current_class.replace(name.clone());
                for method in methods.iter_mut() {
                    self.check_decl(method);
                }
                self.current_class = prev_class;
                Type::Enum(name.clone())
            }
            // Generic functions are templates, their instances get checked instead
            DeclarationKind::Function { type_params, .. } if !type_params.is_empty() => Type::Void,
            DeclarationKind::Function { name, params, body, return_type, .. } => {
                for p in params.iter_mut() {
                    if let Some(default) = &mut p.value {
                        self.check_default(default, &p.ty);
                    }
                }

                let ret = return_type.clone().unwrap_or(Type::Void);
                self.current_return_type = Some(ret.clone());

                let prev_scope = self.scope.clone();
                // The function's parameters start out owning their values, `self` only borrows its
                let prev_moved = std::mem::take(&mut self.moved);
                let prev_borrowed = std::mem::replace(&mut self.borrowed, HashSet::from(["self".to_string()]));
                let prev_roots = std::mem::take(&mut self.binding_roots);
                if let Some(cls) = self.current_class.clone() {
                    // A constructor builds its object, so it may change it. Methods
                    // declare `self` as a parameter, which replaces this one. Static
                    // functions have no `self`
                    let is_ctor = *name == cls;
                    if is_ctor || params.first().is_some_and(|p| p.name == "self") {
                        self.scope.insert("self".to_string(), (Type::Class(cls), is_ctor));
                    }
                }
                for p in params.iter() {
                    self.scope.insert(p.name.clone(), (p.ty.clone(), p.mutable));
                }
                // Loops around the declaration don't continue into the function
                let prev_loops = std::mem::take(&mut self.loop_depth);
                // An `if` or `match` at the end gives the value to return, unless its arms return themselves
                if let (Some(stmts), false) = (body.as_mut(), ret == Type::Void) {
                    if let Some(last) = stmts.last_mut() {
                        let gives_value = match &last.kind {
                            StmtKind::Expression(expr @ Expr { kind: ExprKind::If { else_block: Some(_), .. } | ExprKind::Match { .. }, .. }) => !leaves(expr),
                            _ => false,
                        };
                        if gives_value {
                            let StmtKind::Expression(expr) = std::mem::replace(&mut last.kind, StmtKind::Break) else { unreachable!() };
                            last.kind = StmtKind::Return(expr);
                        }
                    }
                }
                if let Some(stmts) = body {
                    self.check_stmts(stmts);
                }
                // A function giving a value has to give one on every way through it. A
                // constructor gives its object by itself
                let is_ctor = self.current_class.as_ref() == Some(&*name);
                if let (Some(stmts), false, false) = (body.as_ref(), ret == Type::Void, is_ctor) {
                    if !returns_always(stmts) {
                        self.loop_depth = prev_loops;
                        self.scope = prev_scope;
                        self.moved = prev_moved;
                        self.borrowed = prev_borrowed;
                        self.binding_roots = prev_roots;
                        self.current_return_type = None;
                        diagnostic::throw(
                            format!("'{}' can reach its end without giving a '{}', it needs a `return` there",
                                name.rsplit("::").next().unwrap_or(name), written_type(&crate::analysis::show_type(&ret))),
                            Some(decl.span.clone()),
                            &self.source,
                        );
                    }
                }
                self.loop_depth = prev_loops;
                self.scope = prev_scope;
                self.moved = prev_moved;
                self.borrowed = prev_borrowed;
                self.binding_roots = prev_roots;
                self.current_return_type = None;

                ret
            }
            // Trait methods are checked through the impls they get copied into
            DeclarationKind::Trait { .. } | DeclarationKind::Extern(ExternItem::Link(_)) => Type::Void,
            DeclarationKind::Impl { type_params, .. } if !type_params.is_empty() => Type::Void,
            DeclarationKind::Impl { trait_name, target, methods, .. } => {
                let prev_class = self.current_class.take();
                self.current_class = target.owner_name();
                // Default methods copied from a generic trait's instance use its type
                // parameters. The impl's own ones, of a generic impl, are set already
                let outer_subst = self.type_subst.clone();
                for (param, ty) in self.trait_substs.get(trait_name.as_str()).cloned().unwrap_or_default() {
                    match self.type_subst.get(&param) {
                        Some(own) if *own != ty => diagnostic::throw(
                            format!("type parameter '{}' stands for '{}' in the impl, but for '{}' in its trait '{}', rename one of them", param, own, ty, trait_name),
                            Some(decl.span.clone()),
                            &self.source,
                        ),
                        _ => { self.type_subst.insert(param, ty); }
                    }
                }
                for method in methods.iter_mut() {
                    self.check_decl(method);
                }
                self.type_subst = outer_subst;
                self.current_class = prev_class;
                Type::Void
            }
            DeclarationKind::Extern(ExternItem::Type { name, methods, .. }) => {
                let prev_class = self.current_class.replace(name.clone());
                for method in methods.iter_mut() {
                    self.check_decl(method);
                }
                self.current_class = prev_class;
                Type::Void
            }
        };

        ty
    }

    /// Checks the default value of a parameter or field against its type `ty`.
    ///
    /// # Panics
    ///
    /// Panics if the value doesn't have the type `ty`.
    fn check_default(&mut self, default: &mut Expr, ty: &Type) {
        self.check_expr_expecting(default, ty);
        self.coerce(default, ty);
        if default.ty != *ty {
            SemanticError::TypeMismatch {
                lhs: default.ty.clone(),
                rhs: ty.clone(),
                span: default.span.clone(),
                context: "default value",
            }.throw(&self.source);
        }
    }
}

/// Whether the statements `body` always leave before their end, with
/// `return`, `break`, `continue`, `panic(...)` or `exit(...)`, also in both
/// branches of an `if`.
fn diverges(body: &[Stmt]) -> bool {
    match body.last().map(|stmt| &stmt.kind) {
        Some(StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue) => true,
        Some(StmtKind::Expression(expr)) => leaves(expr),
        _ => false,
    }
}

/// Whether `expr` never gives a value because it always leaves: a call of
/// `panic` or `exit`, an `if` with `else` or a `match` whose blocks all leave.
fn leaves(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::If { then_block, else_block: Some(else_block), .. } => block_leaves(then_block) && block_leaves(else_block),
        ExprKind::Match { arms, .. } => !arms.is_empty() && arms.iter().all(|arm| block_leaves(&arm.block)),
        _ => is_leaving_call(expr),
    }
}

/// The type `shown` like it's written in code, without the modules its names
/// are from: `Result<Token, String>` for `result::Result<token::Token,String>`.
fn written_type(shown: &str) -> String {
    let mut written = String::new();
    let mut word = String::new();
    let mut chars = shown.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        // A module's name before `::` is left out
        if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            word.clear();
            continue;
        }
        written.push_str(&std::mem::take(&mut word));
        written.push(c);
        if c == ',' && chars.peek() != Some(&' ') {
            written.push(' ');
        }
    }
    written.push_str(&word);
    written
}

/// Whether the statements of a function body always leave it with a value
/// before their end: with `return` or a call like `panic(...)`, in every
/// branch of an `if` or `match` at the end, or in a `while true` loop without
/// `break` that can only be left that way.
fn returns_always(body: &[Stmt]) -> bool {
    // Any statement that always leaves ends it, the code after it never runs
    body.iter().any(|stmt| match &stmt.kind {
        StmtKind::While { condition: Expr { kind: ExprKind::Boolean(true), .. }, body } => !breaks(body),
        _ => block_returns(&Block { stmts: vec![stmt.clone()], value: None }),
    })
}

/// Whether `body`, a loop's body, has a `break` that leaves this loop, not
/// one of a loop inside it.
fn breaks(body: &[Stmt]) -> bool {
    fn in_block(block: &Block) -> bool {
        breaks(&block.stmts) || block.value.as_deref().is_some_and(in_expr)
    }
    fn in_expr(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::If { then_block, else_block, .. } => in_block(then_block) || else_block.as_ref().is_some_and(in_block),
            ExprKind::Match { arms, .. } => arms.iter().any(|arm| in_block(&arm.block)),
            // `x ?? break`
            ExprKind::Leave(stmt) => matches!(stmt.kind, StmtKind::Break),
            ExprKind::BinaryExpr { lhs, rhs, .. } => in_expr(lhs) || in_expr(rhs),
            _ => false,
        }
    }
    body.iter().any(|stmt| match &stmt.kind {
        StmtKind::Break => true,
        StmtKind::Expression(expr) | StmtKind::Return(expr) => in_expr(expr),
        StmtKind::Instantiation { value, .. } | StmtKind::Destructure { value, .. } => in_expr(value),
        // A `break` in an inner loop leaves that one
        StmtKind::While { .. } => false,
        _ => false,
    })
}

/// Whether `block` always leaves the function, with `return`, `panic(...)` or
/// `process::exit(...)`, also in every branch of an `if` or `match` at its
/// end. Unlike [`block_leaves`], `break` and `continue` don't count, the code
/// after the loop or the next round still runs.
fn block_returns(block: &Block) -> bool {
    fn expr_returns(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::If { then_block, else_block: Some(else_block), .. } => block_returns(then_block) && block_returns(else_block),
            ExprKind::Match { arms, .. } => !arms.is_empty() && arms.iter().all(|arm| block_returns(&arm.block)),
            _ => is_leaving_call(expr),
        }
    }
    if let Some(value) = &block.value {
        return expr_returns(value);
    }
    match block.stmts.last().map(|stmt| &stmt.kind) {
        Some(StmtKind::Return(_)) => true,
        Some(StmtKind::Expression(expr)) => expr_returns(expr),
        _ => false,
    }
}

/// Whether `block` always leaves, see [`diverges`].
fn block_leaves(block: &Block) -> bool {
    match &block.value {
        Some(value) => leaves(value),
        None => diverges(&block.stmts),
    }
}

/// The operator `op` on the checked strings `lhs` and `rhs` as calls of the
/// methods of the prelude's `String`: `a + b` is `a.concat(b)`, or
/// `a.concat_char(b)` for a char `b`, `a == b` is `a.equals(b)`, and an
/// ordering like `a < b` is `a.compare(b) < 0`.
fn string_operator(lhs: Expr, op: &Token, rhs: Expr, span: &Span) -> ExprKind {
    let adds_char = rhs.ty == Type::Char;
    let method = |name: &str, ty: Type| Expr {
        kind: ExprKind::FunctionCall {
            name: name.to_string(),
            type_args: vec![],
            params: vec![rhs],
            receiver: Some(Box::new(lhs)),
            mangled: Some(format!("String_{}", name)),
        },
        span: span.clone(),
        ty,
    };
    match op {
        Token::Plus if adds_char => method("concat_char", Type::string()).kind,
        Token::Plus => method("concat", Type::string()).kind,
        Token::EqEq => method("equals", Type::Bool).kind,
        Token::NotEq => ExprKind::Not(Box::new(method("equals", Type::Bool))),
        _ => {
            let zero = Expr { kind: ExprKind::IntLiteral(0), span: span.clone(), ty: Type::INT32 };
            ExprKind::BinaryExpr { lhs: Box::new(method("compare", Type::INT32)), op: op.clone(), rhs: Box::new(zero) }
        }
    }
}

/// Whether `expr` calls `panic` or `process::exit`, which never come back.
fn is_leaving_call(expr: &Expr) -> bool {
    matches!(&expr.kind, ExprKind::FunctionCall { name, receiver: None, .. } if name == "panic" || name == "process::exit")
}

/// Whether `ty` contains `Self`, the type implementing a trait.
fn has_self_type(ty: &Type) -> bool {
    match ty {
        Type::Param(name) => name == "Self",
        Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) | Type::Buffer(inner)
            | Type::Array(inner, _) | Type::Slice(inner) => has_self_type(inner),
        Type::Generic { args, .. } | Type::Tuple(args) => args.iter().any(has_self_type),
        _ => false,
    }
}

/// `ty` with `Self` replaced by `target`.
fn replace_self(ty: &Type, target: &Type) -> Type {
    let inner = |t: &Type| Box::new(replace_self(t, target));
    match ty {
        Type::Param(name) if name == "Self" => target.clone(),
        Type::Ref(t) => Type::Ref(inner(t)),
        Type::Unique(t) => Type::Unique(inner(t)),
        Type::Shared(t) => Type::Shared(inner(t)),
        Type::Buffer(t) => Type::Buffer(inner(t)),
        Type::Slice(t) => Type::Slice(inner(t)),
        Type::Array(t, len) => Type::Array(inner(t), *len),
        Type::Tuple(items) => Type::Tuple(items.iter().map(|t| replace_self(t, target)).collect()),
        Type::Generic { name, args } => Type::Generic { name: name.clone(), args: args.iter().map(|t| replace_self(t, target)).collect() },
        other => other.clone(),
    }
}

/// Whether `a` and `b` are different Buffers or slices of one-byte values,
/// `char` and `uint8`, which are the same memory.
fn same_bytes(a: &Type, b: &Type) -> bool {
    let is_byte = |ty: &Type| matches!(ty, Type::Char | Type::Int { bits: 8, signed: false });
    match (a, b) {
        (Type::Buffer(x), Type::Buffer(y)) | (Type::Slice(x), Type::Slice(y)) => x != y && is_byte(x) && is_byte(y),
        _ => false,
    }
}

/// Whether the generic class `template` is the `Vector` of `std::vector`,
/// or a program's own `Vector`, which list patterns and slices work with.
fn is_vector(template: &str) -> bool {
    template == "vector::Vector" || template == "Vector"
}

/// The `T` of a `Shared<T>` or `Unique<T>`, also through a reference.
fn heap_inner(ty: &Type) -> Option<Type> {
    match ty {
        Type::Shared(inner) | Type::Unique(inner) => Some((**inner).clone()),
        Type::Ref(inner) => match inner.as_ref() {
            Type::Shared(inner) | Type::Unique(inner) => Some((**inner).clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Makes `expr`, a reference to a number, bool or char, the value it refers
/// to. Other expressions stay as they are.
fn read_through(expr: &mut Expr) {
    let Type::Ref(inner) = &expr.ty else { return };
    if !(inner.is_number() || matches!(inner.as_ref(), Type::Bool | Type::Char)) {
        return;
    }
    let value_ty = (**inner).clone();
    let span = expr.span.clone();
    let reference = std::mem::replace(expr, Expr::new(ExprKind::Boolean(false), span.clone()));
    *expr = Expr { kind: ExprKind::Deref(Box::new(reference)), span, ty: value_ty };
}

/// `ty` without the references to it, like `Point` for `&&Point`.
pub fn without_refs(ty: &Type) -> &Type {
    match ty {
        Type::Ref(inner) => without_refs(inner),
        other => other,
    }
}

/// Whether `==` on a `lhs` and a `rhs` compares what they hold, see
/// `CodegenContext::values_equal`: the same type, each maybe through
/// references, other than a number, bool or char compared directly.
pub fn compares_contents(lhs: &Type, rhs: &Type) -> bool {
    let ty = without_refs(lhs);
    ty == without_refs(rhs) && !(ty.is_number() || matches!(ty, Type::Bool | Type::Char | Type::Unknown | Type::Void))
}

/// The type behind a reference, `Unique` or `Shared`, whose fields and
/// methods are used through it. Other types as they are.
fn peel(ty: Type) -> Type {
    match ty {
        Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) => *inner,
        other => other,
    }
}

/// Whether `name` is a built-in method of `Buffer`, see `check_buffer_method`.
fn is_buffer_method(name: &str) -> bool {
    matches!(name, "get" | "take" | "set" | "capacity" | "resize")
}

/// Whether `ty` contains a type parameter like `T`, also nested like in `Box<T>`.
fn contains_type_param(ty: &Type) -> bool {
    match ty {
        Type::Param(_) => true,
        Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) | Type::Buffer(inner) | Type::Array(inner, _) => contains_type_param(inner),
        Type::Generic { args, .. } => args.iter().any(contains_type_param),
        _ => false,
    }
}

/// Name of the instance of the generic `name` with the type arguments in
/// `subst`, like `max<int32>` or `Box<Point,String>`.
fn instance_name(name: &str, type_params: &[TypeParam], subst: &HashMap<String, Type>) -> String {
    let args: Vec<String> = type_params.iter()
        .map(|p| {
            let ty = &subst[&p.name];
            ty.class_name().filter(|_| !matches!(ty, Type::Ref(_))).unwrap_or_else(|| ty.to_string())
        })
        .collect();
    format!("{}<{}>", name, args.join(","))
}

/// Parameters of the constructor of the class `class` among its `methods`,
/// empty if it has none.
/// The name of the first variant of `decl` if it's an enum, for showing how one is made.
fn first_variant(decl: &Declaration) -> Option<&str> {
    match &decl.kind {
        DeclarationKind::Enum { fields, .. } => fields.first().map(|(variant, ..)| variant.as_str()),
        _ => None,
    }
}

fn constructor_params(class: &str, methods: &[Declaration], count: usize) -> Vec<Param> {
    let constructors: Vec<Vec<Param>> = methods.iter()
        .filter_map(|m| match &m.kind {
            DeclarationKind::Function { name, params, .. } if name == class => Some(params.clone()),
            _ => None,
        })
        .collect();
    // With several, the one a call with `count` arguments goes to
    let fits = |params: &&Vec<Param>| {
        let (min, max) = argument_range(params);
        min <= count && count <= max
    };
    constructors.iter().find(fits).or(constructors.first()).cloned().unwrap_or_default()
}

/// The parameters of the constructor of `class` a brace initializer setting
/// `names`, like `Point { x: 1, y: 2 }`, goes to: one taking each of them
/// that needs no others. The first constructor if none does.
fn constructor_params_named(class: &str, methods: &[Declaration], names: &[String]) -> Vec<Param> {
    let constructors: Vec<Vec<Param>> = methods.iter()
        .filter_map(|m| match &m.kind {
            DeclarationKind::Function { name, params, .. } if name == class => Some(params.clone()),
            _ => None,
        })
        .collect();
    constructors.iter().find(|params| fits_names(params, names)).or(constructors.first()).cloned().unwrap_or_default()
}

/// Whether a function with `params` can be called with exactly the
/// arguments named `names`: it takes each of them, and the ones it doesn't
/// get have defaults.
fn fits_names(params: &[Param], names: &[String]) -> bool {
    let params: Vec<&Param> = params.iter().filter(|p| p.name != "self").collect();
    names.iter().all(|name| params.iter().any(|p| p.name == *name))
        && params.iter().all(|p| p.value.is_some() || names.contains(&p.name))
}

/// How many arguments a function with `params` takes: at least the ones
/// without default, at most all of them. `self` doesn't count.
fn argument_range(params: &[Param]) -> (usize, usize) {
    let params: Vec<&Param> = params.iter().filter(|p| p.name != "self").collect();
    (params.iter().filter(|p| p.value.is_none()).count(), params.len())
}

/// The variable a place expression like `a.b.c` starts at, `a` here. `None`
/// if it starts at a temporary value like the result of a call.
/// The mangled name of the built-in `x.clone()`, which codegen recognizes.
pub const CLONE: &str = "builtin.clone";

/// The mangled name of the built-in `s.len()` of a slice `&[T]`.
pub const SLICE_LEN: &str = "builtin.slice_len";

/// The mangled name of the built-in `a.get()` of a `Shared` or `Unique`.
pub const GET: &str = "builtin.get";

/// The trait `Print` of `std::string`, whose `to_string` `print` uses for the
/// types it can't print by itself, like classes.
const PRINT_TRAIT: &str = "string::Print";

/// The trait `Default` of `std::default`, which a class with a constructor
/// taking no arguments has without an impl, see [`Sema::implements`].
const DEFAULT_TRAIT: &str = "default::Default";

/// The variable a moved path like `p.left` belongs to, `p`.
fn path_root(path: &str) -> &str {
    path.split(['.', '[']).next().unwrap_or(path)
}

/// How the place `expr` is written, like `token.kind` or `words[..]`, for errors.
fn place_name(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Identifier(name) => name.clone(),
        ExprKind::FieldAccess { object, field } => format!("{}.{}", place_name(object), field),
        ExprKind::Index { object, .. } => format!("{}[..]", place_name(object)),
        _ => "value".to_string(),
    }
}

fn root_variable(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Identifier(name) => Some(name),
        ExprKind::FieldAccess { object, .. } | ExprKind::Index { object, .. } => root_variable(object),
        _ => None,
    }
}

/// Name of the function `decl` declares, empty for other declarations.
fn function_name(decl: &Declaration) -> &str {
    match &decl.kind {
        DeclarationKind::Function { name, .. } => name,
        _ => "",
    }
}

/// Whether the functions `a` and `b` have the same signature: both or neither
/// take `self`, both or neither change it (`mut &self`), the other parameters
/// have the same types and the return types match.
fn same_signature(a: &DeclarationKind, b: &DeclarationKind) -> bool {
    let (
        DeclarationKind::Function { params: params_a, return_type: return_a, .. },
        DeclarationKind::Function { params: params_b, return_type: return_b, .. },
    ) = (a, b) else {
        return false;
    };

    // `None` without `self`, otherwise whether `self` is mutable
    let self_kind = |params: &[Param]| params.first().filter(|p| p.name == "self").map(|p| p.mutable);
    let arg_types = |params: &[Param]| params.iter()
        .filter(|p| p.name != "self")
        .map(|p| p.ty.clone())
        .collect::<Vec<_>>();

    self_kind(params_a) == self_kind(params_b)
        && arg_types(params_a) == arg_types(params_b)
        && return_a == return_b
}

/// Whether `expr` is a number literal or an expression made only of number literals.
fn is_number_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::IntLiteral(_) | ExprKind::FloatLiteral(_) | ExprKind::CharLiteral(_) => true,
        ExprKind::BinaryExpr { lhs, rhs, .. } => is_number_literal(lhs) && is_number_literal(rhs),
        _ => false,
    }
}

/// Whether `value` fits into an integer type with `bits` bits.
fn int_fits(value: i64, bits: u32, signed: bool) -> bool {
    let value = value as i128;
    if signed {
        let max = (1i128 << (bits - 1)) - 1;
        (-max - 1..=max).contains(&value)
    } else {
        (0..=(1i128 << bits) - 1).contains(&value)
    }
}
