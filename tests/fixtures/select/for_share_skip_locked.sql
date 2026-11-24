-- `FOR SHARE SKIP LOCKED` and `FOR KEY SHARE SKIP LOCKED` with no contention answer every
-- row; this is the `FOR KEY SHARE` one, over a filter.
-- @profile-lock
-- @setup
CREATE TABLE lskip (id INT PRIMARY KEY, code TEXT);
INSERT INTO lskip VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
SELECT id FROM lskip WHERE id > 1 ORDER BY id FOR KEY SHARE SKIP LOCKED;
