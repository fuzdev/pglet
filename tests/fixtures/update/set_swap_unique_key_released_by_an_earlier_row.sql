-- A key swap 18.6 accepts: row `(2, 3)` is written first and moves off `2`, so row
-- `(1, 2)` then takes `2` with no conflict. The swapped values read the old row.
-- @setup
CREATE TABLE sswa (k INT UNIQUE, k2 INT);
INSERT INTO sswa VALUES (2, 3), (1, 2);

-- @test
UPDATE sswa SET k = k2, k2 = k RETURNING k, k2;
