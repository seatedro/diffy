["(" ")" "[" "]" "{" "}"] @delimiter

[(string) (character_literal)] @text

(package_object name: _ @item.name) @item.package
(class_definition name: _ @item.name) @item.class
(object_definition name: _ @item.name) @item.object
(trait_definition name: _ @item.name) @item.trait
(enum_definition name: _ @item.name) @item.enum
(type_definition name: _ @item.name) @item.type
(given_definition name: _ @item.name) @item.given
(function_definition name: _ @item.name) @item.def
(function_declaration name: _ @item.name) @item.def
