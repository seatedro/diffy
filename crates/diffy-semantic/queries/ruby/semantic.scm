["(" ")" "[" "]" "{" "}"] @delimiter

[(string_content) (character)] @text

(class name: _ @item.name) @item.class
(module name: _ @item.name) @item.module
(method name: _ @item.name) @item.def
(singleton_method name: _ @item.name) @item.def
