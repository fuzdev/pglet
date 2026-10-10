-- key_column_usage lists a PK/UNIQUE constraint's key columns in **key order**, with
-- position_in_unique_constraint NULL (that column pairs an FK's columns with the
-- referenced key and is meaningless for a PK/UNIQUE). The composite UNIQUE is
-- declared (b, a) so the ordinal is the constraint's own order, not the table's.
-- Both constraints are left unnamed so each takes Postgres's auto-name, which pglet
-- reproduces; the declared-name spellings are covered by
-- `pk_declared_constraint_name.sql`.
-- @setup
CREATE TABLE iskcu (a INT, b INT, c INT, PRIMARY KEY (a, b), UNIQUE (b, a));

-- @test
SELECT constraint_name::text AS constraint_name, column_name::text AS column_name,
       ordinal_position::int AS ordinal_position,
       position_in_unique_constraint::int AS position_in_unique_constraint
FROM information_schema.key_column_usage
WHERE table_schema = 'public' AND table_name = 'iskcu'
ORDER BY constraint_name, ordinal_position;
