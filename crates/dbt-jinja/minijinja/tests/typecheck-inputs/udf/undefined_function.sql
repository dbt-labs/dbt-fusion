{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ not_a_function() }}
{%- endmacro -%}