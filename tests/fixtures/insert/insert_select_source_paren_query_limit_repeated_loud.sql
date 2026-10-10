-- An `INSERT … SELECT` whose source reads a parenthesized query repeating a clause is
-- refused, and inserts nothing.
-- @setup
CREATE TABLE ispq_s (x INT);
CREATE TABLE ispq_u (x INT);
INSERT INTO ispq_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
INSERT INTO ispq_u SELECT * FROM ((SELECT x FROM ispq_s LIMIT 1) LIMIT 2) d;
