# Drizzle-kit Introspection Queries

Captured from `drizzle-kit push` via spy proxy.

> A reference capture of the introspection queries a drizzle-kit-style consumer issues. The
> lists below are **drizzle-kit's requirements, not pglet's roadmap**: pglet grows its
> `pg_catalog` / `information_schema` surface as its consumers need it, and
> `docs/compatibility.md` is the source of truth for what it synthesizes today.

**Related files:**

- `tools/spy_proxy/main.ts` - Proxy that captured these queries
- `tests-web/orms/drizzle/schema.ts` - Schema that triggered these queries
- `tests-web/orms/drizzle/drizzle_test.ts` - Drizzle ORM integration test
- `crates/pglet_sql/src/catalog/virtual_tables.rs` - CatalogTable enum + column schemas
- `crates/pglet_sql/src/catalog/pg_catalog.rs` - Virtual pg_* table data generation
- `crates/pglet_sql/src/planner/plan.rs` - `Plan::VirtualScan` for catalog tables

## 1. List Tables/Views

```sql
SELECT
    n.nspname AS table_schema,
    c.relname AS table_name,
    CASE
        WHEN c.relkind = 'r' THEN 'table'
        WHEN c.relkind = 'v' THEN 'view'
        WHEN c.relkind = 'm' THEN 'materialized_view'
    END AS type,
    c.relrowsecurity AS rls_enabled
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'v', 'm')
  AND n.nspname = 'public';
```

## 2. List Custom Schemas

```sql
SELECT s.nspname AS table_schema
FROM pg_catalog.pg_namespace s
JOIN pg_catalog.pg_user u ON u.usesysid = s.nspowner
WHERE nspname NOT IN ('information_schema', 'pg_catalog', 'public')
  AND nspname NOT LIKE 'pg_toast%'
  AND nspname NOT LIKE 'pg_temp_%'
ORDER BY table_schema;
```

## 3. List Sequences

```sql
SELECT schemaname, sequencename, start_value, min_value, max_value,
       increment_by, cycle, cache_size
FROM pg_sequences
WHERE schemaname = 'public';
```

## 4. List Enums

```sql
SELECT n.nspname AS enum_schema,
       t.typname AS enum_name,
       e.enumlabel AS enum_value,
       e.enumsortorder AS sort_order
FROM pg_type t
JOIN pg_enum e ON t.oid = e.enumtypid
JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
WHERE n.nspname = 'public'
ORDER BY enum_schema, enum_name, sort_order;
```

## 5. List Roles

```sql
SELECT rolname, rolinherit, rolcreatedb, rolcreaterole
FROM pg_roles;
```

## 6. List RLS Policies

```sql
SELECT schemaname, tablename, policyname AS name,
       permissive AS "as", roles AS to, cmd AS for,
       qual AS using, with_check AS "withCheck"
FROM pg_policies
WHERE schemaname = 'public';
```

## 7. Column Details (per table)

Complex query joining:

- `pg_attribute` - column metadata
- `pg_class` - table info
- `pg_namespace` - schema info
- `information_schema.columns` - additional column details
- `pg_type` - type info
- `pg_attrdef` - default values
- `pg_get_serial_sequence()` - serial detection

Returns: table_name, column_name, is_nullable, array_dimensions, data_type,
seq_name, column_default, enum_name, is_generated, is_identity, identity_*
fields

## 8. Primary Key Constraints

```sql
SELECT c.column_name, c.data_type, constraint_type,
       constraint_name, constraint_schema
FROM information_schema.table_constraints tc
JOIN information_schema.constraint_column_usage AS ccu
  USING (constraint_schema, constraint_name)
JOIN information_schema.columns AS c
  ON c.table_schema = tc.constraint_schema
  AND tc.table_name = c.table_name
  AND ccu.column_name = c.column_name
WHERE tc.table_name = '{table}' AND constraint_schema = 'public';
```

## 9. Check Constraints

```sql
SELECT tc.constraint_name, tc.constraint_type,
       pg_get_constraintdef(con.oid) AS constraint_definition
FROM information_schema.table_constraints AS tc
JOIN pg_constraint AS con ON tc.constraint_name = con.conname
  AND con.conrelid = (SELECT oid FROM pg_class WHERE relname = tc.table_name ...)
WHERE tc.table_name = '{table}'
  AND tc.constraint_schema = 'public'
  AND tc.constraint_type = 'CHECK'
  AND con.contype = 'c';
```

## 10. Foreign Keys

```sql
SELECT con.contype AS constraint_type,
       nsp.nspname AS constraint_schema,
       con.conname AS constraint_name,
       rel.relname AS table_name,
       att.attname AS column_name,
       fnsp.nspname AS foreign_table_schema,
       frel.relname AS foreign_table_name,
       fatt.attname AS foreign_column_name,
       CASE con.confupdtype ... END AS update_rule,
       CASE con.confdeltype ... END AS delete_rule
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class rel ON rel.oid = con.conrelid
JOIN pg_catalog.pg_namespace nsp ON nsp.oid = con.connamespace
LEFT JOIN pg_catalog.pg_attribute att ON att.attnum = ANY(con.conkey) ...
LEFT JOIN pg_catalog.pg_class frel ON frel.oid = con.confrelid
...
WHERE nsp.nspname = 'public'
  AND rel.relname = '{table}'
  AND con.contype IN ('f');
```

## 11. Indexes

```sql
SELECT DISTINCT ON (t.relname, ic.relname, k.i)
       t.relname AS table_name,
       ic.relname AS indexname,
       k.i AS index_order,
       i.indisunique AS is_unique,
       am.amname AS method,
       ic.reloptions AS with,
       coalesce(a.attname, pg_get_indexdef(i.indexrelid, k.i, false)) AS column_name,
       CASE WHEN pg_get_expr(i.indexprs, i.indrelid) IS NOT NULL THEN 1 ELSE 0 END AS is_expression,
       i.indoption[k.i-1] & 1 = 1 AS descending,
       i.indoption[k.i-1] & 2 = 2 AS nulls_first,
       pg_get_expr(i.indpred, i.indrelid) AS where,
       opc.opcname
FROM pg_class t
LEFT JOIN pg_index i ON t.oid = i.indrelid
LEFT JOIN pg_class ic ON ic.oid = i.indexrelid
CROSS JOIN LATERAL (SELECT unnest(i.indkey), generate_subscripts(i.indkey, 1) + 1) AS k(attnum, i)
LEFT JOIN pg_attribute AS a ON i.indrelid = a.attrelid AND k.attnum = a.attnum
JOIN pg_namespace c ON c.oid = t.relnamespace
LEFT JOIN pg_am AS am ON ic.relam = am.oid
JOIN pg_opclass opc ON opc.oid = ANY(i.indclass)
WHERE c.nspname = 'public' AND t.relname = '{table}';
```

## 12. Index Constraint Association

```sql
SELECT idx.indexrelname AS index_name,
       idx.relname AS table_name,
       schemaname,
       CASE WHEN con.conname IS NOT NULL THEN 1 ELSE 0 END AS generated_by_constraint
FROM pg_stat_user_indexes idx
LEFT JOIN pg_constraint con ON con.conindid = idx.indexrelid
WHERE idx.relname = '{table}' AND schemaname = 'public'
GROUP BY index_name, table_name, schemaname, generated_by_constraint;
```

---

## DDL Statements Observed

```sql
-- Table creation
CREATE TABLE IF NOT EXISTS "users" (
    "id" serial PRIMARY KEY NOT NULL,
    "name" text NOT NULL,
    "email" text NOT NULL,
    "age" integer,
    "active" boolean DEFAULT true,
    "created_at" timestamp DEFAULT now(),
    CONSTRAINT "users_email_unique" UNIQUE("email")
);

-- Foreign key
ALTER TABLE "posts" ADD CONSTRAINT "posts_author_id_users_id_fk"
    FOREIGN KEY ("author_id") REFERENCES "public"."users"("id")
    ON DELETE no action ON UPDATE no action;

-- Drop table
DROP TABLE "todo" CASCADE;
```

---

## Key pg_catalog Tables (drizzle-kit's introspection surface)

| Table                  | Purpose                             |
| ---------------------- | ----------------------------------- |
| `pg_class`             | Tables, views, indexes, sequences   |
| `pg_namespace`         | Schemas                             |
| `pg_attribute`         | Columns                             |
| `pg_type`              | Data types                          |
| `pg_enum`              | Enum values                         |
| `pg_constraint`        | Constraints (PK, FK, CHECK, UNIQUE) |
| `pg_index`             | Index metadata                      |
| `pg_attrdef`           | Column defaults                     |
| `pg_am`                | Access methods (btree, hash, etc.)  |
| `pg_opclass`           | Operator classes                    |
| `pg_roles`             | Roles/users                         |
| `pg_policies`          | RLS policies                        |
| `pg_sequences`         | Sequence metadata (view)            |
| `pg_stat_user_indexes` | Index stats (view)                  |

## Key Functions (drizzle-kit's introspection surface)

- `pg_get_constraintdef(oid)` - Get constraint definition
- `pg_get_serial_sequence(table, column)` - Get serial sequence name
- `pg_get_indexdef(indexrelid, column_no, pretty)` - Get index definition
- `pg_get_expr(expr, relid)` - Deparse expression
- `format_type(type_oid, typemod)` - Format type name
- `generate_subscripts(array, dim)` - Generate array subscripts
- `unnest(array)` - Expand array to rows
