-- A pending set-op subquery written twice in one statement: the sort key equals the
-- target entry, so the key is matched to it and its own node is discarded — and the
-- statement's second parse has to build the two the same way for that match to hold.
-- @setup
CREATE TABLE sodk (i INT);
INSERT INTO sodk VALUES (7), (9);
-- @test
SELECT DISTINCT (SELECT i EXCEPT SELECT '7') AS z FROM sodk ORDER BY (SELECT i EXCEPT SELECT '7');
