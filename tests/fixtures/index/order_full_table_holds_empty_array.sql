-- An ordered index read over a `text[]` key starts at the empty array, the least key a
-- value has.
-- @profile-lock
-- @setup
CREATE TABLE lk (c TEXT[] NOT NULL UNIQUE, v TEXT);
INSERT INTO lk VALUES ('{a}', 'a'), ('{}', 'empty'), ('{""}', 'empty string');
-- @test
SELECT v FROM lk ORDER BY c;
