-- The third gate is a Postgres **success**, and a gate standing in for no Postgres error
-- must run after every gate that decides one: `IF NOT EXISTS` sits at the top of
-- `transformCreateStmt`, ahead of the `tableElts` walk, so the skip outranks the refusal
-- even though a *plain* `CREATE TABLE` over the same relation answers it.
--
-- Asserted as a value fixture rather than an error one: what a broken ordering would show
-- is that the statement raises at all, and the surviving column shape is what an
-- accidental widening would also disturb.
-- @setup
CREATE TABLE rtcine (a INT);
CREATE TABLE IF NOT EXISTS rtcine (c CHAR(0));
CREATE TABLE IF NOT EXISTS rtcine (c NUMERIC(5,2,3));
-- @no-declared-types: 18.6 sends `name` (19) for `column_name` and
--   `information_schema.character_data` for `data_type`, where pglet sends `text` (25).
-- @test
SELECT column_name, data_type FROM information_schema.columns
WHERE table_name = 'rtcine' ORDER BY ordinal_position;
