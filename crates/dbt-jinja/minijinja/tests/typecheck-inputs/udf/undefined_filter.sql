{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ a | not_a_filter }}
{%- endmacro -%}