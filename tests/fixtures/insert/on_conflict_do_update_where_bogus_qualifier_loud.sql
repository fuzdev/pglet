-- The `DO UPDATE … WHERE` spelling of the same bogus qualifier, and it decides *whether*
-- the row is written rather than what is written into it: `zzz.v` bound to the excluded
-- row, so this predicate compared `'new'` against `'new'` and the update fired, where
-- 18.6 answers `42P01` and leaves `orig` in place.
--
-- The polarity is load-bearing and is why the fixture compares against `'new'` rather
-- than `'orig'`: written the other way the mis-binding makes the predicate *false* and
-- the statement looks harmless — same wrong resolution, no visible write. A guard on the
-- quiet polarity would have passed on the day this shipped.
-- @setup
CREATE TABLE ocw_t (id INT PRIMARY KEY, v TEXT);
INSERT INTO ocw_t VALUES (1, 'orig');

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @no-profile-lock: the arbiter's unique-index probe is constraint enforcement rather than acceleration and runs identically on both profiles; the refusal is raised evaluating the DO UPDATE action, which selects no index
-- @test
INSERT INTO ocw_t VALUES (1, 'new') ON CONFLICT (id) DO UPDATE SET v = 'x' WHERE zzz.v = 'new';
