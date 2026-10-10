-- An ordered index read with no bound starts at the least key a value has: the empty
-- string is the first row.
-- @profile-lock
-- @setup
CREATE TABLE lk (c TEXT NOT NULL, v TEXT);
CREATE INDEX lk_c ON lk (c);
INSERT INTO lk VALUES ('b', 'b'), ('', 'empty'), ('a', 'a');
-- @test
SELECT v FROM lk ORDER BY c LIMIT 1;
