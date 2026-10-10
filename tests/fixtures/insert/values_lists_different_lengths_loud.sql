-- An `INSERT … VALUES` list whose rows differ in length is refused in parse analysis —
-- `42601 VALUES lists must all be the same length` — even with no column list and every row
-- within the table's width. pglet used to pad the short row with its defaults, as a one-row
-- `INSERT` is padded, and write `1|2`, `3|7`.
-- @setup
CREATE TABLE vld (a INT, b INT DEFAULT 7);

-- @test
-- @expect-error: VALUES lists must all be the same length (42601)
-- @expect-message
INSERT INTO vld VALUES (1, 2), (3);
