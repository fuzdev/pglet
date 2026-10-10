-- A bare DEFAULT ahead of an indexed conjunct is refused where the scan reaches it, and the
-- write takes no probe that would skip it: over a row the probe would not return, this is
-- 42601 as on 18.6. (Written after the key, `a = -1 AND DEFAULT IS NULL`, the connective
-- stops at the key for every row and the statement reports `UPDATE 0` — EDGES.md row 56.)
-- @profile-lock
-- @expect-error: DEFAULT is not allowed in this context
-- @setup
CREATE TABLE dl (a INT, v TEXT);
CREATE INDEX dl_a ON dl (a);
INSERT INTO dl VALUES (1, 'x');
-- @test
UPDATE dl SET v = 'q' WHERE DEFAULT IS NULL AND a = -1;
