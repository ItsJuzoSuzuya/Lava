use std::fmt::Display;

use inkwell::{AddressSpace, types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType, IntType}};
use serde::Serialize;

use crate::{codegen_ctx::CodegenContext, expression::ExprKind};

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Bool,
    String,
    Unknown,
    Void,

    /// `float32` or `float64`.
    Float { bits: u32 },
    /// `int8` … `int64` when `signed`, `uint8` … `uint64` otherwise.
    Int { bits: u32, signed: bool },

    Object(String),
    Enum(String),
    Class(String),
    Ref(Box<Type>),
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Bool          => write!(f, "bool"),
            Type::String        => write!(f, "String"),
            Type::Unknown       => write!(f, "Unknown"),
            Type::Void          => write!(f, "Void"),

            Type::Float { bits }        => write!(f, "float{}", bits),
            Type::Int { bits, signed }  => write!(f, "{}int{}", if *signed { "" } else { "u" }, bits),

            Type::Object(name)  => write!(f, "Object({})", name),
            Type::Enum(name)    => write!(f, "Enum({})", name),
            Type::Class(name)   => write!(f, "Class({})", name),
            Type::Ref(inner)    => write!(f, "Ref({})", inner),
        }
    }
}

impl<'ctx> Type {
    /// The default type of integer literals without context.
    pub const INT32: Type = Type::Int { bits: 32, signed: true };
    /// The default type of float literals without context.
    pub const FLOAT64: Type = Type::Float { bits: 64 };

    /// Name of the class or enum this type refers to, also through a reference.
    /// `None` for every other type.
    pub fn class_name(&self) -> Option<String> {
        match self {
            Type::Class(name) | Type::Enum(name) => Some(name.clone()),
            Type::Ref(inner) => inner.class_name(),
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
            Type::String => ctx.context.ptr_type(AddressSpace::default()).into(),
            Type::Unknown => panic!("Unknown type reached codegen — sema did not resolve it"),
            Type::Void => panic!("Void has no LLVM basic type"),

            Type::Float { bits: 32 } => ctx.context.f32_type().into(),
            Type::Float { .. } => ctx.context.f64_type().into(),
            Type::Int { bits, .. } => llvm_int_type(*bits, ctx).into(),

            Type::Bool => ctx.context.bool_type().into(),
            Type::Object(_) => panic!("Object type reached codegen — sema did not canonicalize it"),
            Type::Enum(name) => ctx.module.get_struct_type(name).expect("enum not defined").into(),
            Type::Class(name) => ctx.module.get_struct_type(name).expect("struct not defined").into(),
            Type::Ref(_) => ctx.context.ptr_type(AddressSpace::default()).into(),
        }
    }

    pub fn to_fn_type(&self, param_types: &[BasicMetadataTypeEnum<'ctx>], ctx: &CodegenContext<'ctx>) -> FunctionType<'ctx> {
        match self {
            Type::String => ctx.context.ptr_type(AddressSpace::default()).fn_type(param_types, false),
            Type::Unknown => panic!("Unknown type reached codegen — sema did not resolve it"),
            Type::Void => ctx.context.void_type().fn_type(param_types, false),

            Type::Float { bits: 32 } => ctx.context.f32_type().fn_type(param_types, false),
            Type::Float { .. } => ctx.context.f64_type().fn_type(param_types, false),
            Type::Int { bits, .. } => llvm_int_type(*bits, ctx).fn_type(param_types, false),

            Type::Bool => ctx.context.bool_type().fn_type(param_types, false),
            Type::Object(_) => panic!("Object type reached codegen — sema did not canonicalize it"),
            Type::Enum(name) => ctx.module.get_struct_type(name).expect("enum not defined").fn_type(param_types, false),
            Type::Class(name) => ctx.module.get_struct_type(name).expect("struct not defined").fn_type(param_types, false),
            Type::Ref(_) => ctx.context.ptr_type(AddressSpace::default()).fn_type(param_types, false),
        }
    }

    pub fn formatter(&self) -> Option<&str> {
        let f = match self {
            Type::Bool => "%s",
            Type::Float { .. } => "%f",
            Type::Int { bits: 64, signed: true } => "%lld",
            Type::Int { bits: 64, signed: false } => "%llu",
            Type::Int { signed: true, .. } => "%d",
            Type::Int { signed: false, .. } => "%u",
            Type::String => "%s",

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
            ExprKind::StringLiteral(_) => Type::String,
            ExprKind::Boolean(_) => Type::Bool,
            _ => Type::Unknown
        }
    }
}

