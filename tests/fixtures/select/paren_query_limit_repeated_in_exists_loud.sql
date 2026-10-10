-- The refusal holds for a parenthesized query under `EXISTS`.
-- @setup
CREATE TABLE pqe_s (x INT);
INSERT INTO pqe_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
SELECT EXISTS ((SELECT x FROM pqe_s LIMIT 1) LIMIT 2);
