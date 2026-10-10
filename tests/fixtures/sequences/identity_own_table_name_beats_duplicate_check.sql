-- ...and it lands at the *table's* `42P07` position, ahead of the constraint names:
-- the heap is created before `AddRelationNewConstraints` cooks and names the CHECKs,
-- so this is 42P07 and not the duplicate CHECK name's 42710. Discriminating on the
-- SQLSTATE, which is what makes it a guard on the *placement* and not merely on the
-- check existing.
-- @test
CREATE TABLE idownck (x INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idownck), CONSTRAINT idownck_dup CHECK (x > 0), CONSTRAINT idownck_dup CHECK (x > 1));
-- @expect-error: the table's own name is taken before its constraints are named (42P07)
