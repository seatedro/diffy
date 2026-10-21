["(" ")" "[" "]" "{" "}" "<" ">"] @delimiter

[(string) (string_fragment) (regex)] @text

(decorator) @item.lead

(function_declaration name: _ @item.name) @item.function
(generator_function_declaration name: _ @item.name) @item.function
(method_definition name: _ @item.name) @item.method
(class_declaration name: _ @item.name) @item.class
