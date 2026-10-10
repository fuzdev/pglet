-- The column belongs to the statement's own level, two levels above the set operation, so
-- the middle level's lowering cannot type it and the statement's can.
-- @setup
CREATE TABLE dtl (i INT);
INSERT INTO dtl VALUES (7), (8);
CREATE TABLE dtm (k INT);
INSERT INTO dtm VALUES (1);
-- @test
DELETE FROM dtl WHERE EXISTS (SELECT 1 FROM dtm WHERE EXISTS (SELECT i EXCEPT SELECT '7')) RETURNING i;
