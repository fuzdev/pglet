-- A set operation's parenthesized arm that repeats a clause of its own inner query is
-- refused.
-- @setup
CREATE TABLE pqs_s (x INT);
INSERT INTO pqs_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
SELECT x FROM pqs_s UNION ((SELECT x FROM pqs_s LIMIT 1) LIMIT 2);
