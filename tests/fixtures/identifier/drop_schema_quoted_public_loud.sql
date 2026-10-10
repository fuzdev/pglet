-- The destructive one. `execute_drop_schema` compared its argument to `public`
-- case-insensitively and without a quotedness bit, so `DROP SCHEMA "PUBLIC" CASCADE`
-- matched and cleared the entire catalog and its storage — every table and every row —
-- where 18.6 answers `3F000 schema "PUBLIC" does not exist`. The comparison is exact
-- now, over a name the parser has already folded when it was unquoted.
-- This fixture pins the refusal. That the rows **survive** it is a separate guard and
-- cannot live here: the `@test` is the failing statement, so the fixture has nowhere
-- to observe the table afterwards. See `crates/pglet/tests/case_folding.rs`.
-- @setup
CREATE TABLE keepme (a INT);
INSERT INTO keepme VALUES (1), (2), (3);

-- @test
DROP SCHEMA "PUBLIC" CASCADE;
-- @expect-error: a quoted schema name is exact, so `PUBLIC` is not the implicit schema (3F000)
