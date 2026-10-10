-- `IF NOT EXISTS` requires the name: `gram.y`'s `IndexStmt` spells the two productions
-- apart (`opt_single_name` in the plain one, `name` behind `IF NOT EXISTS`), so there is
-- no unnamed index to skip over and 18.6 answers `42601 syntax error at or near "ON"`.
-- @setup
CREATE TABLE ciu21 (a INT);
-- @test
CREATE INDEX IF NOT EXISTS ON ciu21 (a);
-- @expect-error: syntax error at or near "ON" (42601)
-- @expect-message
