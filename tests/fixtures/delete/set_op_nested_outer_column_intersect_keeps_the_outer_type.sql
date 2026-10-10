-- A set operation over one that names an enclosing column unifies with that column's
-- type: `{1.25} INTERSECT {1}` is empty, so the row stays, and the row `1` is deleted.
-- @setup
CREATE TABLE soni (n NUMERIC);
INSERT INTO soni VALUES (1.25), (1), (3);
-- @test
DELETE FROM soni WHERE EXISTS ((SELECT n EXCEPT SELECT 3) INTERSECT SELECT 1) RETURNING n;
