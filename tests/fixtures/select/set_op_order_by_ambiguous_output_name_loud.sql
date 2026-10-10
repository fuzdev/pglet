-- Why the claimant set a set-op key resolves against carries **no** expression identity.
-- Everywhere else a target list's entries are compared as `equal()` compares them, so two
-- spellings of one expression are one claimant; here 18.6 counts `a AS x, a AS x` as two
-- and answers `42702` even though the entries are identical. Carrying identities would
-- make this pair one claimant and accept a statement 18.6 refuses.
--
-- Over two empty arms, so the eval-time resolver never runs and the plan-time gate is the
-- only judge — seeded, this was already `42702` here.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);

-- @expect-error: column reference "x" is ambiguous (42702)
-- @test
SELECT a AS x, a AS x FROM sopa UNION SELECT a, a FROM sopb ORDER BY x;
