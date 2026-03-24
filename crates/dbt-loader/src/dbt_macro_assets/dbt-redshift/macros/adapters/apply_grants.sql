{% macro redshift__get_show_grant_sql(relation) %}

with privileges as (

    -- valid options per https://docs.aws.amazon.com/redshift/latest/dg/r_HAS_TABLE_PRIVILEGE.html
    select 'select' as privilege_type
    union all
    select 'insert' as privilege_type
    union all
    select 'update' as privilege_type
    union all
    select 'delete' as privilege_type
    union all
    select 'references' as privilege_type

)

select
    u.usename as grantee,
    p.privilege_type
from pg_user u
cross join privileges p
where has_table_privilege(u.usename, '{{ relation }}', privilege_type)
    and u.usename != current_user
    and not u.usesuper

{% endmacro %}


{%- macro redshift__format_grantee(grantee) -%}
    {%- if ':' in grantee or '.' in grantee or ' ' in grantee or '-' in grantee or '@' in grantee -%}
        "{{ grantee }}"
    {%- else -%}
        {{ grantee }}
    {%- endif -%}
{%- endmacro -%}


{%- macro redshift__get_grant_sql(relation, privilege, grantees) -%}
    {%- set formatted = [] -%}
    {%- for grantee in grantees -%}
        {%- do formatted.append(redshift__format_grantee(grantee)) -%}
    {%- endfor -%}
    grant {{ privilege }} on {{ relation.render() }} to {{ formatted | join(', ') }}
{%- endmacro -%}


{%- macro redshift__get_revoke_sql(relation, privilege, grantees) -%}
    {%- set formatted = [] -%}
    {%- for grantee in grantees -%}
        {%- do formatted.append(redshift__format_grantee(grantee)) -%}
    {%- endfor -%}
    revoke {{ privilege }} on {{ relation.render() }} from {{ formatted | join(', ') }}
{%- endmacro -%}
