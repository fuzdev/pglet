-- @setup
CREATE TABLE dob (k INT, v INT);
INSERT INTO dob VALUES (1, 10), (1, 20), (2, 30);

-- `transformDistinctOnClause` resolves every `DISTINCT ON` expression through
-- `findTargetlistEntrySQL92` and only then looks for the leftmost-ORDER-BY match, so a
-- key naming nothing is `42703` and never `42P10`. Ordering the two the other way is
-- what pglet did: this statement answered `42P10`, reporting a mismatch between an
-- ORDER BY key and a key that does not exist.
-- @expect-error: an unbound DISTINCT ON key outranks the ORDER BY mismatch
-- @test
SELECT DISTINCT ON (bogus) v FROM dob ORDER BY v;
