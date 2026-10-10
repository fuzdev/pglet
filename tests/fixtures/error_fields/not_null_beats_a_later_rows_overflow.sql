-- The same ordering against the other input-function failure a later row can raise: a
-- `varchar(n)` overflow. `22001` where the gate is hoisted, `23502` where it is per row.
-- @setup
CREATE TABLE y2 (a INT NOT NULL, s VARCHAR(3), k INT);
INSERT INTO y2 VALUES (1, 'ab', 1), (2, 'cd', 2);

-- @test
-- @expect-error: row 1's null, not row 2's varchar overflow
-- @expect-message
UPDATE y2 SET a = CASE WHEN k = 1 THEN NULL ELSE 8 END,
              s = CASE WHEN k = 2 THEN 'toolong' ELSE 'r' END;
