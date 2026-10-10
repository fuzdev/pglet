-- `JOIN … USING (k)` matches on `left.k = right.k`, so a `text` `k` against an
-- `int` `k` resolves to no operator (`42883`). The USING matcher called
-- `values_equal` directly, bypassing the operator gate, and silently produced an
-- empty join.
-- @setup
CREATE TABLE a (k TEXT, x INT);
CREATE TABLE b (k INT, y INT);
INSERT INTO a VALUES ('1', 10);
INSERT INTO b VALUES (1, 20);
-- @test
-- @expect-error: no text = integer operator
SELECT * FROM a JOIN b USING (k);
