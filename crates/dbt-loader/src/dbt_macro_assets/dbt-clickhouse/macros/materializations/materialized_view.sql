{#-
  Create or update a materialized view in ClickHouse.
  The dbt model relation is the destination table. Fusion/dbt creates and names
  the physical ClickHouse materialized view that writes into that table.
-#}
{%- materialization materialized_view, adapter='clickhouse' -%}

  {%- set result = clickhouse__materialized_view_standard(sql) -%}
    {{ return(result) }}
{%- endmaterialization -%}

{% macro materialization_target_table(target_relation) %}
  {% do exceptions.raise_compiler_error(
    "materialization_target_table() is not supported by Fusion ClickHouse materialized_view models. "
    ~ "Name the dbt model as the destination table; Fusion will generate the physical materialized view name."
  ) %}
{% endmacro %}


{#-
  Implicit target mode: creates the destination table represented by this dbt
  model plus the materialized view object(s) that write into it.
-#}
{% macro clickhouse__materialized_view_standard(sql) %}
  {# ClickHouse is a two-part namespace; upstream normalizes database to ''. #}
  {%- set target_relation = api.Relation.create(
      database='',
      schema=this.schema,
      identifier=this.identifier,
      type='table'
  ) -%}
  {%- set mv_relation = clickhouse__make_mv_relation(target_relation, '_mv') -%}
  {%- set configured_target_table = config.get('target_table', none) -%}

  {% if configured_target_table is not none %}
    {{ exceptions.raise_compiler_error(
      "The target_table config is not supported by Fusion ClickHouse materialized_view models. "
      ~ "Name the dbt model as the destination table; Fusion will generate the physical materialized view name."
    ) }}
  {% endif %}

  {%- set target_table_markers = modules.re.findall('(?i)--\\s*materialization_target_table\\s*:', sql) -%}
  {% if target_table_markers %}
    {{ exceptions.raise_compiler_error(
      "materialization_target_table markers are not supported by Fusion ClickHouse materialized_view models. "
      ~ "Name the dbt model as the destination table; Fusion will generate the physical materialized view name."
    ) }}
  {% endif %}

  {%- set named_mv_sections = modules.re.findall('--\\s*[^:\\n]+:begin', sql) -%}
  {% if named_mv_sections %}
    {{ exceptions.raise_compiler_error(
      "ClickHouse materialized_view models in Fusion create one generated materialized view named "
      ~ mv_relation.identifier
      ~ ". Named MV sections like --name:begin/--name:end are not supported."
    ) }}
  {% endif %}

  {%- set cluster_clause = on_cluster_clause(target_relation) -%}
  {%- set refreshable_clause = refreshable_mv_clause() -%}
  {%- set catchup_data = config.get('catchup', True) -%}

  {# look for an existing relation for the target table #}
  {%- set existing_relation = load_cached_relation(target_relation) -%}

  {% set grant_config = config.get('grants') %}
  {% set did_build = False %}
  {% set backup_relation = none %}

  {{ run_hooks(pre_hooks, inside_transaction=False) }}

  -- `BEGIN` happens here:
  {{ run_hooks(pre_hooks, inside_transaction=True) }}

  {# Fusion/dbt owns the physical MV name; users name only the target table. #}
  {% set views = {"mv": sql} %}

  {% if existing_relation is none %}
    {% do clickhouse__assert_generated_mv_names_available(target_relation, views) %}
    {{ log('Creating new materialized view ' + target_relation.name )}}
    {{ clickhouse__get_create_materialized_view_as_sql(target_relation, sql, views, catchup_data) }}
    {% set did_build = True %}
  {% elif not existing_relation.is_table %}
    {% do clickhouse__assert_generated_mv_names_available(target_relation, views) %}
    {{ log('Replacing existing non-table relation ' + existing_relation.name + ' with ClickHouse materialized_view target table ' + target_relation.name) }}
    {{ drop_relation_if_exists(existing_relation) }}
    {{ clickhouse__get_create_materialized_view_as_sql(target_relation, sql, views, catchup_data) }}
    {% set did_build = True %}
  {% else %}
    {% set current_config = adapter.describe_clickhouse_materialized_view(target_relation) %}
    {% do clickhouse__assert_generated_mv_relations_owned_by_target(target_relation, current_config, views) %}
    {% set desired_config = clickhouse__materialized_view_desired_config(mv_relation, sql, refreshable_clause) %}
    {% set current_config = {
      'target_table': current_config.target_table,
      'target_table_columns': adapter.get_columns_in_relation(target_relation),
      'materialized_views': current_config.materialized_views
    } %}
    {% set configuration_changes = target_relation.clickhouse_materialized_view_config_changeset(current_config, desired_config) %}
    {% set query_changes = {
      'has_changes': false,
      'requires_full_refresh': false,
      'changed': [],
      'missing': [],
      'recreate': []
    } %}
    {% set table_config_changes = {
      'has_changes': false,
      'requires_full_refresh': false,
      'changes': []
    } %}
    {% set table_schema_changes = {
      'has_changes': false,
      'requires_full_refresh': false,
      'changes': []
    } %}
    {% if configuration_changes is not none %}
      {% set query_changes = configuration_changes.query_changes %}
      {% set table_config_changes = configuration_changes.target_table_config_changes %}
      {% set table_schema_changes = configuration_changes.target_table_schema_changes %}
    {% endif %}
    {% set has_configuration_changes = configuration_changes is not none and configuration_changes.has_changes %}
    {% set on_configuration_change = config.get('on_configuration_change', 'apply') %}

    {% if should_full_refresh() %}
      {% set replace_target_table = True %}
    {% elif has_configuration_changes and on_configuration_change == 'fail' %}
      {% set table_change_names = table_config_changes.changes | join(',') %}
      {% set table_schema_change_names = table_schema_changes.changes | join(',') %}
      {% set changed_mv_names = query_changes.changed | join(',') %}
      {% set missing_mv_names = query_changes.missing | join(',') %}
      {% set recreated_mv_names = query_changes.recreate | join(',') %}
      {{ exceptions.raise_compiler_error(
        'Configuration changes detected for ClickHouse materialized_view "' ~ target_relation.name
        ~ '": target table changes=' ~ table_change_names
        ~ ', target table schema changes=' ~ table_schema_change_names
        ~ ', materialized view query changes=' ~ changed_mv_names
        ~ ', missing materialized views=' ~ missing_mv_names
        ~ ', materialized views requiring recreation=' ~ recreated_mv_names
      ) }}
    {% elif has_configuration_changes and on_configuration_change == 'continue' %}
      {{ log('Configuration changes detected for ClickHouse materialized_view ' ~ target_relation.name ~ ', but on_configuration_change=continue. Skipping changes.', info=True) }}
      {% do clickhouse__materialized_view_execute_no_op(target_relation) %}
      {% set replace_target_table = False %}
    {% elif has_configuration_changes and on_configuration_change == 'apply' %}
      {% set replace_target_table = table_config_changes.requires_full_refresh or table_schema_changes.requires_full_refresh %}
    {% elif has_configuration_changes %}
      {{ exceptions.raise_compiler_error("Unexpected on_configuration_change value: `" ~ on_configuration_change ~ "`") }}
    {% else %}
      {% do clickhouse__materialized_view_execute_no_op(target_relation) %}
      {% set replace_target_table = False %}
    {% endif %}

    {% if (has_configuration_changes and on_configuration_change == 'apply') or should_full_refresh() %}
      {% if replace_target_table %}
        {{ log('Replacing ClickHouse materialized view target table ' + target_relation.name) }}
        {% set backup_relation_type = existing_relation.type %}
        {% set backup_relation = clickhouse__normalize_relation_database(make_backup_relation(target_relation, backup_relation_type)) %}
        {% set preexisting_backup_relation = load_cached_relation(backup_relation) %}
        {% set intermediate_relation = none %}
        {% set preexisting_intermediate_relation = none %}
        {% if not existing_relation.can_exchange %}
          {% set intermediate_relation = clickhouse__normalize_relation_database(make_intermediate_relation(target_relation)) %}
          {% set preexisting_intermediate_relation = load_cached_relation(intermediate_relation) %}
        {% endif %}

        -- drop the temp relations if they exist already in the database
        {{ drop_relation_if_exists(preexisting_intermediate_relation) }}
        {{ drop_relation_if_exists(preexisting_backup_relation) }}

        {% if existing_relation.can_exchange %}
          {% call statement('main') -%}
            {{ clickhouse__create_target_table(backup_relation, sql, catchup_data) }}
          {%- endcall %}

          {# Drop MV just before exchange to minimize blind period while avoiding old MV writing to new table #}
          {{ clickhouse__drop_mvs_by_suffixes(target_relation, cluster_clause, views) }}

          {% do exchange_tables_atomic(backup_relation, existing_relation) %}

          {{ clickhouse__create_mvs(existing_relation, cluster_clause, refreshable_clause, views) }}
        {% else %}
          {{ clickhouse__replace_mv(target_relation, existing_relation, intermediate_relation, backup_relation, sql, views, catchup_data) }}
        {% endif %}
        {% set did_build = True %}
      {% else %}
        {% call statement('main') -%}
          select 1
        {%- endcall %}

        {%- set on_schema_change = incremental_validate_on_schema_change(config.get('on_schema_change'), default='ignore') -%}
        {{ log('on_schema_change strategy for destination table of  MV: ' + on_schema_change, info=True) }}
        {%- if on_schema_change != 'ignore' -%}
          {% do exceptions.raise_compiler_error("ClickHouse materialized_view on_schema_change != 'ignore' is not yet supported in Fusion") %}
        {%- endif %}

        {{ clickhouse__update_changed_mvs(target_relation, cluster_clause, refreshable_clause, views, query_changes) }}
        {% set did_build = True %}
      {% endif %}
    {% endif %}
  {% endif %}
  {% if did_build %}
    -- cleanup
    {% set should_revoke = should_revoke(existing_relation, full_refresh_mode=True) %}
    {% do apply_grants(target_relation, grant_config, should_revoke=should_revoke) %}

    {% do persist_docs(target_relation, model) %}
  {% endif %}

  {{ run_hooks(post_hooks, inside_transaction=True) }}

  {{ adapter.commit() }}

  {% if did_build %}
    {{ drop_relation_if_exists(backup_relation) }}
  {% endif %}

  {{ run_hooks(post_hooks, inside_transaction=False) }}

  {% set relations = [target_relation] %}
  {% for view in views %}
    {{ relations.append(clickhouse__make_mv_relation(target_relation, '_' + view)) }}
  {% endfor %}

  {{ return({'relations': relations}) }}
{% endmacro %}


{#
  Creates a target table for materialized views with optional catchup logic.
  If catchup is True, backfills the table with data from the SQL query.
  If catchup is False, creates an empty table without backfilling.
#}
{% macro clickhouse__create_target_table(relation, sql, catchup=True) -%}
  {% if catchup == True %}
    {{ get_create_table_as_sql(False, relation, sql) }}
  {% else %}
    {{ log('Catchup config set to false, skipping table backfill for ' + relation.name) }}
    {% set has_contract = config.get('contract').enforced %}
    {{ create_table_or_empty(False, relation, sql, has_contract) }}
  {% endif %}
{%- endmacro %}


{#
  There are two steps to creating a materialized view:
  1. Create a new table based on the SQL in the model
  2. Create a materialized view using the SQL in the model that inserts
  data into the table created during step 1
#}
{% macro clickhouse__get_create_materialized_view_as_sql(relation, sql, views, catchup=True ) -%}
  {% call statement('main') %}
    {{ clickhouse__create_target_table(relation, sql, catchup) }}
  {% endcall %}
  {%- set cluster_clause = on_cluster_clause(relation) -%}
  {%- set refreshable_clause = refreshable_mv_clause() -%}
  {{ clickhouse__create_mvs(relation, cluster_clause, refreshable_clause, views) }}
{%- endmacro %}

{% macro clickhouse__make_mv_relation(target_relation, suffix, interpret_suffix_as_full_identifier=False) %}
  {%- if interpret_suffix_as_full_identifier -%}
    {%- set identifier = suffix -%}
  {%- else -%}
    {%- set identifier = target_relation.identifier ~ suffix -%}
  {%- endif -%}
  {%- set mv_relation = api.Relation.create(
      database=target_relation.database,
      schema=target_relation.schema,
      identifier=identifier,
      type='materialized_view'
  ) -%}
  {{ return(mv_relation) }}
{% endmacro %}

{% macro clickhouse__normalize_relation_database(relation) %}
  {# ClickHouse relations in Fusion use an empty database and schema carries the ClickHouse database. #}
  {%- set normalized_relation = api.Relation.create(
      database='',
      schema=relation.schema,
      identifier=relation.identifier,
      type=relation.type
  ) -%}
  {{ return(normalized_relation) }}
{% endmacro %}

{% macro clickhouse__materialized_view_execute_no_op(relation) %}
  {% do store_raw_result(
      name="main",
      message="skip " ~ relation,
      code="skip",
      rows_affected="-1"
  ) %}
{% endmacro %}

{% macro clickhouse__materialized_view_desired_config(mv_relation, sql, refreshable_clause) %}
  {{ return({
    'target_table': clickhouse__expected_target_table_config(),
    'target_table_columns': get_column_schema_from_query(sql),
    'materialized_views': {
      mv_relation.identifier: {
        'query': sql,
        'refreshable_clause': refreshable_clause
      }
    }
  }) }}
{% endmacro %}

{% macro clickhouse__expected_target_table_config() %}
  {{ return({
    'engine': config.get('engine', default='MergeTree()'),
    'order_by': config.get('order_by'),
    'primary_key': config.get('primary_key'),
    'partition_by': config.get('partition_by'),
    'ttl': config.get('ttl')
  }) }}
{% endmacro %}

{% macro clickhouse__drop_mv(mv_relation, cluster_clause)  -%}
  {% call statement('drop existing mv: ' + mv_relation.name) -%}
    drop view if exists {{ mv_relation }} {{ cluster_clause }}
  {% endcall %}
{%- endmacro %}

{% macro clickhouse__create_mv(mv_relation, target_relation, cluster_clause, refreshable_clause, view_sql, is_main_statement=False)  -%}
  {% set statement_name = 'main' if is_main_statement else 'create existing mv: ' + mv_relation.name -%}
  {% call statement(statement_name) -%}
    create materialized view {{ mv_relation }} {{ cluster_clause }}
    {{ refreshable_clause }}
    to {{ target_relation }}
    as {{ view_sql }}
  {% endcall %}
{%- endmacro %}

{% macro clickhouse__assert_generated_mv_names_available(target_relation, views) %}
  {% for view in views.keys() %}
    {%- set mv_relation = clickhouse__make_mv_relation(target_relation, '_' + view) -%}
    {% do clickhouse__assert_generated_mv_relation_available(target_relation, mv_relation) %}
  {% endfor %}
{% endmacro %}

{% macro clickhouse__assert_generated_mv_relation_available(target_relation, mv_relation) %}
  {%- set existing_mv_relation = load_cached_relation(mv_relation) -%}
  {% if existing_mv_relation is not none %}
    {{ exceptions.raise_compiler_error(
      "ClickHouse materialized_view model " ~ target_relation.name
      ~ " needs to create generated materialized view " ~ mv_relation.name
      ~ ", but a relation with that name already exists. Drop or rename that relation, "
      ~ "or choose a different model alias."
    ) }}
  {% endif %}
{% endmacro %}

{% macro clickhouse__assert_generated_mv_relations_owned_by_target(target_relation, current_config, views) %}
  {%- set materialized_views = current_config.get('materialized_views', {}) -%}
  {% for view in views.keys() %}
    {%- set mv_relation = clickhouse__make_mv_relation(target_relation, '_' + view) -%}
    {%- set existing_mv_relation = load_cached_relation(mv_relation) -%}
    {%- set current_mv = materialized_views.get(mv_relation.identifier, none) -%}
    {% if existing_mv_relation is not none and current_mv is none %}
      {{ exceptions.raise_compiler_error(
        "ClickHouse materialized_view model " ~ target_relation.name
        ~ " needs generated materialized view " ~ mv_relation.name
        ~ ", but a relation with that name already exists and does not point to the model target table. "
        ~ "Drop or rename that relation, or choose a different model alias."
      ) }}
    {% endif %}
  {% endfor %}
{% endmacro %}

{% macro clickhouse__modify_mv(mv_relation, cluster_clause, view_sql, is_main_statement=False)  -%}
  {% set statement_name = 'main' if is_main_statement else 'modify existing mv: ' + mv_relation.name -%}
  {% call statement(statement_name) -%}
    alter table {{ mv_relation }} {{ cluster_clause }} modify query {{ view_sql }}
  {% endcall %}
{%- endmacro %}

{% macro clickhouse__update_changed_mvs(target_relation, cluster_clause, refreshable_clause, views, query_changes)  -%}
  {% for view, view_sql in views.items() %}
    {%- set mv_relation = clickhouse__make_mv_relation(target_relation, '_' + view) -%}
    {% if mv_relation.identifier in query_changes.recreate %}
      {{ clickhouse__drop_mv(mv_relation, cluster_clause) }};
      {{ clickhouse__create_mv(mv_relation, target_relation, cluster_clause, refreshable_clause, view_sql) }};
    {% elif mv_relation.identifier in query_changes.changed %}
      {{ clickhouse__modify_mv(mv_relation, cluster_clause, view_sql) }};
    {% elif mv_relation.identifier in query_changes.missing %}
      {% do clickhouse__assert_generated_mv_relation_available(target_relation, mv_relation) %}
      {{ clickhouse__create_mv(mv_relation, target_relation, cluster_clause, refreshable_clause, view_sql) }};
    {% endif %}
  {% endfor %}
{%- endmacro %}

{% macro clickhouse__drop_mvs_by_suffixes(target_relation, cluster_clause, views_suffixes)  -%}
  {% for suffix in views_suffixes.keys() %}
    {%- set mv_relation = clickhouse__make_mv_relation(target_relation, '_' + suffix) -%}
    {{ clickhouse__drop_mv(mv_relation, cluster_clause) }};
  {% endfor %}
{%- endmacro %}

{% macro clickhouse__drop_mvs_by_names(target_relation, cluster_clause, mvs_names)  -%}
  {% for mvs_name in mvs_names %}
    {%- set mv_relation = clickhouse__make_mv_relation(target_relation, mvs_name, interpret_suffix_as_full_identifier=True) -%}
    {{ clickhouse__drop_mv(mv_relation, cluster_clause) }};
  {% endfor %}
{%- endmacro %}

{% macro clickhouse__create_mvs(target_relation, cluster_clause, refreshable_clause, views)  -%}
  {% for view, view_sql in views.items() %}
    {%- set mv_relation = clickhouse__make_mv_relation(target_relation, '_' + view) -%}
    {{ clickhouse__create_mv(mv_relation, target_relation, cluster_clause, refreshable_clause, view_sql) }};
  {% endfor %}
{%- endmacro %}

{% macro clickhouse__search_associated_mvs_to_target(relation_schema, relation_name, mv_suffixes)  -%}
  {% set tables_query = clickhouse__associated_mvs_to_target_sql(relation_schema, relation_name) %}

  {% set expected_mvs = [] %}
  {% for suffix in mv_suffixes.keys() %}
    {% do expected_mvs.append(relation_name ~ "_" ~ suffix) %}
  {% endfor %}
  {{ log('Model mvs to replace ' + expected_mvs | string) }}

  {% set mvs_found = run_query(tables_query) %}
  {% if mvs_found is not none and mvs_found.columns %}
    {% set mv_found_names = mvs_found.columns[0].values() %}
    {{ log('Current mvs found in ClickHouse are: ' + mv_found_names | join(', ')) }}
    {{ return((mv_found_names, expected_mvs,)) }}
  {% else %}
    {{ return(([], expected_mvs,)) }}
  {% endif %}
{%- endmacro %}

{% macro clickhouse__associated_mvs_to_target_sql(relation_schema, relation_name)  -%}
  {% set normalized_schema_target = relation_schema ~ '.' ~ relation_name %}
  {% set tables_query %}
    select name
    from system.tables
    where engine = 'MaterializedView'
      and database = '{{ relation_schema }}'
      and lower(replaceRegexpAll(
        {#-
          Avoid question-mark regex syntax because the ClickHouse ADBC preview
          driver currently treats it as unbound query parameters.
          See https://github.com/ClickHouse/adbc_clickhouse/issues/53.
        -#}
        extract(
          lower(create_table_query),
          '\\bto\\s+((`[^`]+`|"[^"]+"|[^\\s.()]+)(\\s*\\.\\s*(`[^`]+`|"[^"]+"|[^\\s.()]+))*)'
        ),
        '[`"\\s]',
        ''
      )) in (
        lower(replaceRegexpAll('{{ normalized_schema_target }}', '[`"\\s]', '')),
        lower(replaceRegexpAll('{{ relation_name }}', '[`"\\s]', ''))
      )
  {% endset %}
  {{ return(tables_query) }}
{%- endmacro %}


{% macro clickhouse__drop_associated_mv_if_it_was_automatically_created(target_relation)  -%}
  {#-
    Limitations of this logic:
     - Only covers situations where 1 mv was created. Don't cover multiple mvs with different names.
     - Only checks current relation's database.
    We print logs in case we find other mvs in that database
  -#}
  {% set views = {'mv': ''} %}
  {%- set cluster_clause = on_cluster_clause(target_relation) -%}
  {% set found_associated_mvs, expected_mv_tables = clickhouse__search_associated_mvs_to_target(target_relation.schema, target_relation.name, views) %}

  {% if found_associated_mvs is not none %}
    {% for suffix in views.keys() %}
      {%- set mv_relation = clickhouse__make_mv_relation(target_relation, '_' + suffix) -%}
      {% if mv_relation.identifier in found_associated_mvs %}
        {{ clickhouse__drop_mv(mv_relation, cluster_clause) }};
      {% endif %}
    {% endfor %}
  {% endif %}

  {% if found_associated_mvs is not none %}
    {% for table in found_associated_mvs %}
      {% if table not in expected_mv_tables %}
        {{ log('Warning - Materialized View "' + table + '" was detected pointing to the model name "' + target_relation.name + '" that was just updated/removed. It can\'t be automatically removed by DBT. Drop it manually if needed (!!!)', info=True) }}
      {% endif %}
    {% endfor %}
  {% endif %}
  {# Preserve the warning path for unexpected associated MVs while relying on
     the generated name for the dbt-owned MV. #}
{%- endmacro %}

{% macro clickhouse__replace_mv(target_relation, existing_relation, intermediate_relation, backup_relation, sql, views, catchup=True) %}
  {# drop existing materialized view while we recreate the target table #}
  {%- set cluster_clause = on_cluster_clause(target_relation) -%}
  {%- set refreshable_clause = refreshable_mv_clause() -%}
  {{ clickhouse__drop_mvs_by_suffixes(target_relation, cluster_clause, views) }}

  {# recreate the target table #}
  {% call statement('main') -%}
    {{ clickhouse__create_target_table(intermediate_relation, sql, catchup) }}
  {%- endcall %}
  {{ adapter.rename_relation(existing_relation, backup_relation) }}
  {{ adapter.rename_relation(intermediate_relation, target_relation) }}

  {# now that the target table is recreated, we can finally create our new view #}
  {{ clickhouse__create_mvs(target_relation, cluster_clause, refreshable_clause, views) }}
{% endmacro %}

{% macro refreshable_mv_clause() %}
  {%- if config.get('refreshable') is not none -%}

    {% set refreshable_config = config.get('refreshable') %}
    {% if refreshable_config is not mapping %}
      {% do exceptions.raise_compiler_error(
        "The 'refreshable' configuration must be defined as a dictionary. Please review the docs for more details."
      ) %}
    {% endif %}

    {% set refresh_interval = refreshable_config.get('interval', none) %}
    {% set refresh_randomize = refreshable_config.get('randomize', none) %}
    {% set depends_on = refreshable_config.get('depends_on', none) %}
    {% set depends_on_validation = refreshable_config.get('depends_on_validation', false) %}
    {% set append = refreshable_config.get('append', false) %}

    {% if not refresh_interval %}
      {% do exceptions.raise_compiler_error(
        "The 'refreshable' configuration is defined, but 'interval' is missing. "
        ~ "This is required to create a refreshable materialized view."
      ) %}
    {% endif %}

    {% if refresh_interval %}
      REFRESH {{ refresh_interval }}
      {# This is a comment to force a new line between REFRESH and RANDOMIZE clauses #}
      {%- if refresh_randomize -%}
        RANDOMIZE FOR {{ refresh_randomize }}
      {%- endif -%}
    {% endif %}

    {% if depends_on %}
      {% set depends_on_list = [] %}

      {% if depends_on is string %}
        {% set depends_on_list = [depends_on] %}
      {% elif depends_on is iterable %}
        {% set temp_list = depends_on_list %}
        {%- for dep in depends_on %}
          {% if dep is string %}
            {% do temp_list.append(dep) %}
          {% else %}
            {% do exceptions.raise_compiler_error(
              "The 'depends_on' configuration must be either a string or a list of strings."
            ) %}
          {% endif %}
        {% endfor %}
        {% set depends_on_list = temp_list %}
      {% else %}
        {% do exceptions.raise_compiler_error(
          "The 'depends_on' configuration must be either a string or a list of strings."
        ) %}
      {% endif %}

      {% if depends_on_validation and depends_on_list | length > 0 %}
        {%- for dep in depends_on_list %}
          {% do validate_refreshable_mv_existence(dep) %}
        {%- endfor %}
      {% endif %}

      DEPENDS ON {{ depends_on_list | join(', ') }}
    {% endif %}

    {%- if append -%}
      APPEND
    {%- endif -%}

  {%- endif -%}
{% endmacro %}


{% macro validate_refreshable_mv_existence(mv) %}
  {{ log(mv + ' was recognized as a refreshable mv dependency, checking its existence') }}
  {% set default_database = "default" %}

  {%- set parts = mv.split('.') %}
  {%- if parts | length == 2 %}
    {%- set database = parts[0] %}
    {%- set table = parts[1] %}
  {%- else %}
    {%- set database = default_database %}
    {%- set table = parts[0] %}
  {%- endif %}

  {%- set condition = "database='" + database + "' and view='" + table + "'" %}

  {% set query %}
    select count(*)
    from system.view_refreshes
    where {{ condition }}
  {% endset %}

  {% set tables_result = run_query(query) %}
    {{ log(tables_result.columns[0].values()[0]) }}
  {% if tables_result.columns[0].values()[0] > 0 %}
    {{ log('MV ' + mv + ' exists.') }}
  {% else %}
    {% do exceptions.raise_compiler_error(
      'No existing MV found matching MV: ' + mv
    ) %}
  {% endif %}
{% endmacro %}
