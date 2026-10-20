["(" ")" "[" "]" "{" "}"] @delimiter

[(string_literal) (char_literal)] @text

(struct_specifier name: _ @item.name body: _) @item.struct
(enum_specifier name: _ @item.name body: _) @item.enum
(function_definition declarator: [(identifier) (field_identifier) (statement_identifier) (type_identifier)] @item.name) @item.fn
(function_definition declarator: (_ declarator: [(identifier) (field_identifier) (statement_identifier) (type_identifier)] @item.name)) @item.fn
(function_definition declarator: (_ declarator: (_ declarator: [(identifier) (field_identifier) (statement_identifier) (type_identifier)] @item.name))) @item.fn
(function_definition declarator: (_ declarator: (_ declarator: (_ declarator: [(identifier) (field_identifier) (statement_identifier) (type_identifier)] @item.name)))) @item.fn
(function_definition declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: [(identifier) (field_identifier) (statement_identifier) (type_identifier)] @item.name))))) @item.fn
(function_definition declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: [(identifier) (field_identifier) (statement_identifier) (type_identifier)] @item.name)))))) @item.fn
(function_definition declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: [(identifier) (field_identifier) (statement_identifier) (type_identifier)] @item.name))))))) @item.fn
(function_definition declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: (_ declarator: [(identifier) (field_identifier) (statement_identifier) (type_identifier)] @item.name)))))))) @item.fn
