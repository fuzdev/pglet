-- The refusal holds for a parenthesized query inside a derived table.
-- @setup
CREATE TABLE pqt_s (x INT);
INSERT INTO pqt_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
SELECT * FROM ((SELECT x FROM pqt_s LIMIT 1) LIMIT 2) d;
