-- The column-definition position takes the refusal in the ordered column walk, so the
-- statement is rejected outright rather than landing an **unconstrained** column. That
-- column was the silent-and-writing half: `CREATE TABLE q (c CHAR(0))` built one here and
-- `INSERT INTO q VALUES ('abcdef')` stored the row, where 18.6 has no table to hold it.
-- @expect-error: a character length below 1 at a column definition
-- @expect-message
-- @test
CREATE TABLE rtcc (c CHAR(0));
