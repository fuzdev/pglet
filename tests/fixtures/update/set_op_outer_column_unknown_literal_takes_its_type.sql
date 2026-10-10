-- It writes: only the row whose `integer` column differs from the constant `'7'` is
-- updated.
-- @setup
CREATE TABLE uox (i INT);
INSERT INTO uox VALUES (7), (8);
-- @test
UPDATE uox SET i = i + 100 WHERE EXISTS (SELECT i EXCEPT SELECT '7') RETURNING i;
