-- The written-order fold is observable as **error precedence**, not only as which
-- specification a name lands on. A table-level CHECK declaring `ctdc_a_check` is written
-- ahead of a column-level CHECK whose auto-name base is that same string: resolved in
-- written order the declared name is claimed first and the auto one steps aside to
-- `ctdc_a_check1`, and the statement is valid. Folding the column-level constraints ahead
-- of the table-level ones inverts it — the auto name takes `ctdc_a_check` unopposed and
-- the *declared* one then collides, 42710, on DDL Postgres accepts.
--
-- The asymmetry is what makes the order visible at all: an auto name is disambiguated
-- (`ChooseConstraintName`) and a declared name is the user's to get wrong, so only one of
-- the two orders can survive. Written the other way round
-- (`a INT CHECK (a > 0), CONSTRAINT ctdc_a_check CHECK (…)`) both folds agree on 42710,
-- which is why the arrangement here is the one that pins the direction.
-- @setup
CREATE TABLE ctdc (CONSTRAINT ctdc_a_check CHECK (a > 100), a INT CHECK (a > 0));

-- @test
SELECT constraint_name::text AS constraint_name
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'ctdc' AND constraint_type = 'CHECK'
ORDER BY constraint_name;
