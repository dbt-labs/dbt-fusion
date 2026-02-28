{%- macro my_macro(a) -%}
{#-- funcsign: (optional[string]) -> string --#}
{% if a is not none %}{{ a.upper() }}{% endif %}
{%- endmacro -%}