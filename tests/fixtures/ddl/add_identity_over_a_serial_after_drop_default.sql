-- A serial column made an identity column: its default dropped, the clause continues the
-- ids from a `START WITH` past the rows, and the serial's sequence is left to drop.
-- @profile-lock
-- @setup
CREATE TABLE aid (c SERIAL PRIMARY KEY, v TEXT);
INSERT INTO aid (v) VALUES ('a'), ('b');
ALTER TABLE aid ALTER COLUMN c DROP DEFAULT;
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY (START WITH 3);
DROP SEQUENCE aid_c_seq;
INSERT INTO aid (v) VALUES ('c');
-- @test
SELECT c, v FROM aid ORDER BY c;
