-- A parenthesized query carrying its own `OFFSET`, followed by another, is refused.
-- @setup
CREATE TABLE pqf_s (x INT);
INSERT INTO pqf_s VALUES (2), (1);

-- @test
-- @expect-error: multiple OFFSET clauses not allowed
-- @expect-message
(SELECT x FROM pqf_s OFFSET 1) OFFSET 0;
