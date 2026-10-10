-- A CHECK over each storable new function holds rows it accepts and reads back as
-- 18.6 prints it.
-- @setup
CREATE TABLE ck (
  a INT CHECK (mod(a, 2) = 0),
  t TEXT CHECK (left(t, 1) <> 'x' AND right(t, 1) <> 'y'),
  u TEXT CHECK (octet_length(u) < 10 AND md5(u) <> '' AND substr(u, 1, 1) <> 'q'));
INSERT INTO ck VALUES (2, 'abc', 'hello'), (4, 'zz', 'héllo');
-- @no-declared-types: 18.6 sends `name` (19) for `conname` and pglet sends `text` (25) —
--   pre-existing catalog-type modelling, not this fixture's subject.
-- @test
SELECT c.conname, pg_get_constraintdef(c.oid) AS def FROM pg_constraint c
WHERE c.conname LIKE 'ck\_%' ORDER BY c.conname;
