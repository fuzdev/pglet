-- An index over each immutable new function builds and reads back as 18.6 prints it.
-- @setup
CREATE TABLE ix (a INT, t TEXT, b BYTEA);
CREATE INDEX ix_l ON ix (left(t, 2));
CREATE INDEX ix_m ON ix (md5(t));
CREATE INDEX ix_mod ON ix (mod(a, 3));
CREATE INDEX ix_o ON ix (octet_length(t));
CREATE INDEX ix_s ON ix (substr(b, 1, 2));
-- @no-declared-types: 18.6 sends `name` (19) for `indexname` and pglet sends `text`
--   (25) — pre-existing catalog-type modelling, not this fixture's subject.
-- @test
SELECT indexname, indexdef FROM pg_indexes WHERE tablename = 'ix' ORDER BY indexname;
