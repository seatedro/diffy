["(" ")" "[" "]" "{" "}" "<" ">"] @delimiter

[(string) (string_fragment) (regex) (predefined_type)] @text

(decorator) @item.lead

(enum_declaration name: _ @item.name) @item.enum
(type_alias_declaration name: _ @item.name) @item.type
(function_declaration name: _ @item.name) @item.function
(generator_function_declaration name: _ @item.name) @item.function
(method_definition name: _ @item.name) @item.method
(class_declaration name: _ @item.name) @item.class
(abstract_class_declaration name: _ @item.name) @item.class
(interface_declaration name: _ @item.name) @item.interface
