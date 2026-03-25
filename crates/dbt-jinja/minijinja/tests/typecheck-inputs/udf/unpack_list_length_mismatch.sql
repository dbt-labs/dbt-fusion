{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{% set x, y = [1, 2, 3] %}
{{ x }}{{ y }}
{%- endmacro -%}