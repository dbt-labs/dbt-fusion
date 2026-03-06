{%- macro my_macro(a) -%}
{#-- funcsign: (optional[integer]) -> string --#}
{% if a is not none %}
  {{ a + 1 }}
{% else %}
  {{ a }}
{% endif %}
{%- endmacro -%}