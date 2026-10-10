-- The `HAVING` spelling of a drop nested in a drop: the `sum(1/b)` after the inner
-- dropped `max(a)` is still inside the outer `false AND`, so no group is kept and
-- nothing divides by the `b = 0` row.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT count(*) AS n FROM u HAVING false AND ((true OR max(a) > 0) AND sum(1/b) > 0);
