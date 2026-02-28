{%- macro my_macro(cond) -%}
{#-- funcsign: (boolean) -> string --#}
{% if cond %}{% set x = 1 %}{% else %}{% set x = 2 %}{% endif %}
{{ x }}
{%- endmacro -%}