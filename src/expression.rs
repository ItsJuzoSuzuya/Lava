use std::fmt::Display;
use std::ops::Neg;

use inkwell::AddressSpace;
use inkwell::basic_block::BasicBlock;
use inkwell::intrinsics::Intrinsic;
use inkwell::module::Linkage;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use serde::Serialize;

use crate::codegen_ctx::{CodegenError, CodegenContext};
use crate::span::Span;
use crate::token::Token;
use crate::traits::codegen::Codegen;
use crate::r#type::Type;

#[derive(Debug, Serialize, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub ty: Type,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span, ty: Type::Unknown }
    }
}

impl Neg for Expr {
    type Output = Expr;

    /// Negates the expression.
    ///
    /// Number literals are negated right away, so `-5` becomes the literal `-5`.
    /// Anything else gets wrapped in an [`ExprKind::Negation`] that is negated at runtime.
    fn neg(self) -> Expr {
        let span = self.span.clone();
        match self.kind {
            ExprKind::IntLiteral(value)   => Expr::new(ExprKind::IntLiteral(-value), span),
            ExprKind::FloatLiteral(value) => Expr::new(ExprKind::FloatLiteral(-value), span),
            _ => Expr::new(ExprKind::Negation(Box::new(self)), span),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub enum ExprKind {
    Boolean(bool),
    /// Conversion of `value` to the number type `target`.
    ///
    /// Written as `value as target`, or inserted by sema where a value
    /// [widens](Type::widens_to) losslessly. Narrowing ints wrap, float to int
    /// cuts off the fraction and saturates at the target's limits (NaN becomes 0),
    /// `float64` to `float32` rounds and `bool` becomes 0 or 1.
    Convert {
        value: Box<Expr>,
        target: Type,
    },
    /// Float literal, its exact float type is decided by sema from context.
    FloatLiteral(f64),
    Identifier(String),
    /// Integer literal, its exact integer type is decided by sema from context.
    IntLiteral(i64),
    /// `-expr` on anything that isn't a number literal.
    Negation(Box<Expr>),
    /// Moves the value onto the heap, giving a `Unique`. Sema creates it from
    /// a call of the built-in `unique(value)`.
    Unique(Box<Expr>),
    /// A new `Buffer` with room for `capacity` elements, of the element type in
    /// this expression's type. Sema creates it from a call of `Buffer<T>(capacity)`.
    NewBuffer(Box<Expr>),
    Print(Box<Expr>),
    Ref(Box<Expr>),
    StringLiteral(String),

    Assignment {
        lhs: Box<Expr>,
        rhs: Box<Expr>
    },
    BinaryExpr {
        lhs: Box<Expr>,
        op: Token,
        rhs: Box<Expr>,
    },
    ColonAccess {
        name: String,
        tag: String,
    },
    /// `object.field`, where `object` can be any expression like `a.b` or `make()`.
    FieldAccess {
        object: Box<Expr>,
        field: String,
    },
    FunctionCall {
        name: String,
        /// Explicit type arguments like `int64` in `max<int64>(3, 7)`, empty otherwise.
        type_args: Vec<Type>,
        params: Vec<Expr>,
        /// The object a method is called on, like `a.b` in `a.b.get()`. `None`
        /// for free functions and for method calls without receiver inside a method.
        receiver: Option<Box<Expr>>,
        mangled: Option<String>,
    },
    ModuleImport {
        name: String,
        module: Vec<Expr>,
        objects: Option<Vec<Expr>>
    },

    EnumInit {
        name: String,
        /// Explicit type arguments of a generic enum, like `int32` in `Option<int32>::Some(5)`.
        type_args: Vec<Type>,
        tag: String,
        value: Box<Expr>
    },
    StructInit {
        class: String,
        /// Explicit type arguments of a generic class, like `int32` in `Box<int32> { value: 5 }`.
        type_args: Vec<Type>,
        fields: Vec<(String, Expr)>,
    },
}

impl Display for ExprKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExprKind::Boolean(val)                     => write!(f, "{}", val),
            ExprKind::Convert { value, target }        => write!(f, "{} as {}", value.kind, target),
            ExprKind::FloatLiteral(value)              => write!(f, "{:?}", value),
            ExprKind::Identifier(name)                 => write!(f, "{}", name),
            ExprKind::IntLiteral(value)                => write!(f, "{}", value),
            ExprKind::Negation(inner)                  => write!(f, "-{}", inner.kind),
            ExprKind::Unique(inner)                    => write!(f, "unique({})", inner.kind),
            ExprKind::NewBuffer(capacity)              => write!(f, "Buffer({})", capacity.kind),
            ExprKind::Print(expr)                      => write!(f, "Print({})", expr.kind),
            ExprKind::Ref(inner)                       => write!(f, "&{}", inner.kind),
            ExprKind::StringLiteral(lit)               => write!(f, "\"{}\"", lit),

            ExprKind::Assignment { lhs, rhs }          => write!(f, "Assignment({} = {})", lhs.kind, rhs.kind),
            ExprKind::BinaryExpr { lhs, op, rhs }      => write!(f, "{} {} {}", lhs.kind, op, rhs.kind),
            ExprKind::ColonAccess { name, tag }        => write!(f, "{}::{}", name, tag),
            ExprKind::FieldAccess { object, field }    => write!(f, "{}.{}", object.kind, field),
            ExprKind::FunctionCall { name, .. }        => write!(f, "{}()", name),
            ExprKind::ModuleImport { name, .. }              => write!(f, "{}", name),

            ExprKind::EnumInit { name, .. }            => write!(f, "{}(..)", name),
            ExprKind::StructInit { class, .. }         => write!(f, "{} {{ .. }}", class),
        }
    }
}

impl Expr {
    /// Emits the value of this expression, see [`Codegen::codegen`] for the
    /// version that also frees the temporaries it leaves behind.
    fn codegen_value<'ctx>(&self, ctx: &mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        let span = &self.span;
        match &self.kind {
            ExprKind::Boolean(val) => {
                let int = match val {
                    true => 1,
                    false => 0
                };
                Some(Box::new(ctx.context.bool_type().const_int(int, false).into()))
            }
            ExprKind::Identifier(name) => {
                let value = match ctx.load_var(name) {
                    Some(v) => v,
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedVariable(name.clone()).throw(&source, span)
                    }
                };
                // Using a `Unique` or `Buffer` as a value moves it, the variable doesn't own it
                // anymore. A class owning memory is copied instead, with memory of its own
                if let Some(owned) = ctx.scope.get(name).and_then(|s| s.owned.clone()) {
                    if owned.ty.is_move_only() {
                        ctx.builder.build_store(owned.alive, ctx.context.bool_type().const_int(0, false)).unwrap();
                    } else {
                        return Some(Box::new(ctx.deep_clone(value, &owned.ty)));
                    }
                }
                Some(Box::new(value))
            }
            ExprKind::Unique(inner) => {
                let value = *inner.codegen(ctx).unwrap();
                let size = inner.ty.to_llvm(ctx).size_of().expect("heap value has no size");
                let malloc = ctx.libc_function("malloc");
                let heap = ctx.builder.build_call(malloc, &[size.into()], "heap").unwrap()
                    .try_as_basic_value().basic().unwrap().into_pointer_value();
                ctx.builder.build_store(heap, value).unwrap();
                Some(Box::new(heap.into()))
            }
            ExprKind::NewBuffer(capacity) => {
                let Type::Buffer(element) = &self.ty else {
                    panic!("a new Buffer has the type '{}'", self.ty);
                };
                let capacity = capacity.codegen(ctx).unwrap().into_int_value();
                let element_size = element.to_llvm(ctx).size_of().expect("Buffer element has no size");

                // Zeroed, so the slots of owning elements start out as empty null pointers
                let calloc = ctx.libc_function("calloc");
                let data = ctx.builder.build_call(calloc, &[capacity.into(), element_size.into()], "data").unwrap()
                    .try_as_basic_value().basic().unwrap();
                let malloc = ctx.libc_function("malloc");
                let header_size = ctx.buffer_header_type().size_of().unwrap();
                let header = ctx.builder.build_call(malloc, &[header_size.into()], "buffer").unwrap()
                    .try_as_basic_value().basic().unwrap().into_pointer_value();

                ctx.builder.build_store(ctx.buffer_capacity_ptr(header), capacity).unwrap();
                ctx.builder.build_store(ctx.buffer_data_ptr(header), data).unwrap();
                Some(Box::new(header.into()))
            }
            ExprKind::Convert { value, target } => {
                let llvm_value = *value.codegen(ctx).unwrap();
                Some(Box::new(build_conversion(ctx, llvm_value, &value.ty, target)))
            }
            ExprKind::FloatLiteral(value) => {
                let float_ty = self.ty.to_llvm(ctx).into_float_type();
                Some(Box::new(float_ty.const_float(*value).into()))
            }
            ExprKind::IntLiteral(value) => {
                let int_ty = self.ty.to_llvm(ctx).into_int_type();
                Some(Box::new(int_ty.const_int(*value as u64, true).into()))
            }
            ExprKind::Negation(inner) => {
                let value = *inner.codegen(ctx).unwrap();
                let negated: BasicValueEnum = match &self.ty {
                    Type::Float { .. } => ctx.builder.build_float_neg(value.into_float_value(), "tmpneg").unwrap().into(),
                    _ => ctx.builder.build_int_neg(value.into_int_value(), "tmpneg").unwrap().into(),
                };
                Some(Box::new(negated))
            }
            ExprKind::Print(expr) => {
                let printfn = match ctx.module.get_function("printf") {
                    Some(x) => x,
                    None => create_printfn(ctx),
                };

                match &expr.ty {
                    Type::Bool => {
                        // The value is only known at runtime, so pick the string there
                        let value = expr.codegen(ctx).unwrap().into_int_value();
                        let true_str = ctx.builder.build_global_string_ptr("true", "true_str").unwrap();
                        let false_str = ctx.builder.build_global_string_ptr("false", "false_str").unwrap();
                        let chosen = ctx.builder.build_select(
                            value,
                            true_str.as_pointer_value(),
                            false_str.as_pointer_value(),
                            "bool_str",
                        ).unwrap();

                        let fmt = ctx.builder.build_global_string_ptr("%s\n", "fmt").unwrap();
                        ctx.builder.build_call(
                            printfn,
                            &[fmt.as_pointer_value().into(), chosen.into()],
                            "printFn",
                        ).unwrap();
                    }
                    Type::Enum(name) => {
                        let printer_name = format!("__print_{}", name);
                        let printer = match ctx.module.get_function(&printer_name) {
                            Some(f) => f,
                            None => create_enum_printer(ctx, name, printfn),
                        };
                        let ptr = match expr.codegen_place(ctx) {
                            Some(p) => p,
                            None => {
                                let val = *expr.codegen(ctx).unwrap();
                                let ty = expr.ty.to_llvm(ctx);
                                let tmp = ctx.build_entry_alloca(ty, "tmp_enum");
                                ctx.builder.build_store(tmp, val).unwrap();
                                tmp
                            }
                        };
                        ctx.builder.build_call(printer, &[ptr.into()], "").unwrap();
                    }
                    _ => {
                        let expr_value = expr.codegen(ctx).unwrap();
                        let signed = !matches!(expr.ty, Type::Int { signed: false, .. });
                        let expr_value = promote_vararg(ctx, *expr_value, signed);
                        let formatter = expr.ty.formatter().unwrap_or_else(|| CodegenError::UndefinedFormatter(expr.ty.clone()).throw(&ctx.source, span));
                        let format_str = ctx.builder.build_global_string_ptr((formatter.to_owned() + &"\n").as_str(), "fmt").unwrap();
                        let args = &[format_str.as_pointer_value().into(), expr_value.into()];
                        ctx.builder.build_call(printfn, args, "printFn").unwrap();
                    }
                }

                None
            }
            ExprKind::Ref(inner) => {
                let ptr = inner.codegen_place(ctx)
                    .unwrap_or_else(|| panic!("'&' operand is not a place expression"));
                Some(Box::new(ptr.into()))
            },
            ExprKind::StringLiteral(lit) => {
                let string = ctx.builder.build_global_string_ptr(lit, "str").unwrap();
                Some(Box::new(string.as_pointer_value().into()))
            },

            ExprKind::Assignment { lhs, rhs } => {
                let ptr = lhs.codegen_place(ctx)
                    .unwrap_or_else(|| panic!("assignment lhs is not a place expression"));
                let llvm_val = *rhs.codegen(ctx).unwrap();

                // An owning variable frees its old value before owning the new one
                let owned = match &lhs.kind {
                    ExprKind::Identifier(name) => ctx.scope.get(name).and_then(|s| s.owned.clone()),
                    _ => None,
                };
                if let Some(owned) = &owned {
                    ctx.emit_drop(owned);
                } else if matches!(lhs.kind, ExprKind::FieldAccess { .. }) && ctx.owns_memory(&lhs.ty) {
                    // So does an owning field, its object keeps owning only the new value
                    ctx.drop_in_place(ptr, &lhs.ty);
                }
                ctx.builder.build_store(ptr, llvm_val).unwrap();
                if let Some(owned) = &owned {
                    ctx.builder.build_store(owned.alive, ctx.context.bool_type().const_int(1, false)).unwrap();
                }

                None
            },
            ExprKind::BinaryExpr { lhs, op, rhs } => {
                let lhs_value = *lhs.codegen(ctx).unwrap();
                let rhs_value = *rhs.codegen(ctx).unwrap();

                if op.is_comparison() {
                    return Some(Box::new(build_comparison(ctx, op, lhs_value, rhs_value, &lhs.ty).into()));
                }

                // Sema made both sides the same type as the whole expression.
                let result: BasicValueEnum = match (&self.ty, op) {
                    (Type::Int { signed, .. }, _) => {
                        let (l, r) = (lhs_value.into_int_value(), rhs_value.into_int_value());
                        match op {
                            Token::Plus     => ctx.builder.build_int_add(l, r, "tmpadd"),
                            Token::Minus    => ctx.builder.build_int_sub(l, r, "tmpsub"),
                            Token::Multiply => ctx.builder.build_int_mul(l, r, "tmpmul"),
                            Token::Divide if *signed => ctx.builder.build_int_signed_div(l, r, "tmpdiv"),
                            Token::Divide   => ctx.builder.build_int_unsigned_div(l, r, "tmpdiv"),
                            other => {
                                let source = ctx.source.clone();
                                CodegenError::UnsupportedOperator(other.to_string()).throw(&source, span)
                            }
                        }.unwrap().into()
                    }
                    (Type::Float { .. }, _) => {
                        let (l, r) = (lhs_value.into_float_value(), rhs_value.into_float_value());
                        match op {
                            Token::Plus     => ctx.builder.build_float_add(l, r, "tmpadd"),
                            Token::Minus    => ctx.builder.build_float_sub(l, r, "tmpsub"),
                            Token::Multiply => ctx.builder.build_float_mul(l, r, "tmpmul"),
                            Token::Divide   => ctx.builder.build_float_div(l, r, "tmpdiv"),
                            other => {
                                let source = ctx.source.clone();
                                CodegenError::UnsupportedOperator(other.to_string()).throw(&source, span)
                            }
                        }.unwrap().into()
                    }
                    (_, other) => {
                        let source = ctx.source.clone();
                        CodegenError::UnsupportedOperator(other.to_string()).throw(&source, span)
                    }
                };
                Some(Box::new(result))
            }
            ExprKind::ColonAccess { name, tag } => {
                let variants = match ctx.enums.get(name) {
                    Some(v) => v,
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedClass(name.clone()).throw(&source, span)
                    }
                };
                let discriminant = variants.iter()
                    .position(|(n, _)| *n == tag.to_string())
                    .unwrap_or_else(|| {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedField {
                            class: name.clone(),
                            field: tag.to_string(),
                        }.throw(&source, span)
                    });
                let value = ctx.context.i32_type().const_int(discriminant as u64, false);
                Some(Box::new(value.into()))
            },
            ExprKind::FieldAccess { object, field } => {
                let (field_ptr, field_type) = field_address(ctx, object, field, span);
                let field_val = ctx.builder.build_load(field_type, field_ptr, "tmpfield").unwrap();
                // The object keeps owning its field, so the value used here is a copy
                Some(Box::new(ctx.deep_clone(field_val, &self.ty)))
            }
            ExprKind::FunctionCall { name: _, type_args: _, params, receiver, mangled } => {
                let mangled = mangled.as_ref()
                    .expect("function call mangled name missing — sema must run first");

                // Methods of `Buffer` are built in, sema marks them like `Buffer.get`
                if let Some(method) = mangled.strip_prefix("Buffer.") {
                    let receiver = receiver.as_ref().expect("Buffer methods have a receiver");
                    return codegen_buffer_method(ctx, method, receiver, params, span).map(Box::new);
                }

                // A method gets the address of its receiver as `self`
                let target_has_self = ctx.func_has_self(mangled);
                let parent_sym = if !target_has_self {
                    None
                } else {
                    match receiver {
                        Some(receiver) => Some(object_address(ctx, receiver, span)),
                        // Inside a method, a call without receiver goes to `self`. Checked by
                        // scope, since instances of generic methods are generated outside their class
                        None if ctx.scope.contains_key("self") => Some(variable_address(ctx, "self", span)),
                        None => None,
                    }
                };
                // Put all Params into the function (parent is always first parent)
                let mut llvm_params: Vec<BasicMetadataValueEnum> = match parent_sym {
                    Some(sym) => vec![sym.into()],
                    None => Vec::new()
                };

                let decl_info: Option<Vec<(String, Type, Option<Expr>)>> = ctx
                    .load_func_params(mangled)
                    .map(|ps| ps.iter()
                        .map(|p| (p.name.clone(), p.ty.clone(), p.value.as_ref().map(|v| *v.clone())))
                        .collect());

                match decl_info {
                    Some(infos) => {
                        let arg_infos: Vec<_> = infos.iter().filter(|(n, _, _)| n != "self").collect();
                        for (i, (_, def_ty, default)) in arg_infos.iter().enumerate() {
                            let value: BasicValueEnum = match params.get(i) {
                                Some(passed) => {
                                    let needs_ref = matches!(def_ty, Type::Ref(_))
                                        && !matches!(passed.ty, Type::Ref(_));
                                    if needs_ref {
                                        let ptr = passed.codegen_place(ctx).unwrap_or_else(|| {
                                            panic!("cannot take address of non-place expression for arg {}", i)
                                        });
                                        ptr.into()
                                    } else {
                                        *passed.codegen(ctx).unwrap()
                                    }
                                },
                                None => {
                                    let default_expr = default.as_ref().unwrap_or_else(|| {
                                        let source = ctx.source.clone();
                                        CodegenError::MissingArgument { index: i, function: mangled.clone() }
                                            .throw(&source, span)
                                    });
                                    *default_expr.codegen(ctx).unwrap()
                                }
                            };
                            llvm_params.push(value.into());
                        }
                    }
                    None => {
                        for param in params {
                            llvm_params.push((*param.codegen(ctx).unwrap()).into());
                        }
                    }
                }

                let function = match ctx.module.get_function(mangled) {
                    Some(f) => f,
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedFunction(mangled.clone()).throw(&source, span)
                    }
                };

                let return_val = ctx.builder.build_call(function, &llvm_params, "fn")
                    .unwrap_or_else(|_| panic!("function '{}' could not be called", mangled));
                Some(Box::new(return_val.try_as_basic_value().basic()?))
            }
            ExprKind::ModuleImport { .. } => None,
            ExprKind::EnumInit { .. } => {
                unreachable!("EnumInit must be lowered to FunctionCall by sema before codegen")
            }
            ExprKind::StructInit { .. } => {
                unreachable!("StructInit must be lowered to FunctionCall by sema before codegen")
            }
        }
    }
}

impl Codegen for Expr {
    /// Emits the value of this expression and frees the owning temporaries
    /// created on the way, like the `Unique` returned by `v.get(0)` in `v.get(0).x`.
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        let mark = ctx.temporaries.len();
        let value = self.codegen_value(ctx);
        ctx.drop_temporaries(mark);
        value
    }

    fn codegen_place<'ctx>(&self, ctx: &mut CodegenContext<'ctx>)
              -> Option<PointerValue<'ctx>> {
        let span = &self.span;
        match &self.kind {
            ExprKind::Identifier(name) => {
                match ctx.load_var_ptr(name) {
                    Some(v) => Some(v),
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedVariable(name.clone()).throw(&source, span)
                    }
                }
            }
            ExprKind::FieldAccess { object, field } => {
                Some(field_address(ctx, object, field, span).0)
            }
            _ => None,
        }
    }
}

/// Address of the value of the variable `name`, for accessing its fields or
/// passing it as `self`.
///
/// A variable holding a pointer to an object (`self` of a class method, a
/// reference or a `Unique`) gives that pointer. Every other variable, Strings
/// too, gives the address of its own stack slot.
fn variable_address<'ctx>(ctx: &mut CodegenContext<'ctx>, name: &str, span: &Span) -> PointerValue<'ctx> {
    let (ty, ptr, points_to_object) = match ctx.scope.get(name) {
        Some(s) => (s.ty, s.ptr, s.class_name.is_some() || s.owned.is_some()),
        None => {
            let source = ctx.source.clone();
            CodegenError::UndefinedVariable(name.to_string()).throw(&source, span)
        }
    };
    if ty.is_pointer_type() && points_to_object {
        ctx.builder.build_load(ty, ptr, "object").unwrap().into_pointer_value()
    } else {
        ptr
    }
}

/// Address of the value `object` evaluates to, for accessing its fields or
/// passing it as `self`.
///
/// Variables and fields give the address where their value lives, through a
/// reference or `Unique` the address it points to. Other values, like the
/// result of a call, are stored in a temporary slot first.
fn object_address<'ctx>(ctx: &mut CodegenContext<'ctx>, object: &Expr, span: &Span) -> PointerValue<'ctx> {
    if let ExprKind::Identifier(name) = &object.kind {
        return variable_address(ctx, name, span);
    }

    let points_elsewhere = matches!(object.ty, Type::Ref(_) | Type::Unique(_) | Type::Buffer(_));
    if let Some(place) = object.codegen_place(ctx) {
        if points_elsewhere {
            let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
            return ctx.builder.build_load(ptr_ty, place, "object").unwrap().into_pointer_value();
        }
        return place;
    }

    // A temporary like the result of a call, freed when the expression using it ends
    let value = *object.codegen(ctx).unwrap();
    let owns_memory = ctx.owns_memory(&object.ty);
    if points_elsewhere {
        if owns_memory {
            ctx.push_temporary(value, &object.ty);
        }
        return value.into_pointer_value();
    }
    if owns_memory {
        ctx.push_temporary(value, &object.ty);
        return ctx.temporaries.last().unwrap().0;
    }
    let slot = ctx.build_entry_alloca(value.get_type(), "temp_object");
    ctx.builder.build_store(slot, value).unwrap();
    slot
}

/// Address and LLVM type of the field `field` of the class value `object`,
/// or of the tag of an enum value for `tag`.
fn field_address<'ctx>(ctx: &mut CodegenContext<'ctx>, object: &Expr, field: &str, span: &Span) -> (PointerValue<'ctx>, BasicTypeEnum<'ctx>) {
    let class_name = object.ty.class_name()
        .unwrap_or_else(|| panic!("field access on a value of type '{}'", object.ty));
    let object_ptr = object_address(ctx, object, span);
    let struct_ty = ctx.module.get_struct_type(&class_name).expect("struct not defined");

    if ctx.enums.contains_key(&class_name) {
        if field != "tag" {
            let source = ctx.source.clone();
            CodegenError::UndefinedField { class: class_name, field: field.to_string() }.throw(&source, span)
        }
        let tag_ptr = ctx.builder.build_struct_gep(struct_ty, object_ptr, 0, "tag_ptr").unwrap();
        return (tag_ptr, ctx.context.i32_type().into());
    }

    let class_fields = match ctx.classes.get(&class_name) {
        Some(f) => f.clone(),
        None => {
            let source = ctx.source.clone();
            CodegenError::UndefinedClass(class_name).throw(&source, span)
        }
    };
    let Some((index, (_, field_type))) = class_fields.iter().enumerate().find(|(_, (n, _))| n == field) else {
        let source = ctx.source.clone();
        CodegenError::UndefinedField { class: class_name, field: field.to_string() }.throw(&source, span)
    };
    let field_ptr = ctx.builder.build_struct_gep(struct_ty, object_ptr, index as u32, field).unwrap();
    (field_ptr, *field_type)
}

/// Generates a call of the built-in `Buffer` method `method` on `receiver`.
///
/// `get(i)` and `set(i, value)` check that `i` is below the capacity and stop
/// the program otherwise. `resize(n)` keeps the first elements and leaves
/// new slots uninitialized, like `realloc`.
fn codegen_buffer_method<'ctx>(ctx: &mut CodegenContext<'ctx>, method: &str, receiver: &Expr, args: &[Expr], span: &Span) -> Option<BasicValueEnum<'ctx>> {
    let element = match &receiver.ty {
        Type::Buffer(element) => element.as_ref().clone(),
        Type::Ref(inner) | Type::Unique(inner) => match inner.as_ref() {
            Type::Buffer(element) => element.as_ref().clone(),
            other => panic!("'{}' isn't a Buffer", other),
        },
        other => panic!("'{}' isn't a Buffer", other),
    };
    let element_ty = element.to_llvm(ctx);
    let header = object_address(ctx, receiver, span);

    match method {
        "get" | "set" => {
            let index = args[0].codegen(ctx).unwrap().into_int_value();
            // The value is computed before the check, like any argument
            let value = if method == "set" { Some(*args[1].codegen(ctx).unwrap()) } else { None };
            ctx.build_bounds_check(header, index);
            let data = ctx.buffer_data(header);
            let slot = unsafe { ctx.builder.build_in_bounds_gep(element_ty, data, &[index], "element").unwrap() };
            match value {
                Some(value) => {
                    // The overwritten element is gone, so is the memory it owns
                    ctx.drop_in_place(slot, &element);
                    ctx.builder.build_store(slot, value).unwrap();
                    None
                }
                // The element stays in the Buffer, the caller gets its own copy
                None => Some(ctx.deep_clone(ctx.builder.build_load(element_ty, slot, "element").unwrap(), &element)),
            }
        }
        "capacity" => {
            let capacity_ptr = ctx.buffer_capacity_ptr(header);
            Some(ctx.builder.build_load(ctx.context.i64_type(), capacity_ptr, "capacity").unwrap())
        }
        "resize" => {
            let capacity = args[0].codegen(ctx).unwrap().into_int_value();
            let old_capacity = ctx.builder.build_load(ctx.context.i64_type(), ctx.buffer_capacity_ptr(header), "old_capacity")
                .unwrap().into_int_value();

            // Shrinking frees what the cut off elements own
            if ctx.owns_memory(&element) {
                ctx.build_index_loop(capacity, old_capacity, |i| ctx.drop_in_place(ctx.buffer_slot(header, &element, i), &element));
            }

            let element_size = element_ty.size_of().expect("Buffer element has no size");
            let bytes = ctx.builder.build_int_mul(capacity, element_size, "bytes").unwrap();
            let data = ctx.buffer_data(header);
            let realloc = ctx.libc_function("realloc");
            let resized = ctx.builder.build_call(realloc, &[data.into(), bytes.into()], "resized").unwrap()
                .try_as_basic_value().basic().unwrap().into_pointer_value();
            ctx.builder.build_store(ctx.buffer_data_ptr(header), resized).unwrap();
            ctx.builder.build_store(ctx.buffer_capacity_ptr(header), capacity).unwrap();

            // Growing zeroes the new slots, like a new Buffer
            let grows = ctx.builder.build_int_compare(inkwell::IntPredicate::UGT, capacity, old_capacity, "grows").unwrap();
            let kept = ctx.builder.build_select(grows, old_capacity, capacity, "kept").unwrap().into_int_value();
            let added = ctx.builder.build_int_sub(capacity, kept, "added").unwrap();
            let added_bytes = ctx.builder.build_int_mul(added, element_size, "added_bytes").unwrap();
            let tail = unsafe { ctx.builder.build_in_bounds_gep(element_ty, resized, &[kept], "tail").unwrap() };
            let memset = ctx.libc_function("memset");
            let zero = ctx.context.i32_type().const_zero();
            ctx.builder.build_call(memset, &[tail.into(), zero.into(), added_bytes.into()], "").unwrap();
            None
        }
        other => panic!("Buffer has no method '{}', sema must reject it", other),
    }
}

fn create_printfn<'ctx>(ctx: &CodegenContext<'ctx>) -> FunctionValue<'ctx> {
    let context = ctx.context;
    let i32_type = context.i32_type();
    let str_ptr_type = context.ptr_type(AddressSpace::default());
    // printf has no body here - it's just a prototype so LLVM can typecheck
    // the call; the real definition is resolved against libc at link time.
    let printf_type = i32_type.fn_type(&[str_ptr_type.into()], true);
    ctx.module.add_function("printf", printf_type, Some(Linkage::External))
}

fn printf_format_for(ty: &BasicTypeEnum<'_>) -> &'static str {
    match ty {
        BasicTypeEnum::IntType(int) if int.get_bit_width() == 64 => "%lld",
        BasicTypeEnum::IntType(_)     => "%d",
        BasicTypeEnum::PointerType(_) => "%s",
        BasicTypeEnum::FloatType(_)   => "%f",
        _ => "%d",
    }
}

/// Compares `lhs` and `rhs` of the number or bool type `ty` with the comparison operator `op`.
///
/// Ints compare by their signedness, so `uint8` 200 is greater than 100 but
/// `int8` -56 isn't. Floats use ordered comparisons, which are false if a side is NaN.
fn build_comparison<'ctx>(ctx: &CodegenContext<'ctx>, op: &Token, lhs: BasicValueEnum<'ctx>, rhs: BasicValueEnum<'ctx>, ty: &Type) -> IntValue<'ctx> {
    use inkwell::{FloatPredicate, IntPredicate};

    match ty {
        Type::Float { .. } => {
            let predicate = match op {
                Token::EqEq  => FloatPredicate::OEQ,
                // Not equal is also true when a side is NaN
                Token::NotEq => FloatPredicate::UNE,
                Token::Lt    => FloatPredicate::OLT,
                Token::LtEq  => FloatPredicate::OLE,
                Token::Gt    => FloatPredicate::OGT,
                Token::GtEq  => FloatPredicate::OGE,
                other => panic!("'{}' is not a comparison", other),
            };
            ctx.builder.build_float_compare(predicate, lhs.into_float_value(), rhs.into_float_value(), "cmp").unwrap()
        }
        _ => {
            let signed = matches!(ty, Type::Int { signed: true, .. });
            let predicate = match (op, signed) {
                (Token::EqEq, _)      => IntPredicate::EQ,
                (Token::NotEq, _)     => IntPredicate::NE,
                (Token::Lt, true)     => IntPredicate::SLT,
                (Token::Lt, false)    => IntPredicate::ULT,
                (Token::LtEq, true)   => IntPredicate::SLE,
                (Token::LtEq, false)  => IntPredicate::ULE,
                (Token::Gt, true)     => IntPredicate::SGT,
                (Token::Gt, false)    => IntPredicate::UGT,
                (Token::GtEq, true)   => IntPredicate::SGE,
                (Token::GtEq, false)  => IntPredicate::UGE,
                (other, _) => panic!("'{}' is not a comparison", other),
            };
            ctx.builder.build_int_compare(predicate, lhs.into_int_value(), rhs.into_int_value(), "cmp").unwrap()
        }
    }
}

/// Converts `value` from the number or bool type `from` to the number type `to`.
///
/// Ints extend by their signedness and truncate when narrowing, so they wrap.
/// Float to int uses LLVM's saturating conversion: the fraction is cut off,
/// values beyond the target's range become its min or max, and NaN becomes 0.
/// Plain `fptosi`/`fptoui` would give poison for those values instead.
///
/// # Panics
///
/// Panics if `from` or `to` isn't a number type, `from` may also be `bool`.
/// Sema only allows conversions between those.
fn build_conversion<'ctx>(ctx: &CodegenContext<'ctx>, value: BasicValueEnum<'ctx>, from: &Type, to: &Type) -> BasicValueEnum<'ctx> {
    let target = to.to_llvm(ctx);

    // bool is an unsigned one bit int
    let from = match from {
        Type::Bool => &Type::Int { bits: 1, signed: false },
        other => other,
    };

    match (from, to) {
        (Type::Int { bits: from_bits, signed }, Type::Int { bits: to_bits, .. }) => {
            let int = value.into_int_value();
            let target = target.into_int_type();
            if to_bits > from_bits && *signed {
                ctx.builder.build_int_s_extend(int, target, "conv").unwrap().into()
            } else if to_bits > from_bits {
                ctx.builder.build_int_z_extend(int, target, "conv").unwrap().into()
            } else if to_bits < from_bits {
                ctx.builder.build_int_truncate(int, target, "conv").unwrap().into()
            } else {
                // Same width, LLVM ints don't know their signedness
                value
            }
        }
        (Type::Int { signed: true, .. }, Type::Float { .. }) =>
            ctx.builder.build_signed_int_to_float(value.into_int_value(), target.into_float_type(), "conv").unwrap().into(),
        (Type::Int { signed: false, .. }, Type::Float { .. }) =>
            ctx.builder.build_unsigned_int_to_float(value.into_int_value(), target.into_float_type(), "conv").unwrap().into(),
        (Type::Float { .. }, Type::Int { signed, .. }) => {
            let intrinsic = if *signed { "llvm.fptosi.sat" } else { "llvm.fptoui.sat" };
            let float = value.into_float_value();
            let declaration = Intrinsic::find(intrinsic).unwrap()
                .get_declaration(&ctx.module, &[target, float.get_type().into()]).unwrap();
            ctx.builder.build_call(declaration, &[float.into()], "conv").unwrap()
                .try_as_basic_value().basic().unwrap()
        }
        (Type::Float { bits: from_bits }, Type::Float { bits: to_bits }) => {
            let float = value.into_float_value();
            let target = target.into_float_type();
            if to_bits > from_bits {
                ctx.builder.build_float_ext(float, target, "conv").unwrap().into()
            } else if to_bits < from_bits {
                ctx.builder.build_float_trunc(float, target, "conv").unwrap().into()
            } else {
                value
            }
        }
        (from, to) => panic!("no conversion from '{}' to '{}', sema must reject it", from, to),
    }
}

/// Widens `value` the way C varargs like `printf` expect it: integers narrower
/// than 32 bits to `i32`, `float32` to `float64`. Other values stay unchanged.
fn promote_vararg<'ctx>(ctx: &CodegenContext<'ctx>, value: BasicValueEnum<'ctx>, signed: bool) -> BasicValueEnum<'ctx> {
    let i32_type = ctx.context.i32_type();
    match value {
        BasicValueEnum::IntValue(int) if int.get_type().get_bit_width() < 32 => {
            // bool is an i1, sign extending would turn true into -1
            if signed && int.get_type().get_bit_width() > 1 {
                ctx.builder.build_int_s_extend(int, i32_type, "promoted").unwrap().into()
            } else {
                ctx.builder.build_int_z_extend(int, i32_type, "promoted").unwrap().into()
            }
        }
        BasicValueEnum::FloatValue(float) if float.get_type() == ctx.context.f32_type() => {
            ctx.builder.build_float_ext(float, ctx.context.f64_type(), "promoted").unwrap().into()
        }
        other => other,
    }
}

fn create_enum_printer<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    enum_name: &str,
    printfn: FunctionValue<'ctx>,
) -> FunctionValue<'ctx> {
    let prev_block = ctx.builder.get_insert_block();

    let struct_ty = ctx.module.get_struct_type(enum_name)
        .expect("enum struct not defined");
    let variants: Vec<(String, BasicTypeEnum<'ctx>)> = ctx.enums.get(enum_name)
        .cloned()
        .expect("enum variants missing");

    let void_ty = ctx.context.void_type();
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let i32_ty = ctx.context.i32_type();

    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    let function = ctx.module.add_function(&format!("__print_{}", enum_name), fn_ty, None);

    let entry_bb = ctx.context.append_basic_block(function, "entry");
    let exit_bb = ctx.context.append_basic_block(function, "exit");

    ctx.builder.position_at_end(entry_bb);
    let enum_ptr = function.get_nth_param(0).unwrap().into_pointer_value();
    let tag_ptr = ctx.builder.build_struct_gep(struct_ty, enum_ptr, 0, "tag_ptr").unwrap();
    let tag_val = ctx.builder.build_load(i32_ty, tag_ptr, "tag").unwrap().into_int_value();

    let mut cases: Vec<(IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::with_capacity(variants.len());
    let mut variant_bbs: Vec<BasicBlock<'ctx>> = Vec::with_capacity(variants.len());
    for (i, (variant_name, _)) in variants.iter().enumerate() {
        let bb = ctx.context.append_basic_block(function, &format!("v_{}", variant_name));
        cases.push((i32_ty.const_int(i as u64, false), bb));
        variant_bbs.push(bb);
    }
    ctx.builder.build_switch(tag_val, exit_bb, &cases).unwrap();

    for (bb, (_, payload_ty)) in variant_bbs.iter().zip(variants.iter()) {
        ctx.builder.position_at_end(*bb);
        let payload_ptr = ctx.builder.build_struct_gep(struct_ty, enum_ptr, 1, "payload_ptr").unwrap();
        let payload_val = ctx.builder.build_load(*payload_ty, payload_ptr, "payload").unwrap();

        let fmt = printf_format_for(payload_ty).to_string() + "\n";
        let fmt_str = ctx.builder.build_global_string_ptr(&fmt, "fmt").unwrap();
        // The LLVM type has no signedness, so payloads print as signed
        let payload_val = promote_vararg(ctx, payload_val, true);
        ctx.builder.build_call(
            printfn,
            &[fmt_str.as_pointer_value().into(), payload_val.into()],
            "printFn",
        ).unwrap();
        ctx.builder.build_unconditional_branch(exit_bb).unwrap();
    }

    ctx.builder.position_at_end(exit_bb);
    ctx.builder.build_return(None).unwrap();

    if let Some(prev) = prev_block {
        ctx.builder.position_at_end(prev);
    }
    function
}
