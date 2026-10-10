-- The enclosing `integer` column types the inner set operation, and the outer one's `'07'`
-- converts to `integer` `7`, so the intersection holds the row.
-- @setup
CREATE TABLE sonu (i INT);
INSERT INTO sonu VALUES (7);
-- @test
SELECT count(*) FROM sonu WHERE EXISTS ((SELECT i EXCEPT SELECT '8') INTERSECT SELECT '07');
