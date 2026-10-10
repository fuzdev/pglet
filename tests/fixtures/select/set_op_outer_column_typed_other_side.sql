-- The enclosing `integer` column beside a typed `integer` side is one `integer` column.
-- @setup
CREATE TABLE sts (i INT);
INSERT INTO sts VALUES (7);
-- @test
SELECT count(*) FROM sts WHERE i IN (SELECT i UNION ALL SELECT 8);
