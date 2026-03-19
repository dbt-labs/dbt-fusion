{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ a.nonexistent() }}
{%- endmacro -%}