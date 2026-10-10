-- An enclosing query's column, qualified: 18.6 reads each outer row's; refused here
-- (pglet-stricter).
-- @setup
CREATE TABLE gs_c (id INT, n INT);
INSERT INTO gs_c VALUES (1, 2), (2, 0);
-- @expect-error: generate_series with a column reference
-- @test
SELECT id, (SELECT count(*) FROM generate_series(1, gs_c.n)) FROM gs_c ORDER BY id;
