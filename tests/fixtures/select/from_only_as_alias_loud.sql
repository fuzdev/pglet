-- `ONLY` is reserved, so it is no alias.
-- @setup
CREATE TABLE foa (id INT);
-- @expect-error: syntax error at or near "only"
-- @test
SELECT * FROM foa only;
