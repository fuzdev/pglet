-- Under `WITH RECURSIVE`, a repeated `ORDER BY` over a parenthesized query in an item that
-- does not name itself is refused, and the `INSERT` reading it writes nothing.
-- @setup
CREATE TABLE rpo_s (x INT);
CREATE TABLE rpo_u (x INT);
INSERT INTO rpo_s VALUES (2), (1);

-- @test
-- @expect-error: multiple ORDER BY clauses not allowed
-- @expect-message
WITH RECURSIVE c AS ((SELECT x FROM rpo_s ORDER BY x) ORDER BY x) INSERT INTO rpo_u SELECT x FROM c;
