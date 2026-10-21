["(" ")" "[" "]" "{" "}"] @delimiter

[(string) (character)] @text

(procedure_declaration (identifier) @item.name) @item.proc
(overloaded_procedure_declaration (identifier) @item.name) @item.proc
(struct_declaration (identifier) @item.name) @item.struct
(enum_declaration (identifier) @item.name) @item.enum
(union_declaration (identifier) @item.name) @item.union
(bit_field_declaration (identifier) @item.name) @item.bit_field
