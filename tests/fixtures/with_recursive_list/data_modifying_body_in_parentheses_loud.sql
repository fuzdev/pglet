-- A data-modifying statement has no parenthesized form: its parentheses are a syntax
-- error, not a body to read inside.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: syntax error (42601)
-- @test
WITH RECURSIVE w AS ((INSERT INTO s VALUES (1) RETURNING x)) SELECT * FROM w;
