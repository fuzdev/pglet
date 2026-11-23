-- A session-level call over rows takes or gives back one hold per row: two rows of key 1
-- take two holds, so the row-by-row unlocks answer `true` for each of the three rows.
-- @setup
CREATE TABLE asl (a INT);
INSERT INTO asl VALUES (1), (2), (1);
-- @test
SELECT pg_advisory_lock(a) FROM asl;
SELECT a, pg_advisory_unlock(a) AS released FROM asl;
