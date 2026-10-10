-- A range with no lower bound over a plain `text` index starts at the least key a value
-- has: the empty string is in `c < 'b'`.
-- @profile-lock
-- @setup
CREATE TABLE lk (c TEXT, v TEXT);
CREATE INDEX lk_c ON lk (c);
INSERT INTO lk VALUES (NULL, 'null'), ('', 'empty'), ('a', 'a'), ('b', 'b');
-- @test
SELECT v FROM lk WHERE c < 'b' ORDER BY v;
