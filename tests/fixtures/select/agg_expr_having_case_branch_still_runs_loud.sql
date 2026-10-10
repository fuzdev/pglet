-- The `HAVING` spelling of the untaken-branch aggregate: it still runs. Computing an
-- aggregate only where the `CASE` reads it would keep both groups.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT a FROM u GROUP BY a HAVING CASE WHEN a > 0 THEN true ELSE sum(1/b) > 0 END;
