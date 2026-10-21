["(" ")" "[" "]" "{" "}"] @delimiter

[(string) (multiline_string) (character)] @text

(function_declaration name: _ @item.name) @item.fn
(variable_declaration (identifier) @item.name (struct_declaration)) @item.struct
(variable_declaration (identifier) @item.name (enum_declaration)) @item.enum
(variable_declaration (identifier) @item.name (union_declaration)) @item.union
(test_declaration [(string) (identifier)] @item.name) @item.test
