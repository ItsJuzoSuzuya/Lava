use inkwell::{types::BasicTypeEnum, values::PointerValue};

use crate::r#type::Type;

/// A variable as codegen sees it.
///
/// Every variable lives in its own stack slot (an `alloca`). The symbol holds
/// the pointer to that slot plus what's needed to load or store the value
/// through it. Codegen keeps symbols by name in
/// [`CodegenContext::scope`](crate::codegen_ctx::CodegenContext::scope).
#[derive(Clone)]
pub struct Symbol<'ctx> {
    /// LLVM type of the value stored in the slot. For `self`, references and
    /// `Unique`s this is a pointer type, and the object itself lives elsewhere.
    pub ty: BasicTypeEnum<'ctx>,
    /// The stack slot holding the value.
    pub ptr: PointerValue<'ctx>,
    /// Name of the class or enum the value is an instance of, `None` for
    /// everything else.
    ///
    /// Field access needs it to find the struct layout, because LLVM pointers
    /// don't know what they point to.
    pub class_name: Option<String>,
    /// Set if the variable owns heap memory, a `Unique` or `Buffer`, which has
    /// to be freed when the variable goes out of scope.
    pub owned: Option<OwnedSlot<'ctx>>,
}

/// A variable owning heap memory, freed at the end of its scope unless it got moved.
#[derive(Clone)]
pub struct OwnedSlot<'ctx> {
    /// The variable's stack slot, holding the pointer to the heap memory.
    pub slot: PointerValue<'ctx>,
    /// An `i1` on the stack that is true while the variable still owns its value.
    /// Moving the value out clears it, so it doesn't get freed twice. A runtime
    /// flag, because whether a value got moved can depend on a match arm.
    pub alive: PointerValue<'ctx>,
    /// Type of the owned value, like `Unique<Point>` or `Buffer<int32>`.
    pub ty: Type,
}
