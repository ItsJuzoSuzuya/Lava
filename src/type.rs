use std::fmt::Display;

use inkwell::{AddressSpace, types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, IntType}};
use serde::Serialize;

use crate::{codegen_ctx::CodegenContext, expression::ExprKind};

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Bool,
    /// `char`: one byte of text, like `'a'`. Converts to and from numbers with `as`.
    Char,
    Unknown,
    Void,

    /// `float32` or `float64`.
    Float { bits: u32 },
    /// `int8` … `int64` when `signed`, `uint8` … `uint64` otherwise.
    Int { bits: u32, signed: bool },

    Object(String),
    Enum(String),
    Class(String),
    /// A type parameter like `T` of a generic function. Only appears in the
    /// generic declaration itself, sema replaces it in every instance.
    Param(String),
    /// A generic class or enum with type arguments as written, like `Box<int32>`.
    /// Only comes from the parser, sema replaces it by the instance's
    /// [`Type::Class`] or [`Type::Enum`].
    Generic { name: String, args: Vec<Type> },
    /// `Unique<T>`: a `T` on the heap, owned by one variable at a time and freed
    /// when its owner goes out of scope. Created with `unique(value)`.
    Unique(Box<Type>),
    /// `Shared<T>`: a `T` on the heap used by any number of variables, freed
    /// when the last one goes away. Copying one only counts one more user.
    /// Created with `shared(value)`.
    Shared(Box<Type>),
    /// `Buffer<T>`: a block of `T`s on the heap with a fixed but resizable
    /// capacity, the building block for growable containers. Owned like a
    /// `Unique`. Created with `Buffer<T>(capacity)`.
    Buffer(Box<Type>),
    /// `T[N]`: `N` values of `T` next to each other, held directly like a
    /// class's fields. Copying an array copies all of them.
    Array(Box<Type>, u64),
    /// `(T, U)`: values of different types together, used like `t.0` and
    /// `t.1`. Owns what its values own, like a class.
    Tuple(Vec<Type>),
    /// `&[T]`: a borrowed view of `T`s next to each other, of any length,
    /// like an array or the elements of a `Vector`. Its address and length.
    Slice(Box<Type>),
    Ref(Box<Type>),
    /// A C pointer declared with `extern type Name;`, only passed to and
    /// returned from `extern func`s. Copied like a number, C frees it.
    Handle(String),
    /// `impl Iterator<T>` as the return type of a trait method: each impl
    /// returns a type of its own that implements the trait, like `Iterator<T>`
    /// as [`Type::Class`] or [`Type::Generic`]. Never the type of a value.
    Opaque(Box<Type>),
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Bool          => write!(f, "bool"),
            Type::Char          => write!(f, "char"),
            Type::Unknown       => write!(f, "Unknown"),
            Type::Void          => write!(f, "Void"),

            Type::Float { bits }        => write!(f, "float{}", bits),
            Type::Int { bits, signed }  => write!(f, "{}int{}", if *signed { "" } else { "u" }, bits),

            Type::Object(name)  => write!(f, "Object({})", name),
            Type::Enum(name)    => write!(f, "Enum({})", name),
            _ if self.is_string() => write!(f, "String"),
            Type::Class(name)   => write!(f, "Class({})", name),
            Type::Param(name)   => write!(f, "{}", name),
            Type::Generic { name, args } => {
                let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                write!(f, "{}<{}>", name, args.join(", "))
            }
            Type::Unique(inner) => write!(f, "Unique<{}>", inner),
            Type::Shared(inner) => write!(f, "Shared<{}>", inner),
            Type::Buffer(inner) => write!(f, "Buffer<{}>", inner),
            Type::Array(element, len) => write!(f, "{}[{}]", element, len),
            Type::Slice(element) => write!(f, "&[{}]", element),
            Type::Tuple(items) => {
                let items: Vec<String> = items.iter().map(|t| t.to_string()).collect();
                write!(f, "({})", items.join(", "))
            }
            Type::Ref(inner)    => write!(f, "Ref({})", inner),
            Type::Handle(name)  => write!(f, "{}", name),
            Type::Opaque(bound) => match bound.as_ref() {
                Type::Class(name) => write!(f, "impl {}", name),
                other => write!(f, "impl {}", other),
            },
        }
    }
}

impl<'ctx> Type {
    /// The default type of integer literals without context.
    pub const INT32: Type = Type::Int { bits: 32, signed: true };
    /// The default type of float literals without context.
    pub const FLOAT64: Type = Type::Float { bits: 64 };
    /// The type of sizes and indices, like a `Buffer`'s capacity.
    pub const UINT64: Type = Type::Int { bits: 64, signed: false };

    /// `String`, the class of the prelude holding text. String literals are its
    /// values, and its methods implement the operators on them.
    pub fn string() -> Type {
        Type::Class("String".to_string())
    }

    /// Whether this is `String`, see [`Type::string`].
    pub fn is_string(&self) -> bool {
        matches!(self, Type::Class(name) if name == "String")
    }

    /// Name of the class or enum this type refers to, also through a reference
    /// or a `Unique`. `None` for every other type.
    pub fn class_name(&self) -> Option<String> {
        match self {
            Type::Class(name) | Type::Enum(name) => Some(name.clone()),
            Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) => inner.class_name(),
            _ => None,
        }
    }

    /// Whether values of this type own heap memory and get moved instead of
    /// copied: `Unique` and `Buffer`.
    pub fn is_move_only(&self) -> bool {
        matches!(self, Type::Unique(_) | Type::Buffer(_))
    }

    /// Name the methods of this type are declared and mangled under: the class
    /// or enum name, or the type's spelling for built-in types like `int32`.
    /// `None` for types that can't have methods.
    pub fn owner_name(&self) -> Option<String> {
        match self {
            Type::Class(name) | Type::Enum(name) | Type::Handle(name) => Some(name.clone()),
            Type::Int { .. } | Type::Float { .. } | Type::Bool | Type::Char => Some(self.to_string()),
            _ => None,
        }
    }

    /// Whether this is an integer or float type.
    pub fn is_number(&self) -> bool {
        matches!(self, Type::Int { .. } | Type::Float { .. })
    }

    /// Whether every value of this type fits exactly into `target`, so it can be
    /// converted implicitly.
    ///
    /// Ints widen to ints of the same signedness with more bits, unsigned ints
    /// also to signed ints with more bits. Floats widen to bigger floats. Ints
    /// convert to a float when its mantissa holds all their values: up to 16
    /// bits for `float32`, up to 32 bits for `float64`.
    pub fn widens_to(&self, target: &Type) -> bool {
        match (self, target) {
            (Type::Int { bits: from, signed: from_signed }, Type::Int { bits: to, signed: to_signed }) =>
                from < to && (from_signed == to_signed || !from_signed),
            (Type::Float { bits: from }, Type::Float { bits: to }) => from < to,
            (Type::Int { bits, .. }, Type::Float { bits: 32 }) => *bits <= 16,
            (Type::Int { bits, .. }, Type::Float { bits: 64 }) => *bits <= 32,
            _ => false,
        }
    }

    pub fn to_llvm(&self, ctx: &CodegenContext<'ctx>) -> BasicTypeEnum<'ctx> {
        match self {
            Type::Unknown => panic!("Unknown type reached codegen — sema did not resolve it"),
            Type::Void => panic!("Void has no LLVM basic type"),

            Type::Float { bits: 32 } => ctx.context.f32_type().into(),
            Type::Float { .. } => ctx.context.f64_type().into(),
            Type::Int { bits, .. } => llvm_int_type(*bits, ctx).into(),

            Type::Bool => ctx.context.bool_type().into(),
            Type::Char => ctx.context.i8_type().into(),
            Type::Object(_) => panic!("Object type reached codegen — sema did not canonicalize it"),
            Type::Enum(name) => ctx.module.get_struct_type(name).expect("enum not defined").into(),
            Type::Class(name) => ctx.module.get_struct_type(name).expect("struct not defined").into(),
            Type::Param(name) => panic!("type parameter '{}' reached codegen — sema did not substitute it", name),
            Type::Generic { name, .. } => panic!("generic type '{}' reached codegen — sema did not instantiate it", name),
            Type::Opaque(_) => panic!("'{}' reached codegen — only trait methods return it", self),
            Type::Ref(_) | Type::Unique(_) | Type::Shared(_) | Type::Buffer(_) | Type::Handle(_) => ctx.context.ptr_type(AddressSpace::default()).into(),
            Type::Array(element, len) => element.to_llvm(ctx).array_type(*len as u32).into(),
            Type::Slice(_) => ctx.slice_type().into(),
            Type::Tuple(items) => {
                let fields: Vec<BasicTypeEnum> = items.iter().map(|t| t.to_llvm(ctx)).collect();
                ctx.context.struct_type(&fields, false).into()
            }
        }
    }

    pub fn to_fn_type(&self, param_types: &[BasicMetadataTypeEnum<'ctx>], ctx: &CodegenContext<'ctx>) -> FunctionType<'ctx> {
        match self {
            Type::Unknown => panic!("Unknown type reached codegen — sema did not resolve it"),
            Type::Void => ctx.context.void_type().fn_type(param_types, false),

            Type::Float { bits: 32 } => ctx.context.f32_type().fn_type(param_types, false),
            Type::Float { .. } => ctx.context.f64_type().fn_type(param_types, false),
            Type::Int { bits, .. } => llvm_int_type(*bits, ctx).fn_type(param_types, false),

            Type::Bool => ctx.context.bool_type().fn_type(param_types, false),
            Type::Char => ctx.context.i8_type().fn_type(param_types, false),
            Type::Object(_) => panic!("Object type reached codegen — sema did not canonicalize it"),
            Type::Enum(name) => ctx.module.get_struct_type(name).expect("enum not defined").fn_type(param_types, false),
            Type::Class(name) => ctx.module.get_struct_type(name).expect("struct not defined").fn_type(param_types, false),
            Type::Param(name) => panic!("type parameter '{}' reached codegen — sema did not substitute it", name),
            Type::Generic { name, .. } => panic!("generic type '{}' reached codegen — sema did not instantiate it", name),
            Type::Opaque(_) => panic!("'{}' reached codegen — only trait methods return it", self),
            Type::Ref(_) | Type::Unique(_) | Type::Shared(_) | Type::Buffer(_) | Type::Handle(_) => ctx.context.ptr_type(AddressSpace::default()).fn_type(param_types, false),
            Type::Array(..) | Type::Slice(_) | Type::Tuple(_) => self.to_llvm(ctx).fn_type(param_types, false),
        }
    }

    pub fn formatter(&self) -> Option<&str> {
        let f = match self {
            Type::Bool => "%s",
            Type::Char => "%c",
            Type::Float { .. } => "%f",
            Type::Int { bits: 64, signed: true } => "%lld",
            Type::Int { bits: 64, signed: false } => "%llu",
            Type::Int { signed: true, .. } => "%d",
            Type::Int { signed: false, .. } => "%u",

            Type::Ref(_) => "%p",
            _ => return None
        };

        Some(f)
    }
}

/// Returns the LLVM integer type with `bits` bits.
///
/// # Panics
///
/// Panics if `bits` isn't 8, 16, 32 or 64.
fn llvm_int_type<'ctx>(bits: u32, ctx: &CodegenContext<'ctx>) -> IntType<'ctx> {
    match bits {
        8  => ctx.context.i8_type(),
        16 => ctx.context.i16_type(),
        32 => ctx.context.i32_type(),
        64 => ctx.context.i64_type(),
        other => panic!("no {}-bit integer type", other),
    }
}

impl From<ExprKind> for Type {
    fn from(value: ExprKind) -> Self {
        match value {
            ExprKind::Identifier(name) => Type::Object(name.to_string()),
            ExprKind::IntLiteral(_) => Type::INT32,
            ExprKind::FloatLiteral(_) => Type::FLOAT64,
            ExprKind::StringLiteral(_) => Type::string(),
            ExprKind::Boolean(_) => Type::Bool,
            _ => Type::Unknown
        }
    }
}

