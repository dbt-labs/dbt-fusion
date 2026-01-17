{%- materialization test, adapter='snowflake' -%}

    {% set original_warehouse = set_warehouse() %}
    {% set original_query_tag = set_query_tag() %}
    {% set relations = materialization_test_default() %}
    {% do unset_query_tag(original_query_tag) %}
    {% do restore_warehouse(original_warehouse) %}
    {{ return(relations) }}

{%- endmaterialization -%}

