-- `c < ''` over a plain `text` index names no key: nothing sorts below the empty string.
-- @profile-lock
-- @setup
CREATE TABLE lk (c TEXT, v TEXT);
CREATE INDEX lk_c ON lk (c);
INSERT INTO lk VALUES (NULL, 'null'), ('', 'empty'), ('a', 'a'), ('b', 'b');
-- @test
SELECT v FROM lk WHERE c < '';
