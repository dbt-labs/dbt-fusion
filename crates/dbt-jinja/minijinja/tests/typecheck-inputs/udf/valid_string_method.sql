{%- macro my_macro(a) -%}
{#-- funcsign: (string) -> string --#}
{{ a.upper() }}
{%- endmacro -%}