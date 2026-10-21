["(" ")" "[" "]" "{" "}" "<" ">"] @delimiter

[(string_literal) (character_literal)] @text

(class_declaration name: _ @item.name) @item.class
(interface_declaration name: _ @item.name) @item.interface
(enum_declaration name: _ @item.name) @item.enum
(record_declaration name: _ @item.name) @item.record
(annotation_type_declaration name: _ @item.name) @item.annotation
(method_declaration name: _ @item.name) @item.method
(constructor_declaration name: _ @item.name) @item.constructor
