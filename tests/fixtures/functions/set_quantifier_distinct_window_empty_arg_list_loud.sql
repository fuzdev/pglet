-- The window route bypasses `build_call` entirely (it needs the `OVER` clause, so its
-- dispatch is a separate match), which is why the grammar bound lives in the parser
-- ahead of that fork rather than in the dispatch. Without it this answered `1, 2, 3`.
-- 42601 on both.
-- @setup
CREATE TABLE sq_wr (a INT);
INSERT INTO sq_wr VALUES (1), (2), (3);
-- @test
-- @expect-error: syntax error at or near ")"
SELECT row_number(DISTINCT) OVER () FROM sq_wr;
