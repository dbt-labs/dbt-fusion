{%- macro my_macro(a, b) -%}
{#-- funcsign: (string, integer) -> string --#}
{{ a + b }}
{%- endmacro -%}