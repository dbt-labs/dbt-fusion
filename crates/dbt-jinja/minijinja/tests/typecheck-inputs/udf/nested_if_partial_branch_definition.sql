{%- macro my_macro(a, b) -%}
{#-- funcsign: (boolean, boolean) -> string --#}
{% if a %}
  {% if b %}{% set x = 1 %}{% else %}nope{% endif %}
{% else %}
  {% set x = 3 %}
{% endif %}
{{ x }}
{%- endmacro -%}