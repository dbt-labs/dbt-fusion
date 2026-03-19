{%- macro my_macro(a, b) -%}
{#-- funcsign: (string, integer) -> string --#}
{% if a > b %}yes{% endif %}
{%- endmacro -%}