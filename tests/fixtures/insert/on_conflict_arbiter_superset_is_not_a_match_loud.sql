-- The other direction of the same control: an inference list naming more columns than the
-- index holds is `42P10` on both engines. Equality is two-way containment, and this file
-- is the half a one-way `every index key appears in the target` test would lose.
-- @setup
CREATE TABLE oasu (a INT PRIMARY KEY, b INT);
INSERT INTO oasu VALUES (1, 1);

-- @expect-error: there is no unique or exclusion constraint matching the ON CONFLICT specification (42P10)
-- @profile-lock
-- @test
INSERT INTO oasu VALUES (1, 9) ON CONFLICT (a, b) DO NOTHING;
