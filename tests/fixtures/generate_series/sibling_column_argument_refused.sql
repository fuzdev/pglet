-- An argument naming an earlier FROM item's column: 18.6 reads each row's (lateral);
-- pglet evaluates the item once, so it refuses (pglet-stricter).
-- @setup
CREATE TABLE gs_l (id INT, n INT);
INSERT INTO gs_l VALUES (1, 2), (2, 0), (3, 3);
-- @expect-error: generate_series with a column reference
-- @test
SELECT * FROM gs_l, generate_series(1, n) g;
