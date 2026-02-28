{%- macro my_macro(items) -%}
{#-- funcsign: (list[integer]) -> string --#}
{{ items["a":"b"] }}
{%- endmacro -%}