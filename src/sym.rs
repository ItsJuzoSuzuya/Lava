use inkwell::{types::BasicTypeEnum, values::PointerValue};

#[derive(Clone)]
pub struct Symbol<'ctx> {
    pub ty: BasicTypeEnum<'ctx>,
    pub ptr: PointerValue<'ctx>,
    pub mutable: bool,
    pub class_name: Option<String>
}
