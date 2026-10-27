use std::{fmt::{Display, Formatter, Result}, ops::Not};

use inkwell::{AddressSpace, module::Linkage, types::{BasicMetadataTypeEnum, BasicType}, values::BasicValueEnum};
use serde::Serialize;
use crate::{codegen_ctx::{CodegenContext, function_symbol},field::Field, param::Param, span::Span, statement::Stmt, traits::codegen::{Codegen, Declare}, r#type::Type};

/// Scope of declarations and fields in the global context. 
#[derive(Debug, Clone, Serialize)]
pub enum Visibility {
    Public, Private, Protected
}

/// A type parameter of a generic function, class or enum, like `T: Show + Clone`
/// in `func f<T: Show + Clone>(...)`.
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// Name of the parameter. Parameters created from `impl Trait` shorthands
    /// get a name like `impl Show`, which no written identifier can clash with.
    pub name: String,
    /// Traits every type argument for this parameter has to implement.
    pub bounds: Vec<String>,
}

impl TypeParam {
    /// Whether this parameter comes from an `impl Trait` shorthand. Those are
    /// always inferred and can't be given explicitly as `f<int32>(...)`.
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with("impl ")
    }
}

/// How an `extern func` is linked: the C function it calls.
#[derive(Debug, Clone)]
pub struct Extern {
    /// The C function's name, the declared name unless given like
    /// `extern "exit" func c_exit(code: int32);`.
    pub symbol: String,
    /// Whether it takes more arguments after the declared ones, written `..`
    /// at the end of the parameters, like C's `printf`.
    pub variadic: bool,
}

/// A C thing a program uses besides functions.
#[derive(Clone)]
pub enum ExternItem {
    /// `extern type LLVMModuleRef;`: a C pointer only C functions know what
    /// it points to, passed to and returned from them as it is.
    ///
    /// With a body `{ ... }` it has methods like a class, written in Lava,
    /// usually calling C functions with `self`: `module.dump()`. Functions
    /// without `self` are called like `LLVMModuleRef::create(...)`.
    ///
    /// `extern type IntType: Type` makes it a more specific kind of `base`:
    /// it can be used where a `Type` is expected, it's the same pointer, and
    /// has the methods of `Type` too.
    Type { name: String, base: Option<String>, methods: Vec<Declaration> },
    /// `extern link "LLVM";`: the C library to link the program with, `-lLLVM`.
    Link(String),
}

/// Type of declarations
#[derive(Clone)]
pub enum DeclarationKind {
    Class {
        name: String,
        /// Empty for non-generic classes. Generic classes are only templates,
        /// sema creates a class like `Box<int32>` for every set of type arguments.
        type_params: Vec<TypeParam>,
        fields: Vec<Field>,
        methods: Vec<Declaration>,
    },
    Enum {
        name: String,
        /// Empty for non-generic enums, see [`DeclarationKind::Class`].
        type_params: Vec<TypeParam>,
        fields: Vec<(String, u32, Type, Span)>,
        /// Methods declared after the variants. Their `self` is the enum value.
        methods: Vec<Declaration>,
    },
    Function {
        name: String,
        /// Empty for non-generic functions. Generic functions are only templates,
        /// sema creates a non-generic instance for every set of type arguments.
        type_params: Vec<TypeParam>,
        params: Vec<Param>,
        body: Option<Vec<Stmt>>,
        return_type: Option<Type>,
        parent: Option<String>,
        mangled: Option<String>,
        visibility: Visibility,
        /// Set for `extern func`, a C function the program calls.
        external: Option<Extern>,
    },
    Trait {
        name: String,
        methods: Vec<Declaration>
    },
    /// `impl trait_name for target { methods }`.
    ///
    /// Sema copies the trait's default methods that aren't overridden into
    /// `methods`, so afterwards it holds every method of the trait.
    Impl {
        trait_name: String,
        target: Type,
        methods: Vec<Declaration>,
    },
    Extern(ExternItem),
}

/// A declaration built by [`Declaration::new`], with the [`Span`] where it occurred.
#[derive(Clone)]
pub struct Declaration {
    pub kind: DeclarationKind,
    pub span: Span,
    pub visibility: Visibility,
}

impl Declaration {
    pub fn new(kind: DeclarationKind, span: Span, visibility: Visibility) -> Self {
        Self { kind, span, visibility }
    }
}


impl Declare for Declaration {
    fn declare<'ctx>(&self, ctx: &mut CodegenContext<'ctx>) {
        match &self.kind {
            // Generic classes and enums are templates, sema hands their instances to codegen instead
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => {}
            DeclarationKind::Class { name, fields, methods, .. } => {
                let named_fields: Vec<(String, _)> = fields.iter()
                    .map(|p| (p.name.clone(), p.ty.to_llvm(ctx)))
                    .collect();
                let field_types: Vec<_> = named_fields.iter().map(|(_, t)| *t).collect();
                ctx.struct_type(name).set_body(&field_types, false);
                ctx.push_class(name.to_string(), named_fields);
                ctx.class_field_types.insert(name.to_string(), fields.iter().map(|f| f.ty.clone()).collect());

                ctx.current_class = Some(name.clone());
                for method in methods {
                    method.declare(ctx);
                }
                ctx.current_class = None;
            }
            DeclarationKind::Enum { name, fields, methods, .. } => {
                ctx.current_class = Some(name.clone());
                for method in methods {
                    method.declare(ctx);
                }
                ctx.current_class = None;

                // A variant without payload stores nothing, a byte stands in for its type
                let named_fields: Vec<(String, _)> = fields.iter()
                    .map(|p| (p.0.clone(), if p.2 == Type::Void { ctx.context.i8_type().into() } else { p.2.to_llvm(ctx) }))
                    .collect();

                let target_data = ctx.machine.get_target_data();
                let max_payload = named_fields.iter()
                    .map(|(_, t)| target_data.get_store_size(t))
                    .max()
                    .unwrap_or(0);

                // The payload is stored in 8 byte words, so pointers and int64s in it are aligned
                let i32_ty = ctx.context.i32_type();
                let payload_ty = ctx.context.i64_type().array_type(max_payload.div_ceil(8) as u32);
                ctx.struct_type(name)
                    .set_body(&[i32_ty.into(), payload_ty.into()], false);
                ctx.enum_variant_types.insert(name.to_string(), fields.iter().map(|f| f.2.clone()).collect());

                for (variant, _, payload_ty, ty_span) in fields.iter() {
                    let mangled = format!("{}__{}", name, variant);
                    let (param_types, params) = if *payload_ty == Type::Void {
                        (vec![], vec![])
                    } else {
                        (vec![payload_ty.to_llvm(ctx).into()], vec![Param {
                            name: "payload".to_string(),
                            ty: payload_ty.clone(),
                            ty_span: ty_span.clone(),
                            value: None,
                            mutable: false,
                        }])
                    };
                    let fn_type = Type::Enum(name.to_string()).to_fn_type(&param_types, ctx);
                    ctx.module.add_function(&function_symbol(&mangled), fn_type, None);
                    ctx.push_func(mangled, params);
                }

                ctx.push_enum(name.to_string(), named_fields);
            }
            // Generic functions are templates, sema hands their instances to codegen instead
            DeclarationKind::Function { type_params, .. } if !type_params.is_empty() => {}
            DeclarationKind::Function { params, return_type, mangled, external: Some(external), .. } => {
                let mangled = mangled.as_ref()
                    .expect("function mangled name missing — sema must run first");
                // Strings, slices and `&Buffer`s are passed as addresses
                let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
                let param_types: Vec<BasicMetadataTypeEnum> = params.iter()
                    .map(|p| if p.ty.is_string() || matches!(p.ty, Type::Slice(_)) { ptr_ty.into() } else { p.ty.to_llvm(ctx).into() })
                    .collect();
                let fn_type = match return_type {
                    // A C string, copied into a String after the call
                    Some(ty) if ty.is_string() => ptr_ty.fn_type(&param_types, external.variadic),
                    Some(ty) => ty.to_llvm(ctx).fn_type(&param_types, external.variadic),
                    None => ctx.context.void_type().fn_type(&param_types, external.variadic),
                };
                // Calls use `fn_type`, the C function may be declared with other types already
                let function = ctx.module.get_function(&external.symbol)
                    .unwrap_or_else(|| ctx.module.add_function(&external.symbol, fn_type, Some(Linkage::External)));
                ctx.externs.insert(mangled.clone(), (function, fn_type));
                ctx.push_func(mangled.clone(), params.to_vec());
            }
            DeclarationKind::Function { params, return_type, mangled, .. } => {
                let mangled = mangled.as_ref()
                    .expect("function mangled name missing — sema must run first");

                let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
                param_types.extend(params.iter().map(|p| -> BasicMetadataTypeEnum {
                    if p.name == "self" {
                        ctx.context.ptr_type(AddressSpace::default()).into()
                    } else {
                        p.ty.to_llvm(ctx).into()
                    }
                }));

                let fn_type = match return_type {
                    Some(ty) => ty.to_fn_type(&param_types, ctx),
                    None     => ctx.context.void_type().fn_type(&param_types, false),
                };
                ctx.module.add_function(&function_symbol(mangled), fn_type, None);
                ctx.push_func(mangled.clone(), params.to_vec());
            },
            // Traits only describe methods, there's nothing to generate
            DeclarationKind::Trait { .. } | DeclarationKind::Extern(ExternItem::Link(_)) => {}
            DeclarationKind::Impl { target, methods, .. } => {
                // Impl methods are methods of the target, so `self` refers to it
                ctx.current_class = target.owner_name();
                for method in methods {
                    method.declare(ctx);
                }
                ctx.current_class = None;
            }
            DeclarationKind::Extern(ExternItem::Type { name, methods, .. }) => {
                ctx.current_class = Some(name.clone());
                for method in methods {
                    method.declare(ctx);
                }
                ctx.current_class = None;
            }
        }
    }
}

impl Display for DeclarationKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            DeclarationKind::Class { name, .. }    => write!(f, "Class Declaration: {}", name),
            DeclarationKind::Enum { name, .. }     => write!(f, "Emum Declaration: {}", name),
            DeclarationKind::Function { name, .. } => write!(f, "Function Declaration: func {}", name),
            DeclarationKind::Trait { name, .. } => write!(f, "Trait Declaration: {}", name),
            DeclarationKind::Impl { trait_name, target, .. } => write!(f, "Impl Declaration: {} for {}", trait_name, target),
            DeclarationKind::Extern(ExternItem::Type { name, .. }) => write!(f, "Extern Type Declaration: {}", name),
            DeclarationKind::Extern(ExternItem::Link(library)) => write!(f, "Extern Link: {}", library),
        }
    }
}

impl Codegen for Declaration {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        match &self.kind {
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => None,
            DeclarationKind::Class { name, fields, methods, .. } => {
                ctx.current_class = Some(name.clone());
                ctx.current_class_fields = Some(fields.clone());
                for method in methods {
                    method.codegen(ctx);
                }
                ctx.current_class = None;
                ctx.current_class_fields = None;
                None
            }
            DeclarationKind::Trait { .. } | DeclarationKind::Extern(ExternItem::Link(_)) => None,
            DeclarationKind::Extern(ExternItem::Type { name, methods, .. }) => {
                ctx.current_class = Some(name.clone());
                for method in methods {
                    method.codegen(ctx);
                }
                ctx.current_class = None;
                None
            }
            DeclarationKind::Impl { target, methods, .. } => {
                ctx.current_class = target.owner_name();
                for method in methods {
                    method.codegen(ctx);
                }
                ctx.current_class = None;
                None
            }
            DeclarationKind::Enum { name, fields, methods, .. } => {
                ctx.current_class = Some(name.clone());
                for method in methods {
                    method.codegen(ctx);
                }
                ctx.current_class = None;

                let prev_block = ctx.builder.get_insert_block();
                let struct_ty = ctx.module.get_struct_type(name)
                    .expect("enum struct not defined — declare pass must run first");
                let i32_ty = ctx.context.i32_type();

                for (i, (variant, _, _, _)) in fields.iter().enumerate() {
                    let mangled = format!("{}__{}", name, variant);
                    let function = ctx.module.get_function(&function_symbol(&mangled))
                        .expect("enum constructor missing — declare pass must run first");

                    let bb = ctx.context.append_basic_block(function, "entry");
                    ctx.builder.position_at_end(bb);

                    // Zeroed, so the unused payload bytes are always the same
                    let local = ctx.builder.build_alloca(struct_ty, "enum_val").unwrap();
                    ctx.builder.build_store(local, struct_ty.const_zero()).unwrap();

                    let tag_ptr = ctx.builder
                        .build_struct_gep(struct_ty, local, 0, "tag_ptr")
                        .unwrap();
                    let tag_val = i32_ty.const_int(i as u64, false);
                    ctx.builder.build_store(tag_ptr, tag_val).unwrap();

                    if let Some(payload) = function.get_nth_param(0) {
                        let payload_ptr = ctx.builder
                            .build_struct_gep(struct_ty, local, 1, "payload_ptr")
                            .unwrap();
                        ctx.builder.build_store(payload_ptr, payload).unwrap();
                    }

                    let loaded = ctx.builder.build_load(struct_ty, local, "ret_val").unwrap();
                    ctx.builder.build_return(Some(&loaded)).unwrap();
                }

                if let Some(prev) = prev_block {
                    ctx.builder.position_at_end(prev);
                }
                None
            }
            DeclarationKind::Function { type_params, .. } if !type_params.is_empty() => None,
            // The C library has the body
            DeclarationKind::Function { external: Some(_), .. } => None,
            DeclarationKind::Function { params, body, return_type, mangled, .. } => {
                let mangled = mangled.as_ref()
                    .expect("function mangled name missing — sema must run first");
                let function = ctx.module.get_function(&function_symbol(mangled))
                    .expect("function signature missing — declare pass must run first");

                let prev_block = ctx.builder.get_insert_block()?;
                // Params and locals of this function must not leak into the outer scope
                let prev_scope = ctx.scope.clone();
                // The function owns its own `Unique`s, the surrounding code's are freed elsewhere
                let prev_drops = std::mem::replace(&mut ctx.drop_scopes, vec![Vec::new()]);
                let prev_loops = std::mem::take(&mut ctx.loops);
                // So does every temporary a `return` frees
                let prev_temporaries = std::mem::take(&mut ctx.temporaries);
                let bb = ctx.context.append_basic_block(function, "entry");
                ctx.builder.position_at_end(bb);

                let self_class_name = ctx.current_class.clone();
                let is_ctor = self_class_name.as_ref().map_or(false, |cn| cn == mangled);


                let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
                let self_ptr = ctx.builder.build_alloca(ptr_ty, "self").unwrap();

                if is_ctor {
                    let class_name = self_class_name.clone().unwrap();
                    let struct_ty = ctx.module.get_struct_type(&class_name).unwrap();
                    let self_local = ctx.builder.build_alloca(struct_ty, "self_local").unwrap();
                    ctx.builder.build_store(self_ptr, self_local).unwrap();
                    ctx.push_var("self".to_string(), ptr_ty.into(), self_ptr, Some(class_name));
                } 

                for (i, param) in params.iter().enumerate() {
                    if ctx.func_has_self(mangled) && i == 0 {
                        let self_val = function.get_nth_param(0)?;
                        // `self` is passed as a pointer to the receiver. For anything but
                        // classes that pointer is used as the variable's storage, so `self`
                        // reads and writes the caller's value directly
                        if !matches!(param.ty, Type::Class(_)) {
                            let ty = param.ty.to_llvm(ctx);
                            ctx.push_var("self".to_string(), ty, self_val.into_pointer_value(), param.ty.class_name());
                            continue;
                        }
                        ctx.builder.build_store(self_ptr, self_val).unwrap();
                        // Instances of generic methods are generated outside their class,
                        // so the class comes from the parameter's type
                        let class_name = param.ty.class_name().or_else(|| self_class_name.clone());
                        ctx.push_var("self".to_string(), ptr_ty.into(), self_ptr, class_name);
                        continue;
                    }

                    let ty = param.ty.to_llvm(ctx);
                    let value = function.get_nth_param(i as u32)?;
                    let ptr = ctx.builder.build_alloca(ty, &param.name).unwrap();
                    ctx.builder.build_store(ptr, value).unwrap();
                    // Owned memory passed in, moved or copied, belongs to the function from now on
                    if ctx.owns_memory(&param.ty) {
                        ctx.push_owned_var(param.name.to_string(), ptr, param.ty.clone());
                        continue;
                    }
                    ctx.push_var(param.name.to_string(), ty, ptr, param.ty.class_name());
                }

                if is_ctor {
                    let class_name = self_class_name.clone().unwrap();
                    let struct_ty = ctx.module.get_struct_type(&class_name).unwrap();
                    let self_ptr = ctx.load_var("self").unwrap().into_pointer_value();
                    let field_defs = ctx.current_class_fields.clone().unwrap();
                    for (i, field) in field_defs.iter().enumerate() {
                        if let Some(default_expr) = &field.value {
                            let val = *default_expr.codegen(ctx).unwrap();
                            let field_ptr = ctx.builder.
                                build_struct_gep(struct_ty, self_ptr, i as u32, "tmp_field").
                                unwrap();
                            ctx.builder.build_store(field_ptr, val).unwrap();
                        }
                    }
                }

                match body {
                    Some(stmts) => for stmt in stmts {
                        stmt.codegen(ctx);
                    }
                    None => ()
                }

                if ctx.builder.get_insert_block()
                    .and_then(|b| b.get_terminator())
                    .is_none()
                {
                    ctx.emit_all_drops();
                    if is_ctor {
                        let class_name = self_class_name.clone().unwrap();
                        let struct_ty = ctx.module.get_struct_type(&class_name).unwrap();
                        let self_ptr = ctx.load_var("self").unwrap().into_pointer_value();
                        let loaded = ctx.builder.build_load(struct_ty, self_ptr, "ret_val").unwrap();
                        ctx.builder.build_return(Some(&loaded)).unwrap();
                    } else if return_type.is_none() {
                        ctx.builder.build_return(None).unwrap();
                    } else {
                        // Reached after an `if` whose branches all return, but nothing else
                        ctx.builder.build_unreachable().unwrap();
                    }
                }

                ctx.builder.position_at_end(prev_block);
                ctx.scope = prev_scope;
                ctx.drop_scopes = prev_drops;
                ctx.loops = prev_loops;
                ctx.temporaries = prev_temporaries;
                None
            },
        }

    }
}
