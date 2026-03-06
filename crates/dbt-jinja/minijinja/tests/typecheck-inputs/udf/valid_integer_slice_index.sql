{%- macro my_macro(items) -%}
{#-- funcsign: (list[integer]) -> string --#}
{{ items[0:2] }}
{%- endmacro -%}