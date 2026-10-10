-- A DELETE whose range names no key over an indexed column deletes nothing: an inverted
-- `BETWEEN`, bound.
-- @profile-lock
-- @param $1 int4 3
-- @param $2 int4 1
-- @setup
CREATE TABLE ir (k INT PRIMARY KEY, v TEXT);
INSERT INTO ir VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
DELETE FROM ir WHERE k BETWEEN $1 AND $2 RETURNING k;
