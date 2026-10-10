-- The inner set operation is `integer` through the enclosing column, so the outer one
-- unifies `integer` with the `integer` constant `8`.
-- @setup
CREATE TABLE sots (i INT);
INSERT INTO sots VALUES (7);
-- @test
SELECT count(*) FROM sots WHERE EXISTS ((SELECT i EXCEPT SELECT '7') UNION SELECT 8);
