-- `42P16` changes sides by route. On `CREATE TABLE` the multiplicity test is
-- `transformIndexConstraints`', above the cook, so it beats a bad `DEFAULT`; on
-- `ADD COLUMN` it sits behind the cook and loses
-- (`add_column_default_beats_the_primary_key_multiplicity.sql`). The pair is why one
-- route's loser list is not the other's by inheritance.
-- @expect-error: the second primary key outranks the column default cook
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctspd (a INT PRIMARY KEY, b INT PRIMARY KEY DEFAULT 'zz');
