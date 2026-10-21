["(" ")" "[" "]" "{" "}" "<" ">"] @delimiter

[(line_str_text) (multi_line_str_text) (raw_str_part)] @text

(class_declaration declaration_kind: "class" name: _ @item.name) @item.class
(class_declaration declaration_kind: "struct" name: _ @item.name) @item.struct
(class_declaration declaration_kind: "enum" name: _ @item.name) @item.enum
(class_declaration declaration_kind: "actor" name: _ @item.name) @item.actor
(class_declaration declaration_kind: "extension" name: _ @item.name) @item.extension
(protocol_declaration name: _ @item.name) @item.protocol
(typealias_declaration name: _ @item.name) @item.typealias
(function_declaration name: _ @item.name) @item.func
(protocol_function_declaration name: _ @item.name) @item.func
(init_declaration name: _ @item.name) @item.init
