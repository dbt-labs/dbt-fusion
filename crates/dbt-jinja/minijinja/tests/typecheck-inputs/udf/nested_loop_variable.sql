{%- macro my_macro(matrix) -%}
{#-- funcsign: (list[list[integer]]) -> string --#}
{% for row in matrix %}{% for cell in row %}{{ cell + 1 }}{% endfor %}{% endfor %}
{%- endmacro -%}