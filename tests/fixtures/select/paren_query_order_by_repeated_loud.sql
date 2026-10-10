-- A parenthesized query carrying its own `ORDER BY`, followed by another, is refused.
-- @setup
CREATE TABLE pqo_s (x INT);
INSERT INTO pqo_s VALUES (2), (1);

-- @test
-- @expect-error: multiple ORDER BY clauses not allowed
-- @expect-message
(SELECT x FROM pqo_s ORDER BY x) ORDER BY x;
