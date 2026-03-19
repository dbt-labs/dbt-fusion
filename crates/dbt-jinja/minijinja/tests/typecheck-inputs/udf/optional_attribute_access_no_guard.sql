{%- macro my_macro(a) -%}
{#-- funcsign: (optional[string]) -> string --#}
{{ a.upper() }}
{%- endmacro -%}