["(" ")" "[" "]" "{" "}" "<" ">"] @delimiter

[(string_content) (character_literal)] @text

(class_declaration (type_identifier) @item.name) @item.class
(object_declaration (type_identifier) @item.name) @item.object
(type_alias (type_identifier) @item.name) @item.typealias
(function_declaration (simple_identifier) @item.name) @item.fun
