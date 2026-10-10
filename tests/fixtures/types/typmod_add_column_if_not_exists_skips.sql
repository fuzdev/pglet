-- `ADD COLUMN IF NOT EXISTS` over a present column **skips** on both engines, and the
-- modifier is never judged — the skip is a Postgres *success*, and a gate that stands
-- in for no Postgres error must run after every gate that decides one.
--
-- Asserted as a value fixture rather than an error one: the consequence a broken
-- ordering would show is that the statement raises at all, and the surviving column
-- shape is what an accidental widening would also disturb.
-- @setup
CREATE TABLE tmskip (a INT);
ALTER TABLE tmskip ADD COLUMN IF NOT EXISTS a int4(3);
ALTER TABLE tmskip ADD COLUMN IF NOT EXISTS a nosuchtype;
-- @no-declared-types: 18.6 sends `name` (19) for `column_name` and
--   `information_schema.character_data` for `data_type`, where pglet sends `text` (25).
-- @test
SELECT column_name, data_type FROM information_schema.columns
WHERE table_name = 'tmskip' ORDER BY ordinal_position;
