-- The mirror on `CREATE TABLE`, where the asymmetry runs the other way: the
-- `IF NOT EXISTS` test sits at the very top of `transformCreateStmt`, ahead of the
-- `tableElts` walk that resolves the column types — so the skip outranks the modifier
-- refusal here too, while a *plain* `CREATE TABLE` over an existing relation answers
-- the type's `42601` rather than `42P07`.
--
-- Nothing else in the corpus spells `CREATE TABLE IF NOT EXISTS` with a refused type,
-- so moving the gate ahead of the skip would otherwise be green on every gate.
-- @setup
CREATE TABLE tmcine (a INT);
CREATE TABLE IF NOT EXISTS tmcine (a int4(3));
CREATE TABLE IF NOT EXISTS tmcine (a nosuchtype);
-- @no-declared-types: 18.6 sends `name` (19) for `column_name` and
--   `information_schema.character_data` for `data_type`, where pglet sends `text` (25).
-- @test
SELECT column_name, data_type FROM information_schema.columns
WHERE table_name = 'tmcine' ORDER BY ordinal_position;
