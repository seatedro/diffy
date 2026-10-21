["(" ")" "[" "]" "{" "}"] @delimiter

(string_content) @text

(decorated_definition) @item.unit
(decorator) @item.lead

(function_definition name: _ @item.name) @item.def
(class_definition name: _ @item.name) @item.class
