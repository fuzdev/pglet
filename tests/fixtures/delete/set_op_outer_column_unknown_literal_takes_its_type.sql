-- It writes: the enclosing row's `integer` column and the constant `'7'` are one
-- `integer` column, so `7 EXCEPT 7` is empty and only the row `8` is deleted.
-- @setup
CREATE TABLE dox (i INT);
INSERT INTO dox VALUES (7), (8);
-- @test
DELETE FROM dox WHERE EXISTS (SELECT i EXCEPT SELECT '7') RETURNING i;
