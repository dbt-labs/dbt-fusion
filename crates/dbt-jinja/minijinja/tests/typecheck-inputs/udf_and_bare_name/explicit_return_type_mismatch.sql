{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> integer --#}
{{ return("hello") }}
{%- endmacro -%}