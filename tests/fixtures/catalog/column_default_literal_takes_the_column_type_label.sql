-- A string `DEFAULT` was an *unknown* literal until `coerce_to_target_type` typed it,
-- so 18.6 labels it with the **column's** type — the enum's own name for an enum
-- column, `character varying` for a `varchar(n)` one. pglet reported `::text` for all
-- three, which reads to a schema differ as a plain text default on an enum column.
--
-- The `text` column is the polarity that must not move: its label was already right.
-- @setup
CREATE TYPE cdl_mood AS ENUM ('private', 'public');
CREATE TABLE cdl (
  m cdl_mood NOT NULL DEFAULT 'private',
  v VARCHAR(10) DEFAULT 'x',
  t TEXT DEFAULT 'plain',
  n INT2 DEFAULT -1,
  o INT8 DEFAULT -1,
  f FLOAT8 DEFAULT -1.5
);
-- @test
SELECT column_name::text AS column_name, data_type::text AS data_type,
       udt_name::text AS udt_name, column_default::text AS column_default
FROM information_schema.columns WHERE table_name::text = 'cdl'
ORDER BY ordinal_position ASC
