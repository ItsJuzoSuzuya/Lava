use inkwell::{types::BasicTypeEnum, values::PointerValue};

/// A variable as codegen sees it.
///
/// Every variable lives in its own stack slot (an `alloca`). The symbol holds
/// the pointer to that slot plus what's needed to load or store the value
/// through it. Codegen keeps symbols by name in
/// [`CodegenContext::scope`](crate::codegen_ctx::CodegenContext::scope).
#[derive(Clone)]
pub struct Symbol<'ctx> {
    /// LLVM type of the value stored in the slot. For `self` and references
    /// this is a pointer type, and the object itself lives elsewhere.
    pub ty: BasicTypeEnum<'ctx>,
    /// The stack slot holding the value.
    pub ptr: PointerValue<'ctx>,
    /// Name of the class or enum the value is an instance of, `None` for
    /// everything else.
    ///
    /// Field access needs it to find the struct layout, because LLVM pointers
    /// don't know what they point to.
    pub class_name: Option<String>,
}
