["(" ")" "[" "]" "{" "}" "<" ">"] @delimiter

[(string_literal) (verbatim_string_literal) (raw_string_literal) (character_literal) (string_content)] @text

(namespace_declaration name: _ @item.name) @item.namespace
(file_scoped_namespace_declaration name: _ @item.name) @item.namespace
(class_declaration name: _ @item.name) @item.class
(struct_declaration name: _ @item.name) @item.struct
(interface_declaration name: _ @item.name) @item.interface
(enum_declaration name: _ @item.name) @item.enum
(record_declaration name: _ @item.name) @item.record
(delegate_declaration name: _ @item.name) @item.delegate
(method_declaration name: _ @item.name) @item.method
(constructor_declaration name: _ @item.name) @item.constructor
(property_declaration name: _ @item.name) @item.property
