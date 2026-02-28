{%- macro my_macro(cond) -%}
{#-- funcsign: (boolean) -> string --#}
{% if cond %}{% set x = 1 %}{% else %}nope{% endif %}
{{ x }}
{%- endmacro -%}