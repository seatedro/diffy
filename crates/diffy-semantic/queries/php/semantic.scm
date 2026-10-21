["(" ")" "[" "]" "{" "}"] @delimiter

[(string) (string_content)] @text

(namespace_definition name: _ @item.name) @item.namespace
(class_declaration name: _ @item.name) @item.class
(interface_declaration name: _ @item.name) @item.interface
(trait_declaration name: _ @item.name) @item.trait
(enum_declaration name: _ @item.name) @item.enum
(function_definition name: _ @item.name) @item.function
(method_declaration name: _ @item.name) @item.method
