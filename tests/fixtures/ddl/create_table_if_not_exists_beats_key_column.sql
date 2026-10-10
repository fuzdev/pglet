-- `IF NOT EXISTS` is tested at the top of `transformCreateStmt`, ahead of both the
-- column walk and `transformIndexConstraints`, so it outranks the key-column 42703 the
-- same way it outranks 42P16 — the statement is a plain skip, not an error, even though
-- the definition it skips is unbuildable.
--
-- The counterpart without the clause is `create_table_key_column_beats_existing_table.sql`.
-- @setup
CREATE TABLE ctie (a INT);
CREATE TABLE IF NOT EXISTS ctie (a INT, PRIMARY KEY (nosuch));

-- @test
SELECT column_name::text AS column_name
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'ctie'
ORDER BY ordinal_position;
