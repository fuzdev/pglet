-- `UPDATE … FROM … WHERE 't'` updates the row on both engines. Here the row was
-- left untouched and the statement reported success — a silently lost write on a
-- statement neither engine rejects.
-- @setup
CREATE TABLE bcu1 (a INT);
CREATE TABLE bcu2 (z INT);
INSERT INTO bcu1 VALUES (1);
INSERT INTO bcu2 VALUES (5);
-- @test
UPDATE bcu1 SET a = 3 FROM bcu2 WHERE 't' RETURNING a;
