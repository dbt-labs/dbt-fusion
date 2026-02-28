{%- macro my_macro(items) -%}
{#-- funcsign: (list[integer]) -> string --#}
{% for item in items %}{% set x = item %}{% endfor %}
{{ x }}
{%- endmacro -%}