-- A range-filtered UPDATE over an indexed key writes its rows in the scan's order, not the
-- index's: over rows inserted 3, 1, 2, `RETURNING` answers 3, 1, 2 where the index's key
-- order is 1, 2, 3. A write takes no range probe (docs/indexes.md § A write's target
-- rows); this holds the order for both builds, and for a range probe should one return.
-- @profile-lock
-- @setup
CREATE TABLE o1 (k INT, v TEXT);
CREATE INDEX o1_k ON o1 (k);
INSERT INTO o1 VALUES (3, 'c'), (1, 'a'), (2, 'b');
-- @test
UPDATE o1 SET v = v || '!' WHERE k > 0 RETURNING k, v;
