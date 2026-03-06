{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ return("hello") }}
{%- endmacro -%}