{%- macro my_macro(a) -%}
{#-- funcsign: (string | integer) -> string --#}
{% if a is string %}{{ a.upper() }}{% endif %}
{%- endmacro -%}