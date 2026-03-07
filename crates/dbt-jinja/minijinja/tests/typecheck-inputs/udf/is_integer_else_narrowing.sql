{%- macro my_macro(a) -%}
{#-- funcsign: (string | integer) -> string --#}
{% if a is integer %}{{ a + 1 }}{% else %}{{ a.upper() }}{% endif %}
{%- endmacro -%}