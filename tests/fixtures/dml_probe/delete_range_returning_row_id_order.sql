-- A range-filtered DELETE over an indexed key removes its rows in the scan's order: over
-- rows inserted 3, 1, 2, `RETURNING` answers 3, 1, 2, not the index's key order.
-- @profile-lock
-- @setup
CREATE TABLE o2 (k INT, v TEXT);
CREATE INDEX o2_k ON o2 (k);
INSERT INTO o2 VALUES (3, 'c'), (1, 'a'), (2, 'b');
-- @test
DELETE FROM o2 WHERE k >= 1 RETURNING k, v;
