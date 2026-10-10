-- A name both the sibling and the enclosing query carry: 18.6 reads the sibling's
-- (`3`, `3`). Read once against the enclosing row it would answer other counts silently,
-- so it is refused (pglet-stricter).
-- @setup
CREATE TABLE gs_o (id INT, n INT);
CREATE TABLE gs_i (n INT);
INSERT INTO gs_o VALUES (1, 1), (2, 2);
INSERT INTO gs_i VALUES (1), (2);
-- @expect-error: generate_series with a column reference
-- @test
SELECT o.id, (SELECT count(*) FROM gs_i, generate_series(1, n) g) FROM gs_o o ORDER BY 1;
