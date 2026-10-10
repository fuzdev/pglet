-- The same over a plain `UPDATE … WHERE`: every row is matched, so both rows move.
-- @setup
CREATE TABLE bcu3 (a INT);
INSERT INTO bcu3 VALUES (1), (2);
-- @test
UPDATE bcu3 SET a = a + 10 WHERE 'yes' RETURNING a;
