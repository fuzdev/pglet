-- A CTE body leading with a parenthesized query that repeats a clause is refused, so the
-- `DELETE` reading it removes nothing.
-- @setup
CREATE TABLE cpq_s (x INT);
INSERT INTO cpq_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
WITH c AS ((SELECT x FROM cpq_s ORDER BY x LIMIT 1) LIMIT 2) DELETE FROM cpq_s WHERE x IN (SELECT x FROM c);
