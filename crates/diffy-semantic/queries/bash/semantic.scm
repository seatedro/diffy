["(" ")" "[" "]" "{" "}"] @delimiter

[(string) (raw_string) (heredoc_body) (simple_expansion)] @text

(function_definition name: _ @item.name) @item.def
