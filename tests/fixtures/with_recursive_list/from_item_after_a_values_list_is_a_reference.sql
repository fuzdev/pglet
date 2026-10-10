-- An item after a `VALUES` list in a `FROM` list is read: `b` is resolved first.
-- @test
WITH RECURSIVE q AS (SELECT * FROM (VALUES (1), (2)) z (v), b), b AS (SELECT 5 AS x)
SELECT count(*) FROM q;
