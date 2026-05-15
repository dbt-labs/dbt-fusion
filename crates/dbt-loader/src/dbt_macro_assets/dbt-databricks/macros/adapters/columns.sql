{% macro get_columns_comments(relation) -%}
  {{ return(run_query_as(get_columns_comments_sql(relation), 'get_columns_comments')) }}
{% endmacro %}

{% macro get_columns_comments_sql(relation) %}
DESCRIBE TABLE {{ relation.render() }}
{% endmacro %}

{% macro get_columns_comments_as_json(relation) -%}
  {{ return(run_query_as(get_columns_comments_as_json_sql(relation), 'get_columns_comments_as_json')) }}
{% endmacro %}

{% macro get_columns_comments_as_json_sql(relation) %}
  DESCRIBE TABLE EXTENDED {{ relation.render() }} AS JSON
{% endmacro %}

{#--
  dbt-fusion flattens Arrow struct columns in AgateTable using "/" as a separator
  (e.g. a column "my_struct: STRUCT<a, b>" becomes "my_struct/a" and "my_struct/b").
  When persist_docs.columns=true, get_columns_in_query is used to build the column
  list for CREATE OR REPLACE VIEW.  If we pass the expanded sub-field names to
  Databricks it raises CREATE_VIEW_COLUMN_ARITY_MISMATCH because the query only
  returns the top-level struct column.  This override strips sub-fields and returns
  only the unique root column names.
--#}
-- funcsign: (string) -> list[string]
{% macro databricks__get_columns_in_query(select_sql) %}
  {% call statement('get_columns_in_query', fetch_result=True, auto_begin=False) -%}
    {{ get_empty_subquery_sql(select_sql) }}
  {%- endcall %}
  {%- set raw_columns = load_result('get_columns_in_query').table.columns | map(attribute='name') | list -%}
  {%- set result = [] -%}
  {%- for col in raw_columns -%}
    {%- set root = col.split('/')[0] -%}
    {%- if root not in result -%}
      {%- do result.append(root) -%}
    {%- endif -%}
  {%- endfor -%}
  {{ return(result) }}
{% endmacro %}

{% macro databricks__alter_relation_add_remove_columns(relation, add_columns, remove_columns) %}
  {% if remove_columns %}
    {{ run_query_as(drop_columns_sql(relation, remove_columns), 'alter_relation_remove_columns', fetch_result=False) }}
  {% endif %}

  {% if add_columns %}
    {{ run_query_as(add_columns_sql(relation, add_columns), 'alter_relation_add_columns', fetch_result=False) }}
  {% endif %}
{% endmacro %}

{% macro drop_columns_sql(relation, remove_columns) %}
ALTER TABLE {{ relation.render() }} DROP COLUMNS ({{ api.Column.format_remove_column_list(remove_columns) }})
{% endmacro %}

{% macro add_columns_sql(relation, add_columns) %}
ALTER TABLE {{ relation.render() }} ADD COLUMNS ({{ api.Column.format_add_column_list(add_columns) }})
{% endmacro %}


{% macro databricks__alter_column_type(relation, column_name, new_column_type) -%}
  {% call statement('alter_column_type') %}
    alter table {{ relation }} alter column {{ adapter.quote(column_name) }} type {{ new_column_type }};
  {% endcall %}
{% endmacro %}
