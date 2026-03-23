use qmetaobject::*;

#[derive(Clone, Default, SimpleListItem)]
pub struct DiffRowItem {
    pub row_type: QString,
    pub file_index: i32,
    pub hunk_index: i32,
    pub line_index: i32,
    pub old_line_number: i32,
    pub new_line_number: i32,
    pub text: QString,
}
