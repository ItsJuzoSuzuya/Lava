use std::fmt::Display;

use inkwell::{AddressSpace, types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType}};
use serde::Serialize;

use crate::{codegen_ctx::CodegenContext, expression::ExprKind};

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Int32,
    String,
    Unknown,
    Void,

    Bool(bool),
    Object(String),
    Enum(String),
    Class(String),
    Ref(Box<Type>),
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int32         => write!(f, "Int32"),
            Type::String        => write!(f, "String"),
            Type::Unknown       => write!(f, "Unknown"),
            Type::Void          => write!(f, "Void"),

            Type::Bool(val)     => write!(f, "{}", val),
            Type::Object(name)  => write!(f, "Object({})", name),
            Type::Enum(name)    => write!(f, "Enum({})", name),
            Type::Class(name)   => write!(f, "Class({})", name),
            Type::Ref(inner)    => write!(f, "Ref({})", inner),
        }
    }
}

impl<'ctx> Type {
    pub fn to_llvm(&self, ctx: &CodegenContext<'ctx>) -> BasicTypeEnum<'ctx> {
        match self {
            Type::Int32 => ctx.context.i32_type().into(),
            Type::String => ctx.context.ptr_type(AddressSpace::default()).into(),
            Type::Unknown => panic!("Unknown type reached codegen — sema did not resolve it"),
            Type::Void => panic!("Void has no LLVM basic type"),

            Type::Bool(_) => ctx.context.bool_type().into(),
            Type::Object(_) => panic!("Object type reached codegen — sema did not canonicalize it"),
            Type::Enum(name) => ctx.module.get_struct_type(name).expect("enum not defined").into(),
            Type::Class(name) => ctx.module.get_struct_type(name).expect("struct not defined").into(),
            Type::Ref(_) => ctx.context.ptr_type(AddressSpace::default()).into(),
        }
    }

    pub fn to_fn_type(&self, param_types: &[BasicMetadataTypeEnum<'ctx>], ctx: &CodegenContext<'ctx>) -> FunctionType<'ctx> {
        match self {
            Type::Int32 => ctx.context.i32_type().fn_type(param_types, false),
            Type::String => ctx.context.ptr_type(AddressSpace::default()).fn_type(param_types, false),
            Type::Unknown => panic!("Unknown type reached codegen — sema did not resolve it"),
            Type::Void => ctx.context.void_type().fn_type(param_types, false),

            Type::Bool(_) => ctx.context.bool_type().fn_type(param_types, false),
            Type::Object(_) => panic!("Object type reached codegen — sema did not canonicalize it"),
            Type::Enum(name) => ctx.module.get_struct_type(name).expect("enum not defined").fn_type(param_types, false),
            Type::Class(name) => ctx.module.get_struct_type(name).expect("struct not defined").fn_type(param_types, false),
            Type::Ref(_) => ctx.context.ptr_type(AddressSpace::default()).fn_type(param_types, false),
        }
    }

    pub fn formatter(&self) -> Option<&str> {
        let f = match self {
            Type::Bool(_) => "%s",
            Type::Int32 => "%d",
            Type::String => "%s",

            Type::Ref(_) => "%p",
            _ => return None
        };

        Some(f)
    }
}

impl From<ExprKind> for Type {
    fn from(value: ExprKind) -> Self {
        match value {
            ExprKind::Identifier(name) => Type::Object(name.to_string()),
            ExprKind::Int32(_) => Type::Int32,
            ExprKind::StringLiteral(_) => Type::String,
            ExprKind::Boolean(val) => Type::Bool(val),
            _ => Type::Unknown
        }
    }
}

