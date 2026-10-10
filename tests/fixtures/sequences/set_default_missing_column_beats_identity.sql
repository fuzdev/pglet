-- ...but the column lookup outranks the identity refusal, since
-- `ATExecColumnDefault` finds the attribute before it reads its identity flag. With
-- the table's own resolution (42P01) that is the whole prefix ahead of the 42601.
-- @setup
CREATE TABLE sdmc_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER TABLE sdmc_t ALTER COLUMN nosuch SET DEFAULT 1;
-- @expect-error: the missing column outranks the identity refusal (42703)
