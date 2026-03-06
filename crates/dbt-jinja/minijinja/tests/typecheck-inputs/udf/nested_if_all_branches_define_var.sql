{%- macro my_macro(a, b) -%}
{#-- funcsign: (boolean, boolean) -> string --#}
{% if a %}
  {% if b %}{% set x = 1 %}{% else %}{% set x = 2 %}{% endif %}
{% else %}
  {% set x = 3 %}
{% endif %}
{{ x }}
{%- endmacro -%}