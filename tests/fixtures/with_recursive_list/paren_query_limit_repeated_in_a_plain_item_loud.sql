-- Under `WITH RECURSIVE`, an item that does not name itself and leads with a parenthesized
-- query repeating a clause is refused, so the `DELETE` reading it removes nothing.
-- @setup
CREATE TABLE rpq_s (x INT);
INSERT INTO rpq_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
WITH RECURSIVE c AS ((SELECT x FROM rpq_s ORDER BY x LIMIT 1) LIMIT 2) DELETE FROM rpq_s WHERE x IN (SELECT x FROM c);
