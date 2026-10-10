-- `LIMIT ALL` is a `LIMIT` the query carries, so a second one after the parentheses is
-- refused too.
-- @setup
CREATE TABLE pqa_s (x INT);
INSERT INTO pqa_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
(SELECT x FROM pqa_s LIMIT ALL) LIMIT 1;
