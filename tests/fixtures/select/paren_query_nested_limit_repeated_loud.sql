-- The clauses of a query parenthesized twice still merge into it as 18.6's
-- `insertSelectOptions` merges them: the inner `LIMIT` reaches the outermost one through
-- the `ORDER BY` between them. What pglet merges is its record of which clauses are
-- present, which is what refuses the repeat; it plans each level's clauses in turn
-- (`EDGES.md` row 167).
-- @setup
CREATE TABLE pqn_s (x INT);
INSERT INTO pqn_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
((SELECT x FROM pqn_s LIMIT 1) ORDER BY x) LIMIT 2;
