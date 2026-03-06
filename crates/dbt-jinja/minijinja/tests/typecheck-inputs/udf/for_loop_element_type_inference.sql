{%- macro my_macro(items) -%}
{#-- funcsign: (list[integer]) -> string --#}
{% for item in items %}{{ item + 1 }}{% endfor %}
{%- endmacro -%}