["(" ")" "[" "]" "{" "}" "<" ">" "|"] @delimiter

[(char_literal) (string_literal) (raw_string_literal)] @text

(attribute_item) @item.lead

(function_item name: _ @item.name) @item.fn
(function_signature_item name: _ @item.name) @item.fn
(struct_item name: _ @item.name) @item.struct
(enum_item name: _ @item.name) @item.enum
(union_item name: _ @item.name) @item.union
(trait_item name: _ @item.name) @item.trait
(impl_item trait: _? @item.key type: _ @item.name) @item.impl
(mod_item name: _ @item.name) @item.mod
(const_item name: _ @item.name) @item.const
(static_item name: _ @item.name) @item.static
(type_item name: _ @item.name) @item.type
(macro_definition name: _ @item.name) @item.macro
