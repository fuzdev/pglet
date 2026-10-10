-- The same skip on the other statement, where it is one of three gates the refusal loses
-- to rather than the only one.
-- @setup
CREATE TABLE rtaskip (a INT);
ALTER TABLE rtaskip ADD COLUMN IF NOT EXISTS a CHAR(0);
ALTER TABLE rtaskip ADD COLUMN IF NOT EXISTS a NUMERIC(0);
-- @no-declared-types: 18.6 sends `name` (19) for `column_name` and
--   `information_schema.character_data` for `data_type`, where pglet sends `text` (25).
-- @test
SELECT column_name, data_type FROM information_schema.columns
WHERE table_name = 'rtaskip' ORDER BY ordinal_position;
