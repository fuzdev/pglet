-- Postgres never lets one of its *own* generated names fail a statement: when
-- `<table>_<cols>_key` is taken, `ChooseRelationName` appends a digit. So a plain
-- index that happens to sit on the auto-name is worked around, not an error. Gating
-- the auto-name the way a declared one is gated would reject this valid input.
-- @setup
CREATE TABLE uand (a INT NOT NULL, b INT);
CREATE INDEX uand_a_key ON uand (b);
ALTER TABLE uand ADD UNIQUE (a);

-- @test
SELECT indexname::text AS indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'uand' ORDER BY indexname;
