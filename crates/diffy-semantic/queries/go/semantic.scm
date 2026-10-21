["(" ")" "[" "]" "{" "}"] @delimiter

[(interpreted_string_literal) (raw_string_literal)] @text

(type_spec name: _ @item.name) @item.type
(function_declaration name: _ @item.name) @item.function
(method_declaration name: _ @item.name) @item.method
