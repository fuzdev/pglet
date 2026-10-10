-- A user-declared `CONSTRAINT <name>` on a PRIMARY KEY / UNIQUE is reported under
-- that name, not pglet's auto-name. The declared name used to be dropped at parse, so
-- introspection reported a plausible-but-wrong `<table>_pkey` — undetectable to the
-- client. Covers all three spellings: table-level, column-level, and ADD CONSTRAINT.
-- @setup
CREATE TABLE isdc (a INT NOT NULL, b INT, c INT,
  CONSTRAINT isdc_my_pk PRIMARY KEY (a),
  CONSTRAINT isdc_my_uq UNIQUE (b));
CREATE TABLE isdc2 (a INT CONSTRAINT isdc2_col_pk PRIMARY KEY, b INT);
CREATE TABLE isdc3 (a INT NOT NULL);
ALTER TABLE isdc3 ADD CONSTRAINT isdc3_added_pk PRIMARY KEY (a);

-- @test
SELECT constraint_name::text AS constraint_name, table_name::text AS table_name,
       constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name IN ('isdc', 'isdc2', 'isdc3')
  AND constraint_type IN ('PRIMARY KEY', 'UNIQUE')
ORDER BY constraint_name;
